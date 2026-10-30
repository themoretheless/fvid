use std::{io::Cursor, path::Path};
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/mp4-nondisplay-endpoint/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn bytes(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-nondisplay-endpoint")
            .join(c["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn hidden_and_shown_controls_keep_exact_display_intervals() {
    for c in cases() {
        let data = bytes(&c);
        let root =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let owned =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let expected_header = c["header_duration"].as_u64();
        assert_eq!(root.track_header_duration(0), expected_header);
        assert_eq!(owned.track_header_duration(0), expected_header);
        assert_eq!(root.tracks()[0].duration, 3);
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(&c)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for span in c["intervals"].as_array().unwrap() {
            let f = r.read_frame().unwrap().unwrap();
            assert_eq!(f.presentation_time.ticks, span[0].as_i64().unwrap());
            assert_eq!(
                f.duration.ticks,
                span[1].as_i64().unwrap() - span[0].as_i64().unwrap()
            );
            assert!(f.picture.y.iter().all(|&y| y == 128));
        }
        assert!(r.read_frame().unwrap().is_none());
    }
}
#[cfg(feature = "player")]
#[test]
fn late_hidden_pts_does_not_stretch_native_duration_or_seek() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let mut r = fvid::playback_native::NativeReader::software(Cursor::new(bytes(&c)), 32 << 20)
            .unwrap();
        let expected = Some(Duration::from_millis(c["duration"].as_u64().unwrap() * 20));
        if r.duration() != expected {
            failures.push(format!("{}: {:?}", c["file"], r.duration()));
        }
        for span in c["intervals"].as_array().unwrap() {
            assert_eq!(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            assert_eq!(
                r.frame_interval(),
                Some((
                    span[0].as_u64().unwrap() as u128,
                    span[1].as_u64().unwrap() as u128,
                    50
                ))
            );
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        let span = c["intervals"].as_array().unwrap().last().unwrap();
        let target = span[1].as_u64().unwrap() * 20 - 1;
        assert!(r.seek_raw(Duration::from_millis(target)).unwrap().is_some());
        assert_eq!(
            r.frame_interval(),
            Some((
                span[0].as_u64().unwrap() as u128,
                span[1].as_u64().unwrap() as u128,
                50
            ))
        );
        assert!(r.read_frame_raw().unwrap().is_none());
    }
    assert!(
        failures.is_empty(),
        "non-display samples changed native duration: {failures:?}"
    );
}
