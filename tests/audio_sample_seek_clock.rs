#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
    )
}
#[test]
fn seek_to_a_presented_sample_boundary_recovers_the_tick_instead_of_replaying_the_sample() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-sample-seek-clock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let frames = case["frames"].as_i64().unwrap();
        let bytes = std::fs::read(base.join(format!("{name}.wav"))).unwrap();
        let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
        let mut video = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(base.join(format!("{name}.mkv"))).unwrap()),
            32 << 20,
        )
        .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source = open(bytes.clone());
        let end = source.time_of(frames);
        assert_eq!(source.seek_to(frames), frames);
        assert!(source.next_packet().unwrap().is_none());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture::worker_check_with_seek(open(bytes), &[(Duration::ZERO, expected)], Some(end))
        }));
        if result.is_err() {
            failures.push(format!(
                "{name}: raw sample seek is EOF, but worker Duration seek to{end:?} replays audio"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
