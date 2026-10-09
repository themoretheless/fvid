use fvid_media::owned_aac::{
    aac_ltp_channel::LtpChannel,
    aac_ltp_syntax::{LtpData, Usage},
    aac_synthesis::{WindowSequence, WindowShape},
    aac_tns::{TnsData, TnsFilter},
};
use serde_json::Value;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn channel_pcm_matches_scalar_history_analysis_tns_and_synthesis() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-channel.json")).unwrap();
    let gold = bytes("aac-ltp-channel-reference.f64le");
    for case in m["cases"].as_array().unwrap() {
        let n = case["n"].as_u64().unwrap() as usize;
        let mut channel = LtpChannel::new(n).unwrap();
        let offsets = [0, 4, 8, n];
        let mut beginning = None;
        for (frame, c) in case["frames"].as_array().unwrap().iter().enumerate() {
            if frame == 0 {
                beginning = Some(channel.checkpoint());
            }
            let residual = c["residual"].as_array().unwrap();
            let mut source = vec![0.; n];
            for (out, v) in source.iter_mut().zip(residual) {
                *out = v.as_f64().unwrap() as f32;
            }
            let data = LtpData {
                lag: n as u16,
                coefficient_index: c["coefficient"].as_u64().unwrap() as u8,
                usage: Usage::Bands(vec![true, false, false]),
            };
            let tns = TnsData {
                windows: vec![vec![
                    TnsFilter {
                        length: 2,
                        reverse: false,
                        lpc: vec![],
                    },
                    TnsFilter {
                        length: 1,
                        reverse: c["reverse"].as_bool().unwrap(),
                        lpc: vec![0.25, -0.0625],
                    },
                ]],
            };
            let shape = if c["shape"] == 0 {
                WindowShape::Sine
            } else {
                WindowShape::Kbd
            };
            let data = c["active"].as_bool().unwrap().then_some(&data);
            let saved = channel.checkpoint();
            let pcm = channel
                .process(
                    source.clone(),
                    data,
                    WindowSequence::OnlyLong,
                    shape,
                    &offsets,
                    3,
                    Some(&tns),
                )
                .unwrap();
            let at = c["reference_offset"].as_u64().unwrap() as usize;
            for (i, v) in gold[at..at + n * 8].chunks_exact(8).enumerate() {
                let expected = f64::from_le_bytes(v.try_into().unwrap());
                assert!(
                    (pcm[i] - expected).abs() < 2e-6,
                    "n={n} frame={frame} sample={i}: {} vs {expected}",
                    pcm[i]
                );
            }
            channel.restore(&saved).unwrap();
            let prepared = channel
                .prepare_spectrum(
                    source.clone(),
                    data,
                    WindowSequence::OnlyLong,
                    shape,
                    &offsets,
                    3,
                    Some(&tns),
                )
                .unwrap();
            let repeated = channel
                .synthesize_spectrum(&prepared, WindowSequence::OnlyLong, shape)
                .unwrap();
            assert_eq!(repeated, pcm);
            // Refusal after a checkpoint must not advance either channel history.
            channel.restore(&saved).unwrap();
            let mut bad = source.clone();
            bad[0] = f32::NAN;
            assert!(
                channel
                    .process(
                        bad,
                        data,
                        WindowSequence::OnlyLong,
                        shape,
                        &offsets,
                        3,
                        Some(&tns)
                    )
                    .is_err()
            );
            let after_failure = channel
                .process(
                    source,
                    data,
                    WindowSequence::OnlyLong,
                    shape,
                    &offsets,
                    3,
                    Some(&tns),
                )
                .unwrap();
            assert_eq!(after_failure, pcm);
        }
        channel.reset();
        let reset = channel.checkpoint();
        channel.restore(beginning.as_ref().unwrap()).unwrap();
        let source = vec![1.; n];
        let a = channel
            .process(
                source.clone(),
                None,
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                &offsets,
                3,
                None,
            )
            .unwrap();
        channel.restore(&reset).unwrap();
        let b = channel
            .process(
                source,
                None,
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                &offsets,
                3,
                None,
            )
            .unwrap();
        assert_eq!(a, b);
    }
}
#[test]
fn mismatched_channel_checkpoint_refuses_and_preserves_pcm() {
    let mut channel = LtpChannel::new(1024).unwrap();
    let saved = channel.checkpoint();
    let other = LtpChannel::new(960).unwrap().checkpoint();
    assert!(channel.restore(&other).is_err());
    let first = channel
        .process(
            vec![123.; 1024],
            None,
            WindowSequence::OnlyLong,
            WindowShape::Kbd,
            &[0, 1024],
            1,
            None,
        )
        .unwrap();
    channel.restore(&saved).unwrap();
    let next = channel
        .process(
            vec![123.; 1024],
            None,
            WindowSequence::OnlyLong,
            WindowShape::Kbd,
            &[0, 1024],
            1,
            None,
        )
        .unwrap();
    assert_eq!(first, next);
}
