#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn smf_unknown_chunks_do_not_consume_the_declared_track_count() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/smf-unknown-chunks");
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
    let mut reference = None;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let parsed = fvid::container::smf::SmfFile::parse(&bytes, &Default::default());
        if case["bad"].as_bool().unwrap() {
            let refused = parsed.is_err();
            match parsed {
                Ok(_) => failures.push(format!("{name}: truncated chunk accepted")),
                Err(e) => assert!(
                    e.to_string()
                        .contains("runs past the end of the file"),
                    "{name}: unrelated refusal:{e}"
                ),
            };
            assert_eq!(
                fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                    .is_err(),
                refused
            );
        } else {
            let parsed = match parsed {
                Ok(parsed) => parsed,
                Err(e) => { failures.push(format!("{name}: valid extension refused: {e}")); continue; }
            };
            assert_eq!(parsed.duration(), Duration::from_millis(100));
            let mut source = fvid::playback_smf::SmfAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            let mut engine = source.make_decoder().unwrap();
            let mut expected = Vec::new();
            while let Some(p) = source.next_packet().unwrap() {
                let f = engine
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                expected.push((f.packet.presentation_time(), f.packet.data));
            }
            if let Some(ref reference) = reference {
                assert_eq!(&expected, reference, "{name}: unknown chunks changed PCM");
            } else { reference = Some(expected.clone()); }
            capture::worker_check_with_seek(
                Box::new(
                    fvid::playback_smf::SmfAudioReader::open(
                        Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                &expected,
                Some(Duration::from_millis(300)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
