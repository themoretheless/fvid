use std::io::Cursor;
const CASES: [&str; 6] = [
    "regular-known.mp4",
    "fragmented-known.mp4",
    "regular-unknown-v0.mp4",
    "regular-unknown-v1.mp4",
    "fragmented-unknown-v0.mp4",
    "fragmented-unknown-v1.mp4",
];
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unknown-duration")
            .join(n),
    )
    .unwrap()
}
#[test]
fn unknown_media_duration_is_recovered_from_indexed_samples() {
    for n in CASES {
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(r.tracks()[0].duration, 3, "{n}");
        for i in [2, 0, 1] {
            let mut got = Vec::new();
            r.read_packet(0, i, &mut got).unwrap();
            assert_eq!(got, bytes("frame.vp9"));
        }
    }
}
#[test]
fn owned_unknown_media_duration_is_recovered_from_indexed_samples() {
    for n in CASES {
        let r = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        assert_eq!(r.tracks()[0].duration, 3, "{n}");
    }
}
#[cfg(feature = "player")]
#[test]
fn unknown_media_duration_preserves_native_playback_and_seek() {
    use std::time::Duration;
    for n in CASES {
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(60)), "{n}");
        for i in 0..3 {
            let raw = r.read_frame_raw().unwrap().unwrap();
            assert_eq!(raw.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        for ms in [0, 19, 20, 39, 40, 59, 60, 100] {
            assert!(r.seek_raw(Duration::from_millis(ms)).unwrap().is_some());
            let i = u128::from(ms.min(59) / 20);
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        }
    }
}

#[test]
fn unknown_duration_probe_reports_the_indexed_sixty_milliseconds() {
    for n in CASES {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unknown-duration")
            .join(n);
        let info = fvid_media::owned_mp4_probe::probe_mp4(&path).unwrap();
        assert_eq!(info.duration_us, Some(60_000), "{n}");
        assert_eq!(info.streams.len(), 1);
        assert_eq!(info.streams[0].duration, Some(3), "{n}");
    }
}
