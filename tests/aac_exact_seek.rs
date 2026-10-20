#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(bytes: Vec<u8>) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_aac::AacAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
    )
}
fn time(ticks: u64, rate: u64) -> Duration {
    Duration::from_nanos(ticks * 1_000_000_000 / rate)
}
#[test]
fn raw_aac_output_sample_seeks_match_continuous_lc_sbr_ps_pcm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/aac-exact-seek");
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

        let bytes = std::fs::read(if let Some(file) = case["file"].as_str() {
            base.join(file)
        } else {
            root.join(case["source"].as_str().unwrap())
        })
        .unwrap();
        let mut source = open(bytes.clone());
        let rate = u64::from(source.sample_rate());
        let channels = usize::from(source.channels());
        let mut engine = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            if let Some(f) = engine
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
            {
                if let Some(packet) = source.present_decoded(f.packet, f.source_pts).unwrap() {
                    expected.push((packet.presentation_time(), packet.data));
                }
            }
        }
        while let Some(f) = engine.finish_packet().unwrap() {
            if let Some(packet) = source.present_decoded(f.packet, f.source_pts).unwrap() {
                expected.push((packet.presentation_time(), packet.data));
            }
        }
        let total = expected
            .iter()
            .map(|(_, p)| p.len() / 4 / channels)
            .sum::<usize>() as u64;
        assert!(source.duration().unwrap().abs_diff(time(total, rate)) <= Duration::from_nanos(1));
        let mut targets = vec![1, 3, rate / 100, 1025, total - 1, total];
        if total > rate * 13 / 10 + 1 {
            targets.push(rate * 13 / 10 + 1);
        }
        for target in targets {
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
            let mut cold = open(bytes.clone());
            assert_eq!(cold.seek_to(target as i64), target as i64);
            let mut engine = cold.make_decoder().unwrap();
            let mut actual = Vec::new();
            while let Some(p) = cold.next_packet().unwrap() {
                if let Some(f) = engine
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(p) = cold.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.push((p.presentation_time(), p.data));
                    }
                }
            }
            while let Some(f) = engine.finish_packet().unwrap() {
                if let Some(p) = cold.present_decoded(f.packet, f.source_pts).unwrap() {
                    actual.push((p.presentation_time(), p.data));
                }
            }
            assert!(
                actual == suffix,
                "{name}: cold source seek{target} differs from exact suffix"
            );
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
