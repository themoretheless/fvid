#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-aac-split-subtick-seek")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn open(n: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap()
}
fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            a.is_finite() && b.is_finite() && (a - b).abs() < 2e-6,
            "{a}!={b}"
        );
    }
}
fn expected(c: &serde_json::Value) -> Vec<u8> {
    let r = bytes("reference.f32le");
    [
        vec![0; c["leading_silence"].as_u64().unwrap_or(0) as usize * 4],
        r[..48 * 4].to_vec(),
        r[960 * 4..1008 * 4].to_vec(),
    ]
    .concat()
}
#[test]
fn split_edit_pcm_and_video_controls_are_valid() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let oracle = expected(&c);
        let frames = c["frames"].as_u64().unwrap() as usize;
        let mut r = open(name);
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(
                c["frames"].as_u64().unwrap() * 1_000_000_000 / 48000
            ))
        );
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|p| p.3)
            .collect();
        assert_eq!(pcm.len(), frames * 4);
        compare(&pcm, &oracle);
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert_eq!(
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
        }
        assert!(v.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn split_edit_wall_time_seeks_keep_exact_native_suffixes() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let oracle = expected(&c);
        let frames = c["frames"].as_u64().unwrap() as usize;
        let mut r = open(name);
        for target in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_nanos(500001),
            Duration::from_millis(1),
            Duration::from_nanos(1000001),
            Duration::from_nanos(1999999),
            Duration::from_millis(2),
            Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000),
            Duration::MAX,
        ] {
            let first = (target.as_nanos() * 48000)
                .div_ceil(1_000_000_000)
                .min(frames as u128) as usize;
            let at = r.seek_time(target);
            let packets = transitions::collect(&mut r);
            let pcm: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
            if pcm.len() != (frames - first) * 4 {
                failures.push(format!(
                    "{name}:{target:?}:{}bytes expected{}",
                    pcm.len(),
                    (frames - first) * 4
                ));
                continue;
            }
            compare(&pcm, &oracle[first * 4..]);
            let expected = Duration::from_nanos(first as u64 * 1_000_000_000 / 48000);
            if r.time_of(at) != expected {
                failures.push(format!(
                    "{name}:{target:?}:anchor{:?} expected{expected:?}",
                    r.time_of(at)
                ));
            }
            if let Some(p) = packets.first() {
                assert_eq!(p.2, expected);
            }
        }
    }
    for c in cases() {
        let mut r = open(c["file"].as_str().unwrap());
        r.seek_time(Duration::MAX);
        let raw = r.seek_to(0);
        assert_eq!(r.time_of(raw), Duration::ZERO);
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|p| p.3)
            .collect();
        compare(&pcm, &expected(&c));
    }
    assert!(failures.is_empty(), "{failures:?}");
}
#[test]
fn actual_worker_split_seek_keeps_pcm_suffix_and_eof_anchor() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let oracle = expected(&c);
        let frames = c["frames"].as_u64().unwrap() as usize;
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut worker =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        worker.play();
        ended(&worker, 0);
        let full: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        compare(&full, &oracle);
        for target in [
            Duration::from_nanos(1),
            Duration::from_nanos(500001),
            Duration::from_millis(1),
            Duration::from_nanos(1000001),
            Duration::from_millis(2),
            Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000),
            Duration::MAX,
            Duration::ZERO,
        ] {
            trace.lock().unwrap().packets.clear();
            worker.seek(target);
            ended(&worker, worker.generation());
            let first = (target.as_nanos() * 48000)
                .div_ceil(1_000_000_000)
                .min(frames as u128) as usize;
            let guard = trace.lock().unwrap();
            let pcm: Vec<_> = guard
                .packets
                .iter()
                .flat_map(|p| p.3.iter().copied())
                .collect();
            compare(&pcm, &oracle[first * 4..]);
            if let Some(p) = guard.packets.first() {
                assert_eq!(
                    p.2,
                    Duration::from_nanos(first as u64 * 1_000_000_000 / 48000)
                );
            } else {
                assert_eq!(
                    worker.position(),
                    Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000)
                );
            }
        }
    }
}
