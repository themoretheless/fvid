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
fn time(ticks: u64, rate: u64) -> Duration {
    Duration::from_nanos(ticks * 1_000_000_000 / rate)
}
#[test]
fn raw_ac3_sample_seeks_match_continuous_native_pcm_including_coupled_noise() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/ac3-exact-seek");
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
        let rate = case["rate"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let bytes = std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap();
        let mut source = open(bytes.clone());
        let mut engine = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let f = engine
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let packet = source
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap();
            expected.push((packet.presentation_time(), packet.data));
        }
        let total = expected
            .iter()
            .map(|(_, p)| p.len() / 4 / channels)
            .sum::<usize>() as u64;
        assert_eq!(source.duration(), Some(time(total, rate)));
        for target in [1, rate / 50, 1536, 4000, total - 1, total] {
            let mut suffix = Vec::new();
            for (at, data) in &expected {
                let start = (at.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as u64;
                let frames = data.len() / 4 / channels;
                let cut = target.saturating_sub(start).min(frames as u64) as usize;
                if cut < frames {
                    suffix.push((
                        time(start + cut as u64, rate),
                        data[cut * channels * 4..].to_vec(),
                    ));
                }
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek_expected(
                    open(bytes.clone()),
                    &expected,
                    time(target, rate),
                    &suffix,
                )
            }));
            if result.is_err() {
                failures.push(format!(
                    "{name}: seek sample{target} differs from continuous suffix"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
