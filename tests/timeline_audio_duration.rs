#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[path = "support/audio_capture.rs"]
mod capture;
use std::{io::Cursor, path::Path};
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "smf" => Box::new(
            fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "xm" => Box::new(
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn source_duration_includes_the_release_endpoint_presented_by_native_workers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/timeline-audio-duration");
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
        let kind = case["kind"].as_str().unwrap();
        let bytes = std::fs::read(if let Some(file) = case["file"].as_str() {
            base.join(file)
        } else {
            root.join(case["source"].as_str().unwrap())
        })
        .unwrap();
        let mut source = open(kind, bytes.clone());
        let stated = source.duration().unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        let mut end = std::time::Duration::ZERO;
        let mut scheduled_end = end;
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            scheduled_end = source.time_of(p.pts + p.duration);
            let start = f.packet.presentation_time();
            let frames = f.packet.data.len() / 4 / usize::from(source.channels());
            end = start
                + std::time::Duration::from_nanos(
                    frames as u64 * 1_000_000_000 / u64::from(source.sample_rate()),
                );
            expected.push((start, f.packet.data));
        }
        if case["file"].is_null() {
            assert_eq!(
                end,
                std::time::Duration::from_millis(if kind == "smf" { 2200 } else { 480 })
            );
        }
        assert!(
            scheduled_end >= end
                && scheduled_end - end
                    < std::time::Duration::from_nanos(
                        1_000_000_000 / u64::from(source.sample_rate()) + 1
                    )
        );
        if stated != scheduled_end {
            failures.push(format!(
                "{kind}: advertised{stated:?} versus actual rendered{end:?}"
            ));
        }
        assert_eq!(source.duration(), Some(stated));
        source.rewind();
        assert_eq!(source.duration(), Some(stated));
        capture::worker_check_with_seek(open(kind, bytes), &expected, Some(scheduled_end));
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
