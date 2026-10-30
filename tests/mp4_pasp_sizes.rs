use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-pasp-sizes")
            .join(n),
    )
    .unwrap()
}
#[test]
fn incomplete_or_oversized_pixel_spacing_is_not_silently_ignored() {
    let mut accepted = Vec::new();
    for size in [0, 4, 7, 9, 12] {
        let n = format!("pasp-{size}-bytes.mp4");
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
        match (a, b) {
            (Err(a), Err(b)) => {
                assert!(
                    a.to_string().contains("invalid MP4 pixel-aspect box size"),
                    "{n}: {a}"
                );
                assert!(
                    b.to_string().contains("invalid MP4 pixel-aspect box size"),
                    "{n}: {b}"
                );
                assert!(!matches!(a, fvid::Error::Unsupported(_)));
                assert!(!b.is_unsupported());
            }
            _ => accepted.push(n),
        }
    }
    assert!(
        accepted.is_empty(),
        "silently accepted malformed spacing: {accepted:?}"
    );
}
#[test]
fn exact_spacing_and_absence_preserve_packets_clock_and_fallback_policy() {
    for (n, aspect) in [
        ("wide.mp4", (2, 1)),
        ("absent.mp4", (1, 1)),
        ("zero-fallback.mp4", (1, 1)),
    ] {
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[0].pixel_aspect, aspect);
        assert_eq!(b.tracks()[0].pixel_aspect, aspect);
        for i in [2, 0, 1] {
            let s = a.tracks()[0].samples.get(i).unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (i as u64, i as i64, 1));
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(0, i, &mut x).unwrap();
            b.read_packet(0, i, &mut y).unwrap();
            assert_eq!(x, y);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn exact_spacing_plays_pixels_seek_rewind_and_eof_without_changing_the_clock() {
    use std::time::Duration;
    for n in ["wide.mp4", "absent.mp4", "zero-fallback.mp4"] {
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
            assert_eq!(p, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(raw.read_frame().unwrap().is_none());
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut colors = Vec::new();
        assert_eq!(r.duration(), Some(Duration::from_millis(60)));
        for i in 0..3 {
            colors.push(
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
            );
            assert_eq!(r.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(r.read_frame_raw().unwrap().is_none());
        for ms in [0, 19, 20, 39, 40, 59, 60, 100] {
            let i = (ms / 20).min(2) as usize;
            assert_eq!(
                r.seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                colors[i]
            );
            assert_eq!(r.frame_interval(), Some((i as u128, i as u128 + 1, 50)));
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
