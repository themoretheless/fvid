use fvid_media::owned_aac::{
    aac_ltp_history::LtpHistory,
    aac_ltp_syntax::{LtpData, Usage},
    aac_synthesis::LongSineSynthesis,
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
fn history_and_gain_match_independent_time_axis_oracle() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-history.json")).unwrap();
    let source = bytes("aac-ltp-history-input.f64le");
    let oracle = bytes("aac-ltp-history-reference.f64le");
    for c in m["cases"].as_array().unwrap() {
        let n = c["n"].as_u64().unwrap() as usize;
        let mut state = LtpHistory::new(n).unwrap();
        assert_eq!(state.storage_bytes(), 4 * n * 2);
        for step in c["steps"].as_array().unwrap() {
            let at = step["input_offset"].as_u64().unwrap() as usize;
            let input = doubles(&source[at..at + 2 * n * 8]);
            let saved = state.clone();
            state.update_raw(&input[..n], &input[n..]).unwrap();
            let updated = state.clone();
            state.restore(&saved).unwrap();
            assert_eq!(state, saved);
            state.update_raw(&input[..n], &input[n..]).unwrap();
            assert_eq!(state, updated);
            for row in step["estimates"].as_array().unwrap() {
                let data = LtpData {
                    lag: row["lag"].as_u64().unwrap() as u16,
                    coefficient_index: row["coefficient"].as_u64().unwrap() as u8,
                    usage: Usage::Bands(vec![true, false, true]),
                };
                let at = row["offset"].as_u64().unwrap() as usize;
                let gold = doubles(&oracle[at..at + 2 * n * 8]);
                let mut out = vec![f64::NAN; 2 * n];
                state.estimate_long(&data, &mut out).unwrap();
                assert_eq!(
                    out, gold,
                    "n={n} lag={} coefficient={}",
                    data.lag, data.coefficient_index
                );
                assert_eq!(state, updated);
            }
        }
        state.reset();
        let mut out = vec![1.; 2 * n];
        state
            .estimate_long(
                &LtpData {
                    lag: 0,
                    coefficient_index: 7,
                    usage: Usage::Bands(vec![]),
                },
                &mut out,
            )
            .unwrap();
        assert!(out.iter().all(|x| *x == 0.));
    }
}
#[test]
fn invalid_updates_estimates_and_restore_preserve_buffers() {
    assert!(LtpHistory::new(512).is_err());
    for n in [960, 1024] {
        let mut s = LtpHistory::new(n).unwrap();
        s.update_raw(&vec![123.; n], &vec![456.; n]).unwrap();
        let saved = s.clone();
        let mut bad = vec![0.; n];
        bad[n / 2] = f64::NAN;
        assert!(s.update_raw(&bad, &vec![0.; n]).is_err());
        assert_eq!(s, saved);
        assert!(s.update_raw(&vec![0.; n - 1], &vec![0.; n]).is_err());
        assert_eq!(s, saved);
        for data in [
            LtpData {
                lag: (2 * n + 1) as u16,
                coefficient_index: 0,
                usage: Usage::Bands(vec![]),
            },
            LtpData {
                lag: 0,
                coefficient_index: 8,
                usage: Usage::Bands(vec![]),
            },
            LtpData {
                lag: 0,
                coefficient_index: 0,
                usage: Usage::Bands(vec![false; 41]),
            },
            LtpData {
                lag: 0,
                coefficient_index: 0,
                usage: Usage::Windows(std::array::from_fn(|_| None)),
            },
        ] {
            let mut out = vec![987.; 2 * n];
            assert!(s.estimate_long(&data, &mut out).is_err());
            assert!(out.iter().all(|x| *x == 987.));
            assert_eq!(s, saved);
        }
        let other = LtpHistory::new(if n == 960 { 1024 } else { 960 }).unwrap();
        assert!(s.restore(&other).is_err());
        assert_eq!(s, saved);
    }
}
#[test]
fn raw_synthesis_overlap_can_feed_ltp_history() {
    for n in [960, 1024] {
        let mut synthesis = LongSineSynthesis::new(n).unwrap();
        let mut spectrum = vec![0f32; n];
        spectrum[0] = 100000.;
        let mut pcm = vec![0.; n];
        synthesis.synthesize(&spectrum, &mut pcm).unwrap();
        let saved = synthesis.history();
        assert_eq!(saved.overlap_raw().len(), n);
        assert!(saved.overlap_raw().iter().any(|x| x.abs() > 1.));
        let mut history = LtpHistory::new(n).unwrap();
        history.update_raw(&pcm, saved.overlap_raw()).unwrap();
        let mut prediction = vec![0.; 2 * n];
        history
            .estimate_long(
                &LtpData {
                    lag: 0,
                    coefficient_index: 0,
                    usage: Usage::Bands(vec![true]),
                },
                &mut prediction,
            )
            .unwrap();
        assert!(prediction[..n].iter().any(|x| x.abs() > 1.));
        assert!(prediction[n..].iter().all(|x| *x == 0.));
    }
}
