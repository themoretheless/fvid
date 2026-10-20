#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn decoded(bytes: Vec<u8>) -> Vec<(Duration, Vec<u8>)> {
    let mut source =
        fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        pcm.push((f.packet.presentation_time(), f.packet.data));
    }
    pcm
}
#[test]
fn fractional_tempo_segments_preserve_event_time_and_complete_audio_windows() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/smf-tempo-remainder");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let expected = decoded(std::fs::read(base.join("control.mid")).unwrap());
    assert_eq!(expected.len(), 3);
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let file = fvid::container::smf::SmfFile::parse(&bytes, &Default::default()).unwrap();
        let actual = decoded(bytes.clone());
        if file.duration() != Duration::from_millis(100) || actual != expected {
            failures.push(format!(
                "{name}: score{:?}, PCMwindows{}, expected100ms/3identicalwindows",
                file.duration(),
                actual.len()
            ));
        } else {
            capture::worker_check_with_seek(
                Box::new(
                    fvid::playback_smf::SmfAudioReader::open(
                        Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                &expected,
                Some(Duration::from_millis(100)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
