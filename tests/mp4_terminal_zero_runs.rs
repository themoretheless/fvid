#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-terminal-zero-runs")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn video(n: &str, visible: usize) {
    let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(n)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..visible {
        let f = r.read_frame().unwrap().unwrap();
        assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i as i64, 1));
        let p: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(p, bytes(&format!("frame-{i}.yuv16")), "{n}");
    }
    assert!(r.read_frame().unwrap().is_none());
    assert!(r.read_frame().unwrap().is_none());
    let mut native =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
    assert_eq!(
        native.duration(),
        Some(Duration::from_millis(visible as u64 * 20))
    );
    let rgb: Vec<_> = (0..visible)
        .map(|_| {
            native
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect();
    assert!(native.read_frame_raw().unwrap().is_none());
    native.rewind().unwrap();
    for ns in [
        0, 19_999_999, 20_000_000, 39_999_999, 40_000_000, 59_999_999, 60_000_000,
    ] {
        let i = (ns as usize / 20_000_000).min(visible - 1);
        let f = native.seek_raw(Duration::from_nanos(ns)).unwrap().unwrap();
        assert_eq!(f.into_rgb(32 << 20).unwrap(), rgb[i]);
        assert_eq!(
            native.frame_interval(),
            Some((i as u128, i as u128 + 1, 50)),
            "{n}"
        );
    }
}
#[test]
fn positive_and_single_terminal_zero_video_preserve_packets_pixels_seek_and_eof() {
    for c in manifest()["valid_video"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 3;
        let mut a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), limits).unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut time = 0u64;
        for i in 0..3 {
            let delta = c["durations"][i].as_u64().unwrap() as u32;
            for track in [&a.tracks()[0]] {
                let s = track.samples.get(i).unwrap();
                assert_eq!((s.dts, s.pts, s.duration), (time, time as i64, delta));
            }
            let s = b.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (time, time as i64, delta));
            time += u64::from(delta);
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(0, i, &mut x).unwrap();
            b.read_packet(0, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("frame-{i}.packet")));
            assert_eq!(y, x);
        }
        assert_eq!(a.tracks()[0].duration, time);
        assert_eq!(b.tracks()[0].duration, time);
        limits.samples = 2;
        assert!(fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), limits).is_err());
        a.restrict_packet_bytes(1);
        assert!(
            a.read_packet(0, 0, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("packet exceeds budget")
        );
        video(n, c["visible"].as_u64().unwrap() as usize);
    }
}
#[test]
fn terminal_zero_audio_uses_real_pcm_frames_or_mace_blocks_without_rejecting_one_packet() {
    for c in manifest()["valid_audio"].as_array().unwrap() {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let count = c["packet_count"].as_u64().unwrap() as usize;
        let unit = c["unit"].as_u64().unwrap();
        assert_eq!(a.tracks().len(), 2);
        assert_eq!(b.tracks().len(), 2);
        assert_eq!(a.tracks()[1].samples.len(), count);
        assert_eq!(b.tracks()[1].samples.len(), count);
        for i in 0..count {
            let delta = if i + 1 == count { 0 } else { unit as u32 };
            let s = a.tracks()[1].samples.get(i).unwrap();
            assert_eq!(
                (s.dts, s.pts, s.duration),
                (i as u64 * unit, i as i64 * unit as i64, delta),
                "{n}"
            );
            let t = b.tracks()[1].samples.get(i).unwrap();
            assert_eq!((t.dts, t.pts, t.duration), (s.dts, s.pts, s.duration));
        }
        assert_eq!(
            a.tracks()[1].duration,
            c["audio_duration_ticks"].as_u64().unwrap()
        );
        assert_eq!(b.tracks()[1].duration, a.tracks()[1].duration);
        let mut audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let mut decoder = audio.make_decoder().unwrap();
        let mut decoded = Vec::new();
        for i in 0..count {
            let p = audio.read_packet().unwrap().unwrap();
            let expected = if c["codec"] == "mace" {
                bytes("mace.packet")
            } else {
                bytes("pcm.s16le")[i * 2..i * 2 + 2].to_vec()
            };
            assert_eq!(p.data, expected);
            decoded.extend(
                decoder
                    .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                    .unwrap()
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(decoded.len(), c["raw_count"].as_u64().unwrap() as usize * 4);
        assert!(audio.read_packet().unwrap().is_none());
        // This accepts the existing zero-window metadata/decoded-block convention;
        // video playback is verified independently, without claiming audible output.
        video(n, 3);
    }
}
fn refused(category: &str) {
    let mut accepted = Vec::new();
    for c in manifest()["refused"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["category"] == category)
    {
        let n = c["file"].as_str().unwrap();
        let a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).err();
        let b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).err();
        match (a, b) {
            (Some(a), Some(b)) => {
                assert!(matches!(a, fvid::Error::Invalid(_)));
                assert!(!b.is_unsupported());
                assert_eq!(a.to_string(), "invalid decode-time run", "{n}");
                assert_eq!(b.to_string(), a.to_string());
            }
            other => accepted.push((n.to_owned(), other.0.is_none(), other.1.is_none())),
        }
    }
    assert!(
        accepted.is_empty(),
        "root/owned accepted nonterminal zero packets: {accepted:?}"
    );
}
#[test]
fn previously_rejected_nonterminal_zero_rows_still_refuse() {
    refused("existing");
}
#[test]
fn grouped_zero_suffixes_must_not_bypass_the_single_last_packet_rule() {
    refused("new");
}
