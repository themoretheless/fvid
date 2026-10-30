use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unknown-movie-chapters")
            .join(n),
    )
    .unwrap()
}
fn cases(clock: &str) -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[clock]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}
fn starts(n: &str) -> Vec<u64> {
    let r =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    r.chapters().iter().map(|c| c.start_ns).collect()
}
#[test]
fn ten_mhz_chapters_keep_their_clock_when_movie_duration_is_unavailable() {
    for n in cases("ten_mhz") {
        assert_eq!(starts(&n), [0, 20_000_000, 40_000_000], "root: {n}");
        let r = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
            .unwrap();
        assert_eq!(
            r.chapters().iter().map(|c| c.start_ns).collect::<Vec<_>>(),
            [0, 20_000_000, 40_000_000],
            "owned: {n}"
        );
        assert_eq!(
            r.chapters()
                .iter()
                .map(|c| c.title.as_str())
                .collect::<Vec<_>>(),
            ["Opening", "Middle", "End"]
        );
    }
}
#[test]
fn legacy_movie_clock_chapters_and_packet_clocks_remain_exact() {
    for n in cases("movie_clock") {
        assert_eq!(starts(&n), [0, 20_000_000, 40_000_000], "{n}");
        let mut r =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
                .unwrap();
        assert_eq!(
            (
                r.movie_timescale(),
                r.tracks()[0].timescale,
                r.tracks()[0].duration
            ),
            (1000, 50, 3)
        );
        for i in [2, 0, 1] {
            let sample = r.tracks()[0].samples.get(i).unwrap();
            assert_eq!(
                (sample.dts, sample.pts, sample.duration),
                (i as u64, i as i64, 1)
            );
            let mut data = Vec::new();
            r.read_packet(0, i, &mut data).unwrap();
            assert_eq!(data, bytes("frame.vp9"));
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
fn chapters_seek_to_the_expected_native_frames() {
    use std::time::Duration;
    for n in cases("ten_mhz").into_iter().chain(cases("movie_clock")) {
        let mut r = fvid::playback_native::NativeReader::software(Cursor::new(bytes(&n)), 32 << 20)
            .unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(60)), "{n}");
        for i in 0..3 {
            assert_eq!(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)), "{n}");
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(r.read_frame_raw().unwrap().is_none());
        r.rewind().unwrap();
        assert!(r.read_frame_raw().unwrap().is_some());
        for (i, ns) in starts(&n).into_iter().enumerate() {
            let frame = r.seek_raw(Duration::from_nanos(ns)).unwrap().unwrap();
            assert_eq!(frame.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
            assert_eq!(
                r.frame_interval(),
                Some((i as u128, i as u128 + 1, 50)),
                "chapter {i} of {n}"
            );
        }
    }
}

#[test]
fn legacy_chapter_clock_uses_movie_edit_duration_including_empty_edits() {
    for n in cases("edited_movie_clock") {
        assert_eq!(
            starts(&n),
            [5_000_000_000, 5_020_000_000, 5_040_000_000],
            "{n}"
        );
        let r = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
            .unwrap();
        assert_eq!(
            r.tracks()[0].edits.iter().map(|e| e.duration).sum::<u64>(),
            5060
        );
        assert_eq!(
            r.chapters().iter().map(|c| c.start_ns).collect::<Vec<_>>(),
            starts(&n)
        );
    }
}
