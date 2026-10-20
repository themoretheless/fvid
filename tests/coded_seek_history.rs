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
        _ => unreachable!(),
    }
}
fn time(ticks: u64, rate: u64) -> Duration {
    Duration::from_nanos(ticks * 1_000_000_000 / rate)
}
#[test]
fn predictive_coded_seeks_restore_the_continuous_pcm_history() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/coded-seek-history");
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
        let kind = case["kind"].as_str().unwrap();
        let grain = case["grain"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut source = open(kind, bytes.clone());
        let rate = u64::from(source.sample_rate());
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
        for target in [grain, grain + 1, 1, total - 1, total] {
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
            let mut cold = open(kind, bytes.clone());
            assert_eq!(cold.seek_to(target as i64), target as i64, "{name}");
            let mut decoder = cold.make_decoder().unwrap();
            let mut actual = Vec::new();
            while let Some(p) = cold.next_packet().unwrap() {
                if let Some(f) = decoder.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                    if let Some(packet) = cold.present_decoded(f.packet, f.source_pts).unwrap() {
                        actual.push((packet.presentation_time(), packet.data));
                    }
                }
            }
            cold.validate_eof().unwrap();
            assert_eq!(actual, suffix, "{name}: cold seek sample{target}");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                capture::worker_check_with_seek_expected(
                    open(kind, bytes.clone()),
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
