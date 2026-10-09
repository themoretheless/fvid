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
