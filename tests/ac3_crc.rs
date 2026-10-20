#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
    )
}
fn decode(bytes: &[u8]) -> fvid::audio::AudioPacket {
    let mut engine = fvid::codec::make_audio_decoder("A_AC3", &[], 48000, 2, 16).unwrap();
    engine
        .decode_packet(bytes, 0, 1536)
        .unwrap()
        .unwrap()
        .packet
}
#[test]
fn crc_damaged_ac3_uses_existing_silence_concealment_instead_of_decoding_corrupt_pcm() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ac3-crc");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let control = std::fs::read(base.join("control.ac3")).unwrap();
    let normal = decode(&control);
    assert!(normal
        .data
        .chunks_exact(4)
        .any(|p| f32::from_le_bytes(p.try_into().unwrap()).abs() > 0.001));
    capture::worker_check_with_seek(
        open(control),
        &[(Duration::ZERO, normal.data.clone())],
        Some(Duration::from_nanos(1280 * 1_000_000_000 / 48000)),
    );
    let combined = std::fs::read(base.join("valid-then-bad.ac3")).unwrap();
    let joined = decode(&combined);
    let mut wanted = normal.data.clone();
    wanted.extend(vec![0; 1536 * 2 * 4]);
    assert_eq!(
        joined.data, wanted,
        "packet grouping must conceal only the bad second frame"
    );
    capture::worker_check_with_seek(
        open(combined),
        &[
            (Duration::ZERO, normal.data.clone()),
            (
                Duration::from_nanos(1280 * 1_000_000_000 / 48000),
                vec![0; 1536 * 2 * 4],
            ),
        ],
        Some(Duration::from_nanos(2816 * 1_000_000_000 / 48000)),
    );
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let actual = decode(&bytes);
        assert_eq!(actual.data.len(), normal.data.len());
        assert_eq!(actual.presentation_time(), Duration::ZERO);
        if actual.data.iter().any(|b| *b != 0) {
            failures.push(format!(
                "{name}: CRC corruption ignored; nonsilent PCM, differs from clean={}",
                actual.data != normal.data
            ));
        } else {
            capture::worker_check_with_seek(
                open(bytes),
                &[(Duration::ZERO, vec![0; normal.data.len()])],
                Some(Duration::from_nanos(1280 * 1_000_000_000 / 48000)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
