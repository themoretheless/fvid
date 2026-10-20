#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn base() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/midi-fine-tuning")
}
fn open(bytes: Vec<u8>) -> fvid::playback_smf::SmfAudioReader {
    fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default()).unwrap()
}
fn render(bytes: Vec<u8>, channels: u16) -> Vec<(Duration, Vec<u8>)> {
    let mut source = open(bytes);
    let mut decoder =
        fvid::codec::midi_decoder::MidiDecoder::new(source.sample_rate(), channels).unwrap();
    let mut out = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        out.push((f.packet.presentation_time(), f.packet.data));
    }
    out
}
#[test]
fn midi_fine_tuning_is_supported() {
    let base = base();
    assert_eq!(
        render(std::fs::read(base.join("value-16382.mid")).unwrap(), 2),
        render(std::fs::read(base.join("value-16382-bend.mid")).unwrap(), 2)
    );
}
#[test]
fn midi_fine_tuning_acceptance_matches_independent_pitch_controls() {
    let base = base();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(base.join(name)).unwrap();
        let control = std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();
        assert_eq!(
            open(bytes.clone()).duration(),
            open(control.clone()).duration()
        );
        let mut stereo = Vec::new();
        for channels in [1, 2] {
            let expected = render(control.clone(), channels);
            if render(bytes.clone(), channels) != expected {
                failures.push(format!(
                    "{name}/{channels}: nativeRPN1differs from independent pitch control"
                ));
            }
            if channels == 2 {
                stereo = expected;
            }
        }
        let end = open(bytes.clone()).duration().unwrap();
        for target in [Duration::from_millis(100), end] {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek(
                    Box::new(open(bytes.clone())),
                    &stereo,
                    Some(target),
                )
            }))
            .is_err()
            {
                failures.push(format!("{name}: actualRPN1playback differs"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
