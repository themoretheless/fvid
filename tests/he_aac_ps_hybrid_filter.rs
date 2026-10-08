use fvid_media::owned_aac::{
    aac_ps_hybrid_filter::{Analysis, DELAY, Prototype, synthesize},
    aac_sbr_qmf::Complex,
};
use serde_json::Value;

const PROTOTYPES: [Prototype; 5] = [
    Prototype::TwentyEight,
    Prototype::TwentyTwo,
    Prototype::ThirtyFourTwelve,
    Prototype::ThirtyFourEight,
    Prototype::ThirtyFourFour,
];
fn prototype(name: &str) -> Prototype {
    *PROTOTYPES
        .iter()
        .find(|p| format!("{p:?}") == name)
        .unwrap()
}
fn complex(v: &Value) -> Complex {
    Complex {
        re: v[0].as_f64().unwrap(),
        im: v[1].as_f64().unwrap(),
    }
}
fn close(a: Complex, b: Complex) {
    assert!((a.re - b.re).abs() < 2e-13, "real: {a:?} != {b:?}");
    assert!((a.im - b.im).abs() < 2e-13, "imag: {a:?} != {b:?}");
}
fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-hybrid-filter-oracles.json"
    ))
    .unwrap()
}
#[test]
fn every_raw_subband_matches_independent_decimal_convolution() {
    for row in oracle()["rows"].as_array().unwrap() {
        let p = prototype(row["prototype"].as_str().unwrap());
        let input: Vec<_> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(complex)
            .collect();
        let mut state = Analysis::new(p);
        let output = state.process(&input).unwrap();
        assert_eq!(output.len(), input.len());
        for (slot, expected) in output.iter().zip(row["output"].as_array().unwrap()) {
            assert_eq!(slot.len(), p.subbands());
            for (&actual, expected) in slot.iter().zip(expected.as_array().unwrap()) {
                close(actual, complex(expected));
            }
        }
    }
}
#[test]
fn inverse_sums_recover_all_prototypes_with_exact_six_slot_delay() {
    assert_eq!(DELAY, 6);
    for row in oracle()["rows"].as_array().unwrap() {
        let p = prototype(row["prototype"].as_str().unwrap());
        let input: Vec<_> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(complex)
            .collect();
        let output = Analysis::new(p).process(&input).unwrap();
        for (n, &actual) in synthesize(p, &output).unwrap().iter().enumerate() {
            let expected = if n < DELAY {
                Complex::default()
            } else {
                input[n - DELAY]
            };
            // Q=12 retains the standard's finite decimal precision at g[6].
            close(actual, expected);
        }
    }
}
#[test]
fn arbitrary_chunking_checkpoint_reset_and_empty_calls_preserve_causality() {
    for row in oracle()["rows"].as_array().unwrap() {
        let p = prototype(row["prototype"].as_str().unwrap());
        let input: Vec<_> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(complex)
            .collect();
        let mut complete = Analysis::new(p);
        let expected = complete.process(&input).unwrap();
        for width in [1, 2, 7, 13, 16, 24, 30, 32] {
            let mut state = Analysis::new(p);
            let mut actual = Vec::new();
            for chunk in input.chunks(width) {
                let before = state.clone();
                assert!(state.process(&[]).unwrap().is_empty());
                assert_eq!(state, before);
                let checkpoint = state.clone();
                let part = state.process(chunk).unwrap();
                let mut replay = checkpoint;
                assert_eq!(part, replay.process(chunk).unwrap());
                assert_eq!(state, replay);
                actual.extend(part);
            }
            assert_eq!(actual, expected);
            assert_eq!(state, complete);
            state.reset();
            assert_eq!(state, Analysis::new(p));
            assert_eq!(state.prototype(), p);
        }
    }
}
#[test]
fn invalid_inputs_and_late_overflow_never_commit_history() {
    for p in PROTOTYPES {
        let mut state = Analysis::new(p);
        state
            .process(&[Complex { re: 0.25, im: -0.5 }; 17])
            .unwrap();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let before = state.clone();
            assert!(
                state
                    .process(&[Complex::default(), Complex { re: bad, im: 0. }])
                    .is_err()
            );
            assert_eq!(state, before);
            assert!(synthesize(p, &[vec![Complex { re: 0., im: bad }; p.subbands()]]).is_err());
        }
        assert!(synthesize(p, &[vec![Complex::default(); p.subbands() - 1]]).is_err());
        assert!(p.coefficients(p.subbands()).is_err());
        assert!(
            synthesize(
                p,
                &[vec![
                    Complex {
                        re: f64::MAX,
                        im: 0.
                    };
                    p.subbands()
                ]]
            )
            .is_err()
        );
    }
    let mut state = Analysis::new(Prototype::TwentyTwo);
    let before = state.clone();
    let input: Vec<_> = (0..40)
        .map(|n| Complex {
            re: if n % 2 == 0 { f64::MAX } else { -f64::MAX },
            im: 0.,
        })
        .collect();
    assert!(state.process(&input).is_err());
    assert_eq!(state, before);
}
