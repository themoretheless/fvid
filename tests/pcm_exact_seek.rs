#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "wav" => Box::new(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "aiff" => Box::new(
            fvid::playback_aiff::AiffAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "au" => Box::new(
            fvid::playback_au::AuAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn every_fixed_frame_pcm_width_seeks_to_the_sample_instead_of_rounding_to_a_packet() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/pcm-exact-seek");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for file in ["video.mkv", "long-video.mkv"] {
        let mut video = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(base.join(file)).unwrap()),
            32 << 20,
        )
        .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
    }
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let bytes = std::fs::read(if let Some(file) = case["file"].as_str() {
            base.join(file)
        } else {
            root.join(case["source"].as_str().unwrap())
        })
        .unwrap();
        let reference = std::fs::read(base.join(case["expected"].as_str().unwrap())).unwrap();
        let count = case["frames"].as_u64().unwrap();
        let mut source = open(kind, bytes.clone());
        let mut engine = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let f = engine
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            expected.push((f.packet.presentation_time(), f.packet.data));
        }
        assert!(
            expected
                .iter()
                .flat_map(|(_, p)| p.iter().copied())
                .eq(reference.iter().copied()),
            "{name}: exact numeric baseline"
        );
        let targets: Vec<u64> = if count == 160 {
            vec![1, 7, 79, 159, 160]
        } else {
            vec![1, 2047, 2048, 2049, 4095, 4096, 4097, 6146, 6147]
        };
        for target in targets {
            let mut suffix = Vec::new();
            for (at, data) in &expected {
                let start = (at.as_nanos() * 8000 / 1_000_000_000) as u64;
                let frames = data.len() / 4;
                let cut = target.saturating_sub(start).min(frames as u64) as usize;
                if cut < frames {
                    suffix.push((
                        Duration::from_nanos((start + cut as u64) * 1_000_000_000 / 8000),
                        data[cut * 4..].to_vec(),
                    ));
                }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek_expected(
                    open(kind, bytes.clone()),
                    &expected,
                    Duration::from_nanos(target * 1_000_000_000 / 8000),
                    &suffix,
                )
            }));
            if result.is_err() {
                failures.push(format!("{name}: seek sample{target} rounds or changes PCM"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
