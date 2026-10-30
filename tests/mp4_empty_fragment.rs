use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-empty-fragment")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn empty_intervals_keep_packet_timestamps_and_total_duration_in_both_parsers() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default());
        let (mut a, mut b) = match (a, b) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                failures.push(format!(
                    "{n}: root={}, owned={}",
                    a.err().map_or(String::new(), |e| e.to_string()),
                    b.err().map_or(String::new(), |e| e.to_string())
                ));
                continue;
            }
        };
        assert_eq!(
            a.tracks()[0].duration,
            c["duration"].as_u64().unwrap(),
            "{n}"
        );
        assert_eq!(b.tracks()[0].duration, a.tracks()[0].duration);
        assert_eq!(a.tracks()[0].samples.len(), 3);
        assert_eq!(b.tracks()[0].samples.len(), 3);
        for i in [2, 0, 1] {
            let s = a.tracks()[0].samples.get(i).unwrap();
            let t = b.tracks()[0].samples.get(i).unwrap();
            let pts = c["pts"][i].as_i64().unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (pts as u64, pts, 1), "{n}");
            assert_eq!((t.dts, t.pts, t.duration), (s.dts, s.pts, s.duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(0, i, &mut x).unwrap();
            b.read_packet(0, i, &mut y).unwrap();
            assert_eq!(x, y);
        }
    }
    assert!(
        failures.is_empty(),
        "valid empty fragments refused: {failures:?}"
    );
}
#[test]
fn empty_duration_disallows_even_empty_runs_and_checks_its_endpoint() {
    for (n, expected) in [
        (
            "contradictory-run.mp4",
            "empty-duration MP4 fragment contains track runs",
        ),
        ("overflow.mp4", "MP4 empty-fragment timestamp overflow"),
    ] {
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("invalid gap accepted");
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("invalid gap accepted");
        assert!(a.to_string().contains(expected), "{n}: {a}");
        assert!(b.to_string().contains(expected), "{n}: {b}");
    }
}
#[cfg(feature = "player")]
#[test]
fn empty_intervals_play_exact_pictures_and_seek_on_the_authored_clock() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r =
            match fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{n}: {e}"));
                    continue;
                }
            };
        assert_eq!(
            r.duration(),
            Some(Duration::from_millis(c["duration"].as_u64().unwrap() * 20)),
            "{n}"
        );
        let pts: Vec<_> = c["pts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u128)
            .collect();
        let ends = [pts[1], pts[2], pts[2] + 1];
        let mut colors = Vec::new();
        for i in 0..3 {
            colors.push(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
            );
            assert_eq!(r.frame_interval(), Some((pts[i], ends[i], 50)), "{n}");
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        let mut raw = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
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
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")), "{n}");
        }
        assert!(raw.read_frame().unwrap().is_none());
        for ms in [0, 19, 20, 39, 40, 59, 60, 79, 80, 99, 100, 140] {
            let i = ends.iter().position(|e| *e * 20 > ms as u128).unwrap_or(2);
            assert_eq!(
                r.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                colors[i],
                "{n}: seek {ms}"
            );
            assert_eq!(r.frame_interval(), Some((pts[i], ends[i], 50)));
            for j in i + 1..3 {
                assert_eq!(
                    r.read_frame_raw()
                        .unwrap()
                        .unwrap()
                        .into_rgb(32 << 20)
                        .unwrap(),
                    colors[j],
                    "{n}: continuation after {ms}"
                );
                assert_eq!(r.frame_interval(), Some((pts[j], ends[j], 50)));
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            assert!(r.read_frame_raw().unwrap().is_none());
            assert_eq!(r.frame_interval(), Some((pts[2], ends[2], 50)));
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
    assert!(
        failures.is_empty(),
        "playback rejected empty intervals: {failures:?}"
    );
}

#[test]
fn empty_intervals_do_not_consume_sample_budget() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        for samples in [2, 3] {
            let a = fvid::container::mp4::Mp4Reader::open(
                Cursor::new(bytes(n)),
                fvid::container::mp4::Limits {
                    samples,
                    ..Default::default()
                },
            );
            let b = fvid_media::owned_mp4::Mp4Reader::open(
                Cursor::new(bytes(n)),
                fvid_media::owned_mp4::Limits {
                    samples,
                    ..Default::default()
                },
            );
            if samples == 3 {
                assert_eq!(a.unwrap().tracks()[0].samples.len(), 3, "{n}");
                assert_eq!(b.unwrap().tracks()[0].samples.len(), 3, "{n}");
            } else {
                assert!(
                    a.err()
                        .expect("sample limit bypassed")
                        .to_string()
                        .contains("MP4 sample count exceeds budget")
                );
                assert!(
                    b.err()
                        .expect("sample limit bypassed")
                        .to_string()
                        .contains("MP4 sample count exceeds budget")
                );
            }
        }
    }
}
