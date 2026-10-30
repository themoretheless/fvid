use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-open-ended-edit")
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
#[test]
fn finite_edit_controls_preserve_both_parsers_and_packets() {
    for c in cases("controls") {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        assert_eq!(
            a.movie_timescale(),
            c["movie_scale"].as_u64().unwrap() as u32
        );
        assert_eq!(
            a.tracks()[0].edits[0].duration,
            c["duration"].as_u64().unwrap()
        );
        assert_eq!(
            b.tracks()[0].edits[0].duration,
            a.tracks()[0].edits[0].duration
        );
        for i in (0..3).rev() {
            let s = a.tracks()[0].samples.get(i).unwrap();
            assert_eq!(
                (s.dts, s.pts, s.duration),
                (i as u64, i as i64 + c["shift"].as_i64().unwrap(), 1)
            );
            let mut got = Vec::new();
            a.read_packet(0, i, &mut got).unwrap();
            assert_eq!(got, bytes("frame.vp9"));
        }
    }
}
#[test]
fn single_fragmented_open_edits_preserve_the_unbounded_marker() {
    let mut failures = Vec::new();
    for c in cases("open") {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        match (a, b) {
            (Ok(a), Ok(b)) => {
                assert_eq!(
                    a.tracks()[0].edits[0].duration,
                    0,
                    "{n}: retain the declared open marker, not a rounded movie duration"
                );
                assert_eq!(b.tracks()[0].edits[0].duration, 0);
                assert_eq!(
                    a.tracks()[0].edits[0].media_time,
                    c["start"].as_i64().unwrap()
                );
            }
            (a, b) => failures.push(format!(
                "{n}: root={}, owned={}",
                a.err().map(|e| e.to_string()).unwrap_or_default(),
                b.err().map(|e| e.to_string()).unwrap_or_default()
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "open media edits refused: {failures:?}"
    );
}
#[test]
fn malformed_zero_edits_still_refuse() {
    for c in cases("invalid") {
        let n = c.as_str().unwrap();
        let a = match fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        ) {
            Ok(_) => panic!("{n} accepted"),
            Err(e) => e,
        };
        assert!(
            a.to_string()
                .contains("unsupported MP4 edit rate, duration or media time"),
            "{n}: {a}"
        );
    }
}
#[cfg(feature = "player")]
fn check_native_edits(which: &str) {
    use std::time::Duration;
    for c in cases(which) {
        let n = c["file"].as_str().unwrap();
        let frames = c["frames"].as_u64().unwrap();
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let end = frames * 20;
        assert_eq!(r.duration(), Some(Duration::from_millis(end)), "{n}");
        for i in 0..frames {
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
                Some((i as u128, i as u128 + 1, 50)),
                "{n}"
            );
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        for ms in [0, 19, end - 1, end, end + 40] {
            assert_eq!(
                r.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            let i = u128::from(ms.min(end - 1) / 20);
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)), "{n}");
        }
    }
}
#[test]
fn open_edit_probe_reports_the_exact_remaining_media_duration() {
    for c in cases("open") {
        let n = c["file"].as_str().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-open-ended-edit")
            .join(n);
        let p = fvid_media::owned_mp4_probe::probe_mp4(&path).unwrap();
        assert_eq!(
            p.duration_us,
            Some(c["frames"].as_u64().unwrap() as i64 * 20_000),
            "{n}"
        );
    }
}

#[cfg(feature = "player")]
#[test]
fn finite_edit_controls_keep_native_playback_seek_and_eof() {
    check_native_edits("controls");
}
#[cfg(feature = "player")]
#[test]
fn open_edit_duration_frames_seek_and_eof_use_exact_media_ticks() {
    check_native_edits("open");
}

#[test]
fn owned_video_decode_honors_the_complete_open_edit_window() {
    for c in cases("open") {
        let n = c["file"].as_str().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-open-ended-edit")
            .join(n);
        let stats = fvid_media::decode_video(&path).unwrap();
        assert_eq!(
            (
                stats.width,
                stats.height,
                stats.video_frames,
                stats.decode_errors
            ),
            (16, 16, c["frames"].as_u64().unwrap(), 0),
            "{n}"
        );
    }
}
