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
        .join("tests/fixtures/playback-errors/mp4-clock-description-scan")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap()
}
fn check(pcm: &[u8], frames: usize) {
    assert_eq!(pcm.len(), frames * 4);
    assert!(
        pcm.chunks_exact(4)
            .all(|v| f32::from_le_bytes(v.try_into().unwrap()) == 0.125)
    );
}
#[test]
fn unused_and_selected_header_controls_keep_exact_full_pcm_and_native_video() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let frames = c["frames"].as_u64().unwrap() as usize;
        let mut r = open(name);
        assert_eq!(
            r.track().sample_descriptions.len(),
            c["descriptions"].as_u64().unwrap() as usize
        );
        assert_eq!(r.track().samples.len(), frames);
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(frames as u64 * 1_000_000_000 / 48000))
        );
        let all = transitions::collect(&mut r);
        check(
            &all.into_iter().flat_map(|p| p.3).collect::<Vec<_>>(),
            frames,
        );
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
fn actual_worker_small_controls_rewind_seek_and_finish_with_analytic_pcm() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    for c in cases().into_iter().filter(|c| c["frames"] == 64) {
        let name = c["file"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut worker =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        worker.play();
        ended(&worker, 0);
        check(
            &trace
                .lock()
                .unwrap()
                .packets
                .iter()
                .flat_map(|p| p.3.iter().copied())
                .collect::<Vec<_>>(),
            64,
        );
        for (target, frames) in [
            (Duration::from_nanos(1_000_000_000 / 48000), 63),
            (Duration::MAX, 0),
            (Duration::ZERO, 64),
        ] {
            trace.lock().unwrap().packets.clear();
            worker.seek(target);
            ended(&worker, worker.generation());
            let guard = trace.lock().unwrap();
            check(
                &guard
                    .packets
                    .iter()
                    .flat_map(|p| p.3.iter().copied())
                    .collect::<Vec<_>>(),
                frames,
            );
            if frames == 63 {
                assert_eq!(
                    guard.packets[0].2,
                    Duration::from_nanos(1_000_000_000 / 48000)
                );
            }
            if frames == 0 {
                assert_eq!(
                    worker.position(),
                    Duration::from_nanos(64 * 1_000_000_000 / 48000)
                );
            }
        }
    }
}
