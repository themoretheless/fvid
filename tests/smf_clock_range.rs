#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn smf_wide_clock_products_are_checked_after_division_and_refuse_unrepresentable_time() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/smf-clock-range");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
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
        let result = std::panic::catch_unwind(|| {
            fvid::container::smf::SmfFile::parse(&bytes, &Default::default())
        });
        match result {
            Err(_) => failures.push(format!("{name}: parser panicked")),
            Ok(Err(e)) => {
                assert!(
                    case["microseconds"].is_null(),
                    "{name}: valid wide clock refused:{e}"
                );
                assert!(
                    e.to_string()
                        .contains("timeline exceeds the microsecond range"),
                    "{name}: wrong refusal:{e}"
                );
            }
            Ok(Ok(file)) => {
                if let Some(expected) = case["microseconds"].as_u64() {
                    assert_eq!(file.duration().as_micros(), u128::from(expected), "{name}");
                } else {
                    failures.push(format!("{name}: unrepresentable clock accepted"));
                }
                if name == "control" {
                    let mut source = fvid::playback_smf::SmfAudioReader::open(
                        Cursor::new(bytes.clone()),
                        Default::default(),
                    )
                    .unwrap();
                    let mut decoder = source.make_decoder().unwrap();
                    let mut expected = Vec::new();
                    while let Some(p) = source.next_packet().unwrap() {
                        let f = decoder
                            .decode_packet(&p.data, p.pts, p.duration as u64)
                            .unwrap()
                            .unwrap();
                        expected.push((f.packet.presentation_time(), f.packet.data));
                    }
                    capture::worker_check_with_seek(
                        Box::new(
                            fvid::playback_smf::SmfAudioReader::open(
                                Cursor::new(bytes),
                                Default::default(),
                            )
                            .unwrap(),
                        ),
                        &expected,
                        Some(std::time::Duration::from_millis(300)),
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
