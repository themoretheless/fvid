use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-edit-window-eof")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn raw_controls_keep_the_two_authored_colored_pictures() {
    for name in ["source-tail.mp4", "source-middle.mp4", "source-leading.mp4"] {
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for color in 0..2 {
            let f = r.read_frame().unwrap().unwrap();
            let p: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(p, bytes(&format!("frame-{color}.yuv16")));
        }
        assert!(r.read_frame().unwrap().is_none());
    }
}
#[cfg(feature = "player")]
fn colors(source: &str) -> Vec<Vec<u8>> {
    let mut r = fvid::playback_native::NativeReader::software(Cursor::new(bytes(source)), 32 << 20)
        .unwrap();
    (0..2)
        .map(|_| {
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect()
}
#[cfg(feature = "player")]
#[test]
fn every_edit_window_runs_and_eof_does_not_retime_the_last_picture() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let palette = colors(c["source"].as_str().unwrap());
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        assert_eq!(
            r.duration(),
            Some(Duration::from_millis(c["duration"].as_u64().unwrap() * 20))
        );
        for _ in 0..2 {
            let mut got = Vec::new();
            let mut spans = Vec::new();
            while let Some(raw) = r.read_frame_raw().unwrap() {
                got.push(raw.into_rgb(32 << 20).unwrap());
                spans.push(r.frame_interval().unwrap());
                assert!(got.len() < 16, "edit transition loop");
            }
            let expected: Vec<_> = c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| palette[f["color"].as_u64().unwrap() as usize].clone())
                .collect();
            let times: Vec<_> = c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["start"].as_u64().unwrap() as u128,
                        f["end"].as_u64().unwrap() as u128,
                        50,
                    )
                })
                .collect();
            if got != expected || spans != times {
                failures.push(format!(
                    "{n}: {} pictures, {spans:?}, expected {times:?}",
                    got.len()
                ));
            }
            if let Some(last) = spans.last() {
                if r.frame_interval() != Some(*last) {
                    failures.push(format!(
                        "{n}: EOF changed last interval from {last:?} to {:?}",
                        r.frame_interval()
                    ));
                }
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            r.rewind().unwrap();
        }
    }
    assert!(
        failures.is_empty(),
        "edit program stopped or retimed output: {failures:?}"
    );
}
#[cfg(feature = "player")]
#[test]
fn seek_keeps_edit_positions_and_continuation_across_empty_windows() {
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let palette = colors(c["source"].as_str().unwrap());
        let frames = c["frames"].as_array().unwrap();
        let mut r =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for ms in [0, 19, 20, 39, 40, 59, 60, 79, 80, 99, 100, 119, 120, 180] {
            let i = frames
                .iter()
                .position(|f| ms < f["end"].as_u64().unwrap() * 20)
                .unwrap_or(frames.len() - 1);
            let target = &frames[i];
            let span = (
                target["start"].as_u64().unwrap() as u128,
                target["end"].as_u64().unwrap() as u128,
                50,
            );
            let Some(raw) = r.seek_raw(Duration::from_millis(ms)).unwrap() else {
                failures.push(format!("{n} seek {ms}: no picture"));
                continue;
            };
            if raw.into_rgb(32 << 20).unwrap()
                != palette[target["color"].as_u64().unwrap() as usize]
                || r.frame_interval() != Some(span)
            {
                failures.push(format!(
                    "{n} seek {ms}: {:?}, expected {span:?}",
                    r.frame_interval()
                ));
                continue;
            }
            for next in frames.iter().skip(i + 1) {
                let Some(raw) = r.read_frame_raw().unwrap() else {
                    failures.push(format!("{n} seek {ms}: missing continuation"));
                    break;
                };
                assert_eq!(
                    raw.into_rgb(32 << 20).unwrap(),
                    palette[next["color"].as_u64().unwrap() as usize]
                );
                assert_eq!(
                    r.frame_interval(),
                    Some((
                        next["start"].as_u64().unwrap() as u128,
                        next["end"].as_u64().unwrap() as u128,
                        50
                    ))
                );
            }
            assert!(r.read_frame_raw().unwrap().is_none());
        }
    }
    assert!(failures.is_empty(), "edit seek coverage: {failures:?}");
}
#[cfg(feature = "player")]
#[test]
fn no_picture_programs_are_bounded_and_codec_errors_are_not_window_eof() {
    use std::{
        io::{self, BufRead, Read, Seek, SeekFrom},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
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
        fn seek(&mut self, s: SeekFrom) -> io::Result<u64> {
            self.source.seek(s)
        }
    }
    for n in ["all-hidden-many.mp4", "preroll-many.mp4"] {
        let data = bytes(n);
        let bound = data.len();
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
        assert!(r.read_frame_raw().unwrap().is_none());
        assert!(
            read.load(Ordering::Relaxed) <= bound,
            "{n}: repeatedly decoded a globally empty presentation"
        );
    }
    let mut r = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("corrupt-window.mp4")),
        32 << 20,
    )
    .unwrap();
    let error = loop {
        match r.read_frame_raw() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("codec error skipped as empty window"),
            Err(e) => break e,
        }
    };
    assert!(
        error.to_string().contains("invalid VP9 frame marker"),
        "{error}"
    );
}
