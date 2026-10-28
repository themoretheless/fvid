use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-time-header-flags")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["invalid_flags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}
#[test]
fn reserved_time_header_flags_are_invalid_in_both_parsers() {
    let mut failures = vec![];
    for n in cases() {
        let root =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
        let owned =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
        match (root, owned) {
            (Err(a), Err(b)) => {
                assert!(
                    a.to_string().contains("reserved MP4 time-header flags"),
                    "{n}: {a}"
                );
                assert!(
                    b.to_string().contains("reserved MP4 time-header flags"),
                    "{n}: {b}"
                );
                assert!(!b.is_unsupported());
            }
            _ => failures.push(n),
        }
    }
    assert!(failures.is_empty(), "accepted reserved flags: {failures:?}");
}
#[test]
fn zero_time_header_flags_preserve_packets_clocks_and_budgets() {
    for n in ["valid-v0.mp4", "valid-v1.mp4"] {
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(
            (
                r.movie_timescale(),
                r.tracks()[0].timescale,
                r.tracks()[0].duration
            ),
            (50, 50, 3)
        );
        for i in [2, 0, 1] {
            let s = r.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (i as u64, i as i64, 1));
            let mut got = Vec::new();
            r.read_packet(0, i, &mut got).unwrap();
            assert_eq!(got, bytes("frame.vp9"));
        }
        r.restrict_packet_bytes(1);
        assert!(
            r.read_packet(0, 0, &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains("packet exceeds budget")
        );
    }
}

#[cfg(feature = "player")]
#[test]
fn native_reader_refuses_invalid_time_headers_before_decoding() {
    for n in cases() {
        let error =
            match fvid::playback_native::NativeReader::software(Cursor::new(bytes(&n)), 32 << 20) {
                Ok(_) => panic!("{n} admitted invalid time flags"),
                Err(e) => e,
            };
        assert!(
            error.to_string().contains("reserved MP4 time-header flags"),
            "{n}: {error}"
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn valid_v0_v1_time_headers_preserve_pixels_seek_rewind_and_eof() {
    use std::time::Duration;
    for n in ["valid-v0.mp4", "valid-v1.mp4"] {
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(60)));
        for i in 0..3 {
            assert_eq!(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        for ms in [0, 19, 20, 39, 40, 59, 60, 100] {
            let got = r.seek_raw(Duration::from_millis(ms)).unwrap().unwrap();
            assert_eq!(got.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            let i = u128::from(ms.min(59) / 20);
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        }
    }
}
