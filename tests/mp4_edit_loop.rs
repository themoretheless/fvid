use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-edit-loop")
            .join(n),
    )
    .unwrap()
}
#[test]
fn loop_edits_are_explicit_capability_refusals_and_reserved_bits_are_invalid() {
    let mut accepted = Vec::new();
    for v in [0, 1] {
        for mode in ["loop", "reserved", "loop-reserved"] {
            let n = format!("v{v}-{mode}.mp4");
            let a =
                fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
            let b =
                fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default());
            match (a, b) {
                (Err(a), Err(b)) => {
                    let expected = if mode == "loop" {
                        "looping MP4 edits are not implemented"
                    } else {
                        "reserved MP4 edit-list flags"
                    };
                    assert!(a.to_string().contains(expected), "{n}: {a}");
                    assert!(b.to_string().contains(expected), "{n}: {b}");
                    assert_eq!(matches!(a, fvid::Error::Unsupported(_)), mode == "loop");
                    assert_eq!(b.is_unsupported(), mode == "loop");
                }
                _ => accepted.push(n),
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "silently treated loop/reserved edit flags as finite: {accepted:?}"
    );
}
#[cfg(feature = "player")]
#[test]
fn finite_edits_keep_exact_pixels_clock_seek_and_eof() {
    use std::time::Duration;
    for v in [0, 1] {
        let n = format!("v{v}-finite.mp4");
        let mut raw = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(&n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = raw.read_frame().unwrap().unwrap();
            assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i, 1));
            if i > 0 {
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
        }
        assert!(raw.read_frame().unwrap().is_none());
        let mut r = fvid::playback_native::NativeReader::software(Cursor::new(bytes(&n)), 32 << 20)
            .unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(40)));
        let mut colors = Vec::new();
        for i in 0..2 {
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
        for ms in [0, 19, 20, 39, 40, 100] {
            let i = usize::from(ms >= 20);
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

#[cfg(feature = "player")]
#[test]
fn native_open_does_not_silently_play_a_loop_once() {
    for v in [0, 1] {
        let n = format!("v{v}-loop.mp4");
        match fvid::playback_native::NativeReader::software(Cursor::new(bytes(&n)), 32 << 20) {
            Err(e) => {
                assert!(matches!(e, fvid::Error::Unsupported(_)));
                assert!(
                    e.to_string()
                        .contains("looping MP4 edits are not implemented")
                );
            }
            Ok(_) => panic!("{n}: native playback accepted a loop as a finite edit"),
        }
    }
}
