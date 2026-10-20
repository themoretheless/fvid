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
fn timeline_audio_seeks_to_eof_without_replaying_the_last_frame() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/timeline-audio-eof");
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
        let bytes = std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap();
        let mut source = open(kind, bytes.clone());
        let mut decoder = source.make_decoder().unwrap();
        let mut decoded = Vec::new();
        let mut end = 0;
        let mut count = 0;
        let mut encoded = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            decoded.push((f.packet.presentation_time(), f.packet.data));
            end = end.max(p.pts + p.duration);
            count += 1;
            encoded.push(p);
        }
        assert!(count > 0);
        source.validate_eof().unwrap();
        for target in [0, 1, end - 1] {
            let wanted = encoded.iter().rev().find(|p| p.pts <= target).unwrap();
            assert_eq!(
                source.seek_to(target),
                wanted.pts,
                "{kind} in-range landing"
            );
            let actual = source.next_packet().unwrap().unwrap();
            assert_eq!(
                (actual.pts, actual.duration, actual.data.as_slice()),
                (wanted.pts, wanted.duration, wanted.data.as_slice()),
                "{kind} in-range packet"
            );
        }
        let before_failures = failures.len();
        for target in [end, end + 1, i64::MAX] {
            let at = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                source.seek_to(target)
            })) {
                Ok(at) => at,
                Err(_) => {
                    failures.push(format!("{kind} target{target}: seek panicked"));
                    continue;
                }
            };
            let packet = source.next_packet().unwrap();
            if at != end || packet.is_some() {
                failures.push(format!(
                    "{kind} target{target}: landed{at} instead of{end}, replay={}",
                    packet.is_some()
                ));
            }
        }
        let duration = std::time::Duration::from_millis(end as u64);
        if failures.len() == before_failures {
            capture::worker_check_with_seek(open(kind, bytes), &decoded, Some(duration));
        }
        source.rewind();
        assert_eq!(source.seek_to(i64::MIN), 0);
        assert!(source.next_packet().unwrap().is_some());
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
