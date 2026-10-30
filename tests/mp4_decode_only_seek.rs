use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-decode-only-seek")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn cases() -> [(&'static str, [(i64, i64); 2]); 4] {
    [
        ("ordinary-tail.mp4", [(0, 1), (1, 2)]),
        ("late-tail.mp4", [(0, 1), (1, 2)]),
        ("middle-control.mp4", [(0, 2), (2, 3)]),
        ("leading-control.mp4", [(1, 2), (2, 3)]),
    ]
}
#[test]
fn decode_only_shapes_preserve_exact_colored_pictures_and_intervals() {
    for (n, spans) in cases() {
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..2 {
            let f = r.read_frame().unwrap().unwrap();
            let pixels: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
            assert_eq!(
                (f.presentation_time.ticks, f.duration.ticks),
                (
                    spans[i as usize].0,
                    spans[i as usize].1 - spans[i as usize].0
                )
            );
        }
        assert!(r.read_frame().unwrap().is_none());
        assert!(r.read_frame().unwrap().is_none());
    }
}
#[cfg(feature = "player")]
#[test]
fn native_seek_never_loses_the_last_visible_picture_to_a_decode_only_tail() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for (n, spans) in cases() {
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        assert_eq!(r.duration(), Some(Duration::from_millis(60)));
        let reference: Vec<_> = (0..2)
            .map(|_| {
                r.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap()
            })
            .collect();
        assert_ne!(reference[0], reference[1]);
        assert!(r.read_frame_raw().unwrap().is_none());
        for ms in [0, 19, 20, 39, 40, 59, 60, 80, 2000, 2020, 3000] {
            match r.seek_raw(Duration::from_millis(ms)).unwrap() {
                Some(f) => {
                    let i = spans
                        .iter()
                        .position(|span| ms < span.1 as u64 * 20)
                        .unwrap_or(1);
                    let wanted = Some((spans[i].0 as u128, spans[i].1 as u128, 50));
                    let got = f.into_rgb(32 << 20).unwrap();
                    if got != reference[i] || r.frame_interval() != wanted {
                        failures.push(format!(
                            "{n} seek {ms}: wrong picture/interval {:?}, expected {wanted:?}",
                            r.frame_interval()
                        ));
                        continue;
                    }
                    if i == 0 {
                        assert_eq!(
                            r.read_frame_raw()
                                .unwrap()
                                .unwrap()
                                .into_rgb(32 << 20)
                                .unwrap(),
                            reference[1]
                        );
                        assert_eq!(
                            r.frame_interval(),
                            Some((spans[1].0 as u128, spans[1].1 as u128, 50))
                        );
                    }
                    assert!(r.read_frame_raw().unwrap().is_none());
                }
                None => failures.push(format!("{n} seek {ms} lost the visible endpoint")),
            }
        }
        r.rewind().unwrap();
        assert_eq!(
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            reference[0]
        );
    }
    assert!(
        failures.is_empty(),
        "empty decode-only landing: {failures:?}"
    );
}
#[cfg(feature = "player")]
#[test]
fn empty_replay_is_bounded_and_genuine_leading_gaps_do_not_restart() {
    use std::{
        io::{self, BufRead, Read, Seek, SeekFrom},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    struct Counted {
        source: Cursor<Vec<u8>>,
        read: Arc<AtomicUsize>,
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = self.source.read(b)?;
            self.read.fetch_add(n, Ordering::Relaxed);
            Ok(n)
        }
    }
    impl BufRead for Counted {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.source.fill_buf()
        }
        fn consume(&mut self, n: usize) {
            self.source.consume(n);
            self.read.fetch_add(n, Ordering::Relaxed);
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
            self.source.seek(p)
        }
    }
    for (name, ms, passes, visible) in [
        ("all-hidden.mp4", 80, 2, false),
        ("leading-control.mp4", 0, 1, true),
    ] {
        let data = bytes(name);
        let limit = data.len() * passes;
        let read = Arc::new(AtomicUsize::new(0));
        let mut r = fvid::playback_native::NativeReader::software(
            Counted {
                source: Cursor::new(data),
                read: read.clone(),
            },
            32 << 20,
        )
        .unwrap();
        read.store(0, Ordering::Relaxed);
        assert_eq!(
            r.seek_raw(Duration::from_millis(ms)).unwrap().is_some(),
            visible
        );
        assert!(
            read.load(Ordering::Relaxed) <= limit,
            "{name}: seek exceeded its encoded-byte traversal bound"
        );
        if !visible {
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn seek_decode_errors_remain_errors_and_the_reader_can_recover() {
    use std::time::Duration;
    let mut r = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("corrupt-tail.mp4")),
        32 << 20,
    )
    .unwrap();
    let e = r
        .seek_raw(Duration::from_millis(40))
        .err()
        .expect("broken tail must remain a codec error");
    assert!(e.to_string().contains("invalid VP9 frame marker"), "{e}");
    assert!(r.seek_raw(Duration::ZERO).unwrap().is_some());
    assert_eq!(r.frame_interval(), Some((0, 1, 50)));
}

#[cfg(feature = "player")]
#[test]
fn replay_preserves_trimmed_and_repeated_edit_timeline_and_continuation() {
    use std::time::Duration;
    for (name, spans) in [
        ("middle-trim-edit.mp4", vec![(0u128, 1u128), (1, 2)]),
        (
            "middle-repeat-edit.mp4",
            vec![(0u128, 2u128), (2, 3), (3, 5), (5, 6)],
        ),
    ] {
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        let end = spans.last().unwrap().1 as u64 * 20;
        assert_eq!(r.duration(), Some(Duration::from_millis(end)));
        let reference: Vec<_> = spans
            .iter()
            .map(|&(a, b)| {
                let raw = r.read_frame_raw().unwrap().unwrap();
                assert_eq!(r.frame_interval(), Some((a, b, 50)));
                raw.into_rgb(32 << 20).unwrap()
            })
            .collect();
        assert!(r.read_frame_raw().unwrap().is_none());
        assert_ne!(reference[0], reference[1]);
        for ms in [0, 19, 20, 39, 40, 59, 60, 79, 80, 99, 100, 119, 120, 180] {
            let wanted = spans
                .iter()
                .position(|span| ms < span.1 as u64 * 20)
                .unwrap_or(spans.len() - 1);
            let frame = r.seek_raw(Duration::from_millis(ms)).unwrap().unwrap();
            assert_eq!(
                frame.into_rgb(32 << 20).unwrap(),
                reference[wanted],
                "{name} seek {ms}"
            );
            assert_eq!(
                r.frame_interval(),
                Some((spans[wanted].0, spans[wanted].1, 50))
            );
            for index in wanted + 1..spans.len() {
                assert_eq!(
                    r.read_frame_raw()
                        .unwrap()
                        .unwrap()
                        .into_rgb(32 << 20)
                        .unwrap(),
                    reference[index]
                );
                assert_eq!(
                    r.frame_interval(),
                    Some((spans[index].0, spans[index].1, 50))
                );
            }
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
}
