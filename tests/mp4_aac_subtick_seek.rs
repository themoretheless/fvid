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
        .join("tests/fixtures/playback-errors/mp4-aac-subtick-seek")
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
#[test]
fn independent_pcm_full_edit_and_native_video_controls_are_valid() {
    let reference = bytes("reference.f32le");
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let start = c["start_frames"].as_u64().unwrap() as usize;
        let frames = c["frames"].as_u64().unwrap() as usize;
        let mut r = open(name);
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000))
        );
        let data: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        compare(&data, &reference[start * 4..(start + frames) * 4]);
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert_eq!(
                video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn duration_seek_keeps_native_sample_floor_beneath_media_ticks() {
    let reference = bytes("reference.f32le");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let start = c["start_frames"].as_u64().unwrap() as usize;
        let frames = c["frames"].as_u64().unwrap() as usize;
        let end = Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000);
        let mut r = open(name);
        for target in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_micros(500),
            Duration::from_nanos(500001),
            end - Duration::from_nanos(1),
            end,
            end + Duration::from_nanos(1),
            Duration::MAX,
        ] {
            let first = (target.as_nanos() * 48000)
                .div_ceil(1_000_000_000)
                .min(frames as u128) as usize;
            let at = r.seek_time(target);
            let got = transitions::collect(&mut r);
            let pcm: Vec<_> = got.iter().flat_map(|f| f.3.iter().copied()).collect();
            if pcm.len() != (frames - first) * 4 {
                failures.push(format!(
                    "{name}:{target:?}:{}bytes expected{}",
                    pcm.len(),
                    (frames - first) * 4
                ));
                continue;
            }
            compare(&pcm, &reference[(start + first) * 4..(start + frames) * 4]);
            let expected = if first == frames {
                end
            } else {
                Duration::from_nanos(first as u64 * 1_000_000_000 / 48000)
            };
            if r.time_of(at) != expected {
                failures.push(format!(
                    "{name}:{target:?}:anchor{:?},expected{expected:?}",
                    r.time_of(at)
                ));
            }
            if let Some(packet) = got.first() {
                assert_eq!(packet.2, expected, "{name}");
            }
        }
        let raw = r.seek_to(0);
        assert_eq!(r.time_of(raw), Duration::ZERO);
    }
    assert!(failures.is_empty(), "{failures:?}");
}
#[test]
fn actual_worker_seeks_emit_only_the_requested_native_suffix() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    let reference = bytes("reference.f32le");
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let start = c["start_frames"].as_u64().unwrap() as usize;
        let frames = c["frames"].as_u64().unwrap() as usize;
        let end = Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000);
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut worker =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        worker.play();
        ended(&worker, 0);
        for target in [
            Duration::from_nanos(1),
            Duration::from_nanos(500001),
            end - Duration::from_nanos(1),
            end,
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
            compare(&pcm, &reference[(start + first) * 4..(start + frames) * 4]);
            if first == frames {
                assert_eq!(worker.position(), end, "{name}:EOF anchor");
            }
            if let Some(packet) = guard.packets.first() {
                assert_eq!(
                    packet.2,
                    Duration::from_nanos(first as u64 * 1_000_000_000 / 48000),
                    "{name}:first PCM timestamp"
                );
            }
        }
    }
}
#[test]
fn duration_seek_anchor_keeps_fractional_source_sample_phase() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-aac-fractional-origin");
    for (name, factor, shift) in [
        ("factor4-shift-5.mp4", 4i64, -5i64),
        ("factor4-shift1.mp4", 4, 1),
        ("factor65537-shift1.mp4", 65537, 1),
    ] {
        let mut r = Mp4AudioReader::open(
            Cursor::new(std::fs::read(base.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        for source in [3, 1024, 3000] {
            let target = Duration::from_nanos(
                ((source * factor + shift) as u64) * 1_000_000_000 / (48000 * factor as u64),
            );
            let marker = r.seek_time(target);
            let pcm = transitions::collect(&mut r);
            assert!(!pcm.is_empty());
            assert_eq!(r.time_of(marker), pcm[0].2, "{name}:{target:?}");
        }
    }
}
