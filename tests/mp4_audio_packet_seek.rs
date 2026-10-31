#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-audio-packet-seek")
            .join(name),
    )
    .unwrap()
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap()
}
fn ramp(index: usize) -> Vec<u8> {
    let predictor = [128, 1152, 2176][index % 3];
    (1..=64)
        .flat_map(|n| ((predictor + n) as f32 / 32768.0).to_le_bytes())
        .collect()
}
fn complete_pcm() -> Vec<u8> {
    (0..6).flat_map(ramp).collect()
}
#[test]
fn packet_navigation_preserves_reordered_negative_and_last_duplicate_landings() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mut reader = open(name);
        for (query, index) in case["queries"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["expected_indices"].as_array().unwrap())
        {
            let query = query.as_i64().unwrap();
            let index = index.as_u64().unwrap() as usize;
            let pts = case["pts"][index].as_i64().unwrap();
            assert_eq!(reader.seek(query), pts, "{name} target{query}");
            let packet = reader.read_packet().unwrap().unwrap();
            assert_eq!(
                (packet.sample_index, packet.pts, packet.duration),
                (index, pts, 64)
            );
            let mut decoder = reader.make_decoder().unwrap();
            let decoded = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap();
            assert_eq!(decoded.packet.data, ramp(index), "{name} selected{index}");
        }
    }
}
#[test]
fn ordered_playback_and_fractional_seeks_keep_analytic_pcm_and_eof() {
    let mut reader = open("ordered.mp4");
    let wanted = complete_pcm();
    for target in [0, 1, 63, 64, 65, 383, 384, i64::MAX, -1, 65, 0] {
        let index = (target.max(0) / 64).min(5);
        assert_eq!(reader.seek_to(target), index * 64);
        let frames = transitions::collect(&mut reader);
        let pcm: Vec<_> = frames.iter().flat_map(|p| p.3.iter().copied()).collect();
        let first = target.max(0).min(384) as usize;
        assert_eq!(pcm, wanted[first * 4..], "target{target}");
        if let Some(p) = frames.first() {
            assert_eq!(p.2, Duration::from_micros(first as u64 * 125));
        }
    }
    reader.rewind();
    assert_eq!(
        transitions::collect(&mut reader)
            .into_iter()
            .flat_map(|p| p.3)
            .collect::<Vec<_>>(),
        wanted
    );
}
#[test]
fn every_synthetic_companion_keeps_exact_video_and_seek() {
    for name in [
        "ordered.mp4",
        "reordered.mp4",
        "duplicate.mp4",
        "negative.mp4",
        "equal.mp4",
    ] {
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for i in 0..6 {
            match reader.read_frame_raw().unwrap().unwrap() {
                fvid::playback_native::RawFrame::Planar(p) => {
                    assert_eq!(p.frame.data, vec![128; 384])
                }
                _ => panic!("owned neutral video planes"),
            }
            assert_eq!(reader.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind().unwrap();
        assert!(
            reader
                .seek_raw(Duration::from_millis(45))
                .unwrap()
                .is_some()
        );
        assert_eq!(reader.frame_interval(), Some((2, 3, 50)));
    }
}
#[test]
fn actual_ima4_worker_preserves_pcm_after_repeated_seek() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    let wanted = complete_pcm();
    let trace = Arc::new(Mutex::new(Trace::default()));
    let shared = trace.clone();
    let mut worker = AudioPlayback::start(Box::new(open("ordered.mp4")), move || {
        Box::new(Capture::new(shared))
    });
    worker.play();
    ended(&worker, 0);
    for micros in [0, 125, 500, 8000, 33000, 48000, 100000, 0] {
        trace.lock().unwrap().packets.clear();
        worker.seek(Duration::from_micros(micros));
        ended(&worker, worker.generation());
        let actual: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        let first = ((micros * 8000).div_ceil(1_000_000) as usize).min(384);
        assert_eq!(actual, wanted[first * 4..], "{micros}us");
    }
}
