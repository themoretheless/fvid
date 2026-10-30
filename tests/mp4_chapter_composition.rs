use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-chapter-composition")
            .join(n),
    )
    .unwrap()
}
fn names() -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}
#[test]
fn chapter_clock_accounts_for_the_composition_endpoint_in_both_parsers() {
    let mut failures = Vec::new();
    for n in names() {
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
            .unwrap();
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
            .unwrap();
        let aa: Vec<_> = a
            .chapters()
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        let bb: Vec<_> = b
            .chapters()
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        let expected = vec![
            (200000000, "Opening"),
            (220000000, "Middle"),
            (240000000, "End"),
        ];
        if aa != expected || bb != expected {
            failures.push(format!("{n}: root={aa:?}, owned={bb:?}"));
        }
        assert_eq!(
            (
                a.movie_timescale(),
                a.tracks()[0].timescale,
                a.tracks()[0].duration
            ),
            (1000, 50, 3)
        );
        for i in 0..3 {
            let s = a.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (i as u64, 10 + i as i64, 1));
        }
    }
    assert!(
        failures.is_empty(),
        "composition clock confused with decode duration: {failures:?}"
    );
}
#[cfg(feature = "player")]
#[test]
fn chapter_media_keeps_analytic_pixels_and_exact_playback_navigation() {
    use std::time::Duration;
    for n in names() {
        let mut raw = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(&n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = raw.read_frame().unwrap().unwrap();
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(raw.read_frame().unwrap().is_none());
        let mut r = fvid::playback_native::NativeReader::software(Cursor::new(bytes(&n)), 32 << 20)
            .unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(260)));
        let mut colors = Vec::new();
        for i in 0..3 {
            colors.push(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
            );
            assert_eq!(r.frame_interval(), Some((10 + i, 11 + i, 50)));
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        for ms in [0, 199, 200, 219, 220, 239, 240, 259, 260, 400] {
            let i = if ms < 220 {
                0
            } else if ms < 240 {
                1
            } else {
                2
            };
            assert_eq!(
                r.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                colors[i]
            );
            assert_eq!(
                r.frame_interval(),
                Some((10 + i as u128, 11 + i as u128, 50))
            );
        }
        r.rewind().unwrap();
        assert_eq!(
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            colors[0]
        );
    }
}
