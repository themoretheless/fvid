use fvid::container::mp4::{Limits, Mp4Reader};
use std::{io::Cursor, path::PathBuf};

fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-multi-trun")
            .join(n),
    )
    .unwrap()
}

fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}

fn indexed(n: &str) -> Vec<(u64, i64, u32, bool)> {
    let mut p = Mp4Reader::open(
        Cursor::new(bytes(n)),
        Limits {
            samples: 3,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(p.tracks().len(), 1);
    assert_eq!(p.tracks()[0].codec, *b"vp09");
    assert_eq!((p.tracks()[0].timescale, p.tracks()[0].duration), (50, 3));
    assert_eq!(p.tracks()[0].samples.len(), 3);
    let mut out = Vec::new();
    let packet = bytes("frame.vp9");
    for i in 0..3 {
        let s = p.tracks()[0].samples.get(i).unwrap();
        assert_eq!(
            (s.dts, s.pts, s.duration, s.sync),
            (i as u64, i as i64, 1, true),
            "{n} sample {i}"
        );
        out.push((s.dts, s.pts, s.duration, s.sync));
        let mut got = Vec::new();
        p.read_packet(0, i, &mut got).unwrap();
        assert_eq!(got, packet);
    }
    assert!(p.read_packet(0, 3, &mut Vec::new()).is_err());
    out
}

#[test]
fn valid_multi_trun_continuity_preserves_packets_clocks_and_budgets() {
    let regular = indexed("regular.mp4");
    let packet = bytes("frame.vp9");
    for c in cases("controls") {
        let n = c["file"].as_str().unwrap();
        assert_eq!(indexed(n), regular, "{n}");
        let error = match Mp4Reader::open(
            Cursor::new(bytes(n)),
            Limits {
                samples: 2,
                ..Default::default()
            },
        ) {
            Ok(_) => panic!("{n}: sample budget bypassed"),
            Err(e) => e,
        };
        assert!(
            error.to_string().contains("sample count exceeds budget"),
            "{n}: {error}"
        );
        let mut p = Mp4Reader::open(
            Cursor::new(bytes(n)),
            Limits {
                packet_bytes: packet.len(),
                ..Default::default()
            },
        )
        .unwrap();
        p.read_packet(0, 0, &mut Vec::new()).unwrap();
        let mut p = Mp4Reader::open(
            Cursor::new(bytes(n)),
            Limits {
                packet_bytes: packet.len() - 1,
                ..Default::default()
            },
        )
        .unwrap();
        let e = p.read_packet(0, 0, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("packet exceeds budget"));
    }
}

#[test]
fn cross_traf_and_empty_reset_outside_mdat_refuse_as_invalid() {
    for c in cases("cases") {
        let n = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let e = match Mp4Reader::open(Cursor::new(bytes(n)), Default::default()) {
            Ok(_) => panic!("{n}: cross-traf or empty-reset outside mdat accepted"),
            Err(e) => e,
        };
        assert!(matches!(e, fvid::Error::Invalid(_)), "{n}: {e:?}");
        assert!(e.to_string().contains(expected), "{n}: {e}");
    }
}

#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::playback_native::{NativeReader, RawFrame};
    use std::time::Duration;

    fn open(n: &str) -> NativeReader<Cursor<Vec<u8>>> {
        NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap()
    }

    fn frame(raw: RawFrame) {
        match &raw {
            RawFrame::Planar(p) => {
                assert_eq!(p.depth, 8);
                assert_eq!((p.frame.width, p.frame.height), (16, 16));
                assert_eq!(p.frame.data, vec![128; 384]);
            }
            _ => panic!("VP9 control did not retain native packed planes"),
        };
        assert_eq!(raw.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
    }

    fn play(n: &str) {
        let mut s = open(n);
        assert_eq!(s.duration(), Some(Duration::from_millis(60)));
        for i in 0..3 {
            frame(s.read_frame_raw().unwrap().unwrap());
            assert_eq!(s.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(s.read_frame_raw().unwrap().is_none());
        assert!(s.read_frame_raw().unwrap().is_none());
        s.rewind().unwrap();
        frame(s.read_frame_raw().unwrap().unwrap());
        assert_eq!(s.frame_interval(), Some((0, 1, 50)));
        for ms in [0, 19, 20, 39, 40, 59, 60, 80] {
            let got = s.seek_raw(Duration::from_millis(ms)).unwrap();
            frame(got.expect("seek must present the containing or final frame"));
            let i = u128::from(ms.min(59) / 20);
            assert_eq!(s.frame_interval(), Some((i, i + 1, 50)));
            assert_eq!(s.current_pts(), Some((i as i64, 50)));
            if ms >= 60 {
                assert!(s.read_frame_raw().unwrap().is_none());
                assert!(s.read_frame_raw().unwrap().is_none());
            }
        }
    }

    #[test]
    fn valid_multi_trun_continuity_keeps_analytic_planes_rgb_public_seek_and_eof() {
        play("regular.mp4");
        for c in cases("controls") {
            let n = c["file"].as_str().unwrap();
            play(n);
        }
    }

    #[test]
    fn cross_traf_and_empty_reset_outside_mdat_refuse_before_native_decoder_admission() {
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let expected = c["expected_error"].as_str().unwrap();
            let e = match NativeReader::software(Cursor::new(bytes(n)), 32 << 20) {
                Ok(_) => panic!("{n}: cross-traf or empty-reset outside mdat reached native playback"),
                Err(e) => e,
            };
            assert!(matches!(e, fvid::Error::Invalid(_)), "{n}: {e:?}");
            assert!(e.to_string().contains(expected), "{n}: {e}");
        }
    }
}
