use fvid_media::owned_aac::{
    aac_ltp_analysis::LtpAnalysis,
    aac_synthesis::{WindowSequence, WindowShape},
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
fn doubles(raw: &[u8]) -> Vec<f64> {
    raw.chunks_exact(8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
        .collect()
}
#[test]
fn fft_ltp_analysis_matches_independent_windowed_direct_mdct() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-analysis.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 28);
    let raw = bytes("aac-ltp-analysis-input.f64le");
    let gold = bytes("aac-ltp-analysis-reference.f64le");
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let at = c["input_offset"].as_u64().unwrap() as usize;
        let input = doubles(&raw[at..at + 2 * n * 8]);
        let bins = c["bins"].as_array().unwrap();
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let reference = doubles(&gold[at..at + bins.len() * 8]);
        let sequence = match c["sequence"].as_u64().unwrap() {
            0 => WindowSequence::OnlyLong,
            1 => WindowSequence::LongStart,
            3 => WindowSequence::LongStop,
            _ => panic!(),
        };
        let shape = |v: &Value| {
            if v == 0 {
                WindowShape::Sine
            } else {
                WindowShape::Kbd
            }
        };
        let mut a = LtpAnalysis::new(n).unwrap();
        let mut out = vec![0.; n];
        a.analyze(
            &input,
            sequence,
            shape(&c["previous"]),
            shape(&c["current"]),
            &mut out,
        )
        .unwrap();
        for (bin, expected) in bins.iter().zip(reference) {
            let k = bin.as_u64().unwrap() as usize;
            assert!(
                (out[k] - expected).abs() < 2e-7 + expected.abs() * 2e-11,
                "n={n} seq={sequence:?} bin={k}: {} vs {expected}",
                out[k]
            );
        }
        let first = out.clone();
        a.analyze(
            &input,
            sequence,
            shape(&c["previous"]),
            shape(&c["current"]),
            &mut out,
        )
        .unwrap();
        assert_eq!(out, first);
    }
}
#[test]
fn invalid_analysis_preserves_output_and_is_reusable() {
    assert!(LtpAnalysis::new(512).is_err());
    for n in [960, 1024] {
        let mut a = LtpAnalysis::new(n).unwrap();
        let mut out = vec![123.; n];
        let mut input = vec![1.; 2 * n];
        input[17] = f64::NAN;
        assert!(
            a.analyze(
                &input,
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                WindowShape::Sine,
                &mut out
            )
            .is_err()
        );
        assert!(out.iter().all(|x| *x == 123.));
        input.fill(f64::MAX);
        assert!(
            a.analyze(
                &input,
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                WindowShape::Sine,
                &mut out
            )
            .is_err()
        );
        assert!(out.iter().all(|x| *x == 123.));
        input.fill(0.);
        assert!(
            a.analyze(
                &input,
                WindowSequence::EightShort,
                WindowShape::Sine,
                WindowShape::Sine,
                &mut out
            )
            .unwrap_err()
            .to_string()
            .contains("short-window LTP")
        );
        assert!(out.iter().all(|x| *x == 123.));
        assert!(
            a.analyze(
                &input[..n],
                WindowSequence::OnlyLong,
                WindowShape::Sine,
                WindowShape::Sine,
                &mut out
            )
            .is_err()
        );
        assert!(out.iter().all(|x| *x == 123.));
        a.analyze(
            &input,
            WindowSequence::OnlyLong,
            WindowShape::Sine,
            WindowShape::Sine,
            &mut out,
        )
        .unwrap();
        assert!(out.iter().all(|x| *x == 0.));
    }
}

#[test]
fn selected_ltp_bands_match_independent_interval_oracle() {
    use fvid_media::owned_aac::aac_ltp_analysis::apply_long_prediction;
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-bands.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 512);
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let mask = c["mask"].as_u64().unwrap();
        let offsets: Vec<usize> = c["offsets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        let used: Vec<bool> = (0..8).map(|b| mask & (1 << b) != 0).collect();
        let mut residual: Vec<f32> = (0..n).map(|i| (i as i32 % 29 - 14) as f32 * 0.25).collect();
        let prediction: Vec<f64> = (0..n).map(|i| (i as i32 % 17 - 8) as f64 * 0.125).collect();
        let pointer = residual.as_ptr();
        apply_long_prediction(&mut residual, &prediction, &offsets, &used).unwrap();
        assert_eq!(residual.as_ptr(), pointer);
        for (i, expected) in c["expected"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                residual[i],
                expected.as_f64().unwrap() as f32,
                "n={n} mask={mask} bin={i}"
            );
        }
    }
}
#[test]
fn ltp_band_failure_is_transactional_and_band_40_is_not_applied() {
    use fvid_media::owned_aac::aac_ltp_analysis::apply_long_prediction;
    for n in [960, 1024] {
        let mut residual = vec![1.; n];
        let mut prediction = vec![2.; n];
        let offsets: Vec<usize> = (0..=40).chain(std::iter::once(n)).collect();
        apply_long_prediction(&mut residual, &prediction, &offsets, &[true; 40]).unwrap();
        assert!(residual[..40].iter().all(|x| *x == 3.));
        assert!(residual[40..].iter().all(|x| *x == 1.));
        let before = residual.clone();
        assert!(apply_long_prediction(&mut residual, &prediction, &offsets, &[true; 41]).is_err());
        assert_eq!(residual, before);
        prediction[n - 1] = f64::MAX;
        assert!(
            apply_long_prediction(&mut residual, &prediction, &[0, 16, n], &[true, true]).is_err()
        );
        assert_eq!(residual, before);
        // An unselected finite prediction cannot overflow the residual.
        apply_long_prediction(&mut residual, &prediction, &[0, 16, n], &[false, false]).unwrap();
        assert_eq!(residual, before);
        prediction[n - 1] = f64::NAN;
        assert!(apply_long_prediction(&mut residual, &prediction, &[0, n], &[false]).is_err());
        assert_eq!(residual, before);
        prediction.fill(0.);
        for bad in [vec![1, n], vec![0, 0, n], vec![0, n + 1], vec![]] {
            assert!(apply_long_prediction(&mut residual, &prediction, &bad, &[]).is_err());
            assert_eq!(residual, before);
        }
        assert!(
            apply_long_prediction(&mut residual, &prediction[..n - 1], &[0, n], &[true]).is_err()
        );
        assert_eq!(residual, before);
        apply_long_prediction(&mut residual, &prediction, &[0, n], &[]).unwrap();
        assert_eq!(residual, before);
    }
}

#[test]
fn composed_ltp_prediction_matches_independent_direct_reference() {
    use fvid_media::owned_aac::{
        aac_ltp_history::LtpHistory,
        aac_ltp_syntax::{LtpData, Usage},
        aac_tns::{TnsData, TnsFilter},
    };
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-pipeline.json")).unwrap();
    let raw = bytes("aac-ltp-pipeline-input.f64le");
    let reference = bytes("aac-ltp-pipeline-reference.f32le");
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let at = c["input_offset"].as_u64().unwrap() as usize;
        let blocks = doubles(&raw[at..at + 3 * n * 8]);
        let mut history = LtpHistory::new(n).unwrap();
        history.update_raw(&blocks[..n], &vec![0.; n]).unwrap();
        history
            .update_raw(&blocks[n..2 * n], &blocks[2 * n..])
            .unwrap();
        let saved = history.clone();
        let offsets: Vec<_> = c["offsets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        let data = LtpData {
            lag: c["lag"].as_u64().unwrap() as u16,
            coefficient_index: c["coefficient"].as_u64().unwrap() as u8,
            usage: Usage::Bands(
                c["used"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect(),
            ),
        };
        let tns = TnsData {
            windows: vec![
                c["filters"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| TnsFilter {
                        length: f["length"].as_u64().unwrap() as usize,
                        reverse: f["reverse"].as_bool().unwrap(),
                        lpc: f["lpc"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect(),
                    })
                    .collect(),
            ],
        };
        let seq = match c["sequence"].as_u64().unwrap() {
            0 => WindowSequence::OnlyLong,
            1 => WindowSequence::LongStart,
            _ => WindowSequence::LongStop,
        };
        let shape = |v: &Value| {
            if v == 0 {
                WindowShape::Sine
            } else {
                WindowShape::Kbd
            }
        };
        let source: Vec<f32> = (0..n).map(|i| (i as i32 % 19 - 9) as f32 * 0.25).collect();
        let mut residual = source.clone();
        let mut analysis = LtpAnalysis::new(n).unwrap();
        analysis
            .predict_long(
                &history,
                &data,
                seq,
                shape(&c["previous"]),
                shape(&c["current"]),
                &offsets,
                4,
                Some(&tns),
                &mut residual,
            )
            .unwrap();
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        for (i, chunk) in reference[at..at + n * 4].chunks_exact(4).enumerate() {
            let expected = f32::from_le_bytes(chunk.try_into().unwrap());
            assert!(
                (residual[i] - expected).abs() <= 0.00003 + expected.abs() * 2e-7,
                "n={n} seq={seq:?} bin={i}: {} vs {expected}",
                residual[i]
            );
        }
        assert_eq!(history, saved);
        let first = residual.clone();
        residual.copy_from_slice(&source);
        analysis
            .predict_long(
                &history,
                &data,
                seq,
                shape(&c["previous"]),
                shape(&c["current"]),
                &offsets,
                4,
                Some(&tns),
                &mut residual,
            )
            .unwrap();
        assert_eq!(first, residual);
        // A nested late TNS failure must not expose an intermediate spectrum.
        let bad = TnsData {
            windows: vec![vec![TnsFilter {
                length: 4,
                reverse: false,
                lpc: vec![f64::NAN],
            }]],
        };
        assert!(
            analysis
                .predict_long(
                    &history,
                    &data,
                    seq,
                    WindowShape::Sine,
                    WindowShape::Sine,
                    &offsets,
                    4,
                    Some(&bad),
                    &mut residual
                )
                .is_err()
        );
        assert_eq!(first, residual);
        assert_eq!(history, saved);
    }
}
#[test]
fn composed_ltp_prediction_refuses_mismatched_history_without_residual_write() {
    use fvid_media::owned_aac::{
        aac_ltp_history::LtpHistory,
        aac_ltp_syntax::{LtpData, Usage},
    };
    let mut a = LtpAnalysis::new(1024).unwrap();
    let h = LtpHistory::new(960).unwrap();
    let mut residual = vec![123.; 1024];
    let data = LtpData {
        lag: 960,
        coefficient_index: 0,
        usage: Usage::Bands(vec![true]),
    };
    assert!(
        a.predict_long(
            &h,
            &data,
            WindowSequence::OnlyLong,
            WindowShape::Sine,
            WindowShape::Sine,
            &[0, 1024],
            1,
            None,
            &mut residual
        )
        .is_err()
    );
    assert!(residual.iter().all(|v| *v == 123.));
}
