#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>) -> fvid::Result<Box<dyn AudioStream>> {
    let source: Box<dyn AudioStream> = match kind {
        "wav" => Box::new(fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )?),
        "aiff" => Box::new(fvid::playback_aiff::AiffAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )?),
        _ => unreachable!(),
    };
    Ok(source)
}
#[test]
fn duplicate_audio_payloads_and_contradictory_headers_refuse_before_silently_selecting_a_prefix() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-duplicate-chunks");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let expected = std::fs::read(base.join("expected.f32le")).unwrap();
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut source = match open(kind, bytes.clone()) {
            Ok(source) => source,
            Err(error) => {
                if let Some(needle) = case["refusal"].as_str() {
                    assert!(
                        error.to_string().contains(needle),
                        "{name}: unrelated refusal{error}"
                    );
                    continue;
                }
                panic!("{name} valid control refused: {error}");
            }
        };
        let duration = source.duration();
        let mut decoder = source.make_decoder().unwrap();
        let mut actual = Vec::new();
        let mut intervals = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            actual.extend_from_slice(&frame.packet.data);
            intervals.push((frame.packet.presentation_time(), frame.packet.data));
        }
        if case["refusal"].is_string() {
            wrong.push(format!(
                "{name}: accepted {}frames, duration{duration:?}",
                actual.len() / 4
            ));
            continue;
        }
        assert_eq!(duration, Some(Duration::from_millis(20)));
        assert_eq!(actual, expected, "{name} complete native control");
        capture::worker_check_with_seek(
            open(kind, bytes).unwrap(),
            &intervals,
            Some(Duration::from_millis(20)),
        );
    }
    assert!(
        wrong.is_empty(),
        "duplicate layout/payload ambiguities were silently accepted: {wrong:?}"
    );
}
