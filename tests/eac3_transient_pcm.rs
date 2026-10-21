#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use fvid::codec::eac3_decoder::transient::apply_window;
use std::{io::Cursor, path::Path};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-transient-pcm")
            .join(n),
    )
    .unwrap()
}
fn samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn transient_pcm_corrector_matches_independent_scalar_windows_without_touching_neighbours() {
    let raw = read("input.f32le");
    let mut video = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(read("input.mkv")),
        Default::default(),
    )
    .unwrap();
    assert_eq!((video.sample_rate(), video.channels()), (48000, 1));
    let pcm = match video.next_step().unwrap().unwrap() {
        AudioStep::Pcm(p) => p.data,
        AudioStep::Encoded(p) => p.data,
        AudioStep::ResetDecoder => panic!("unexpected reset"),
    };
    assert_eq!(pcm, raw);
    assert!(video.next_step().unwrap().is_none());
    let original = samples(&pcm);
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["decoder_integration_accepted"], false);
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 24);
    for c in manifest["cases"].as_array().unwrap() {
        let mut got = original.clone();
        let expected = samples(&read(c["file"].as_str().unwrap()));
        apply_window(
            &mut got,
            c["origin"].as_i64().unwrap(),
            c["transient"].as_i64().unwrap(),
            c["length"].as_u64().unwrap() as u8,
        )
        .unwrap();
        let start = c["start"].as_u64().unwrap() as usize;
        let end = c["end"].as_u64().unwrap() as usize;
        let mut changed = false;
        for (i, (&actual, &want)) in got.iter().zip(&expected).enumerate() {
            assert!(actual.is_finite());
            assert!(
                (actual - want).abs() <= manifest["absolute_tolerance"].as_f64().unwrap() as f32,
                "{} sample{i}: {actual} vs {want}",
                c["file"]
            );
            if i < start || i >= end {
                assert_eq!(actual.to_bits(), original[i].to_bits());
            } else {
                changed |= actual.to_bits() != original[i].to_bits();
            }
        }
        assert!(changed, "a no-op is not PCM-correction acceptance");
    }
}
#[test]
fn missing_history_future_and_overflow_fail_before_modifying_pcm() {
    for (origin, at, size) in [
        (0, 0, 32),
        (0, 1600, 1400),
        (i64::MAX, i64::MIN, 32),
        (i64::MIN, i64::MAX, 32),
        (0, i64::MAX, 32),
    ] {
        let mut pcm = vec![0.25; size];
        let before = pcm.clone();
        assert!(apply_window(&mut pcm, origin, at, 255).is_err());
        assert_eq!(pcm, before);
    }
}
