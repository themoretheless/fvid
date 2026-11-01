#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{
    audio_thread::{AudioEvent, AudioPlayback},
    playback_mp4_audio::Mp4AudioReader,
};
use std::{
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[test]
fn seeking_inside_unavailable_source_reports_error_instead_of_successful_eof() {
    let data = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/playback-errors/mp4-native-grid-missing-extent-seek/past-source-edit.mp4",
    ))
    .unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(data.clone()), 32 << 20).unwrap();
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
    let stream = Mp4AudioReader::open(Cursor::new(data), Default::default()).unwrap();
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let shared = trace.clone();
    let mut worker = AudioPlayback::start(Box::new(stream), move || {
        Box::new(transitions::Capture::new(shared))
    });
    worker.seek(Duration::from_nanos(1100001));
    worker.play();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            Instant::now() < deadline,
            "unavailable source seek did not terminate"
        );
        while let Some(event) = worker.poll() {
            match event {
                AudioEvent::Error(e) => {
                    assert!(e.contains("edit extends beyond source packets"), "{e}");
                    assert!(trace.lock().unwrap().packets.is_empty());
                    return;
                }
                AudioEvent::Ended(_) => {
                    panic!("unavailable source seek falsely reported successful EOF")
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
