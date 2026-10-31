use fvid_media::owned_aac::{
    aac_ld_mps_decor::{Decorrelator, Mode, design},
    aac_ld_mps_ott::Ott,
    aac_sbr_qmf::Complex,
};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-upmix")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-11 * (1.0 + b.abs()), "{a} != {b}");
}
fn mode(c: &Value) -> Mode {
    if c["mode"] == "real" {
        Mode::Real
    } else {
        Mode::Complex
    }
}
fn samples(blob: &[u8], at: usize, slots: usize, bands: usize) -> Vec<Vec<Complex>> {
    (0..slots)
        .map(|t| {
            (0..bands)
                .map(|b| {
                    let p = at + (t * bands + b) * 16;
                    Complex {
                        re: f64::from_le_bytes(blob[p..p + 8].try_into().unwrap()),
                        im: f64::from_le_bytes(blob[p + 8..p + 16].try_into().unwrap()),
                    }
                })
                .collect()
        })
        .collect()
}
fn stream(d: &mut Decorrelator, input: &[Vec<Complex>], pattern: &[usize]) -> Vec<Vec<Complex>> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut step = 0;
    while at < input.len() {
        let end = (at + pattern[step % pattern.len()]).min(input.len());
        out.extend(d.process(&input[at..end]).unwrap());
        at = end;
        step += 1;
    }
    out
}
fn same_bits(a: &[Vec<Complex>], b: &[Vec<Complex>]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(a.re.to_bits(), b.re.to_bits());
            assert_eq!(a.im.to_bits(), b.im.to_bits());
        }
    }
}
#[test]
fn all_ott_covariances_residual_limits_and_complex_basis_outputs_match_decimal_oracle() {
    let m = manifest();
    assert_eq!(m["matrices"].as_array().unwrap().len(), 496);
    for c in m["matrices"].as_array().unwrap() {
        let residual = c["residual"].as_bool().unwrap();
        let matrix = Ott::from_indices(
            c["cld"].as_i64().unwrap() as i16,
            c["icc"].as_i64().unwrap() as i16,
            residual,
        )
        .unwrap();
        for row in 0..2 {
            for col in 0..3 {
                close(matrix.rows[row][col], c["rows"][row][col].as_f64().unwrap());
            }
            close(matrix.channel_gains[row], c["gains"][row].as_f64().unwrap());
        }
        close(
            matrix.effective_coherence,
            c["effective_coherence"].as_f64().unwrap(),
        );
        for basis in 0..3 {
            let mut input = [Complex::default(); 3];
            input[basis] = Complex { re: 1.25, im: -0.5 };
            let out = matrix.apply(input[0], input[1], input[2]).unwrap();
            for row in 0..2 {
                close(out[row].re, 1.25 * c["rows"][row][basis].as_f64().unwrap());
                close(out[row].im, -0.5 * c["rows"][row][basis].as_f64().unwrap());
            }
        }
        close(matrix.rows[0][1] + matrix.rows[1][1], 0.0);
        if residual {
            assert!(1.0 / (matrix.rows[0][0] + matrix.rows[1][0]) <= 1.2 + 2e-15);
            assert_eq!([matrix.rows[0][2], matrix.rows[1][2]], [1.0, -1.0]);
        } else {
            for row in 0..2 {
                close(
                    matrix.rows[row][0].powi(2) + matrix.rows[row][1].powi(2),
                    matrix.channel_gains[row].powi(2),
                );
            }
            let covariance =
                matrix.rows[0][0] * matrix.rows[1][0] + matrix.rows[0][1] * matrix.rows[1][1];
            close(
                covariance / (matrix.channel_gains[0] * matrix.channel_gains[1]),
                matrix.effective_coherence,
            );
        }
    }
}
#[test]
fn complex_qmf_decorrelation_matches_decimal_traces_and_is_chunk_checkpoint_reset_invariant() {
    let m = manifest();
    let blob = std::fs::read(root().join("signals.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 120);
    for c in m["cases"].as_array().unwrap() {
        let bands = c["bands"].as_u64().unwrap() as usize;
        let slots = c["slots"].as_u64().unwrap() as usize;
        let input = samples(
            &blob,
            c["input_offset"].as_u64().unwrap() as usize,
            slots,
            bands,
        );
        let expected = samples(
            &blob,
            c["output_offset"].as_u64().unwrap() as usize,
            slots,
            bands,
        );
        let initial = Decorrelator::new(
            bands,
            c["seed"].as_u64().unwrap() as u8,
            c["configuration"].as_u64().unwrap() as u8,
            mode(c),
        )
        .unwrap();
        let mut a = initial.clone();
        let output = stream(&mut a, &input, &[32]);
        for (t, (got, want)) in output.iter().zip(&expected).enumerate() {
            for (bin, (got, want)) in got.iter().zip(want).enumerate() {
                assert!(
                    (got.re - want.re).abs() < 3e-11 && (got.im - want.im).abs() < 3e-11,
                    "{} slot {t} bin {bin}: {got:?} != {want:?}",
                    c["name"]
                );
            }
        }
        let mut partitioned = initial.clone();
        same_bits(
            &output,
            &stream(&mut partitioned, &input, &[1, 7, 13, 3, 32]),
        );
        assert_eq!(a, partitioned);
        let mut rewind = initial.clone();
        stream(&mut rewind, &input[..48], &[32]);
        let saved = rewind.clone();
        let tail = stream(&mut rewind, &input[48..], &[32]);
        let mut replay = saved;
        same_bits(&tail, &stream(&mut replay, &input[48..], &[7, 5, 19]));
        assert_eq!(rewind, replay);
        a.reset();
        assert_eq!(a, initial);
        same_bits(&output, &stream(&mut a, &input, &[31, 2, 1]));
    }
}
#[test]
fn nonfinite_later_slots_bad_geometry_and_overflow_never_advance_any_decorrelation_history() {
    let mut d = Decorrelator::new(64, 2, 0, Mode::Complex).unwrap();
    d.process(&vec![vec![Complex { re: 0.1, im: -0.2 }; 64]; 16])
        .unwrap();
    let saved = d.clone();
    for mut input in [
        vec![vec![Complex::default(); 63]],
        vec![vec![Complex::default(); 64]; 33],
        vec![vec![Complex::default(); 64]; 2],
    ] {
        if input.len() == 2 {
            input[1][63].im = f64::NAN;
        }
        assert!(d.process(&input).is_err());
        assert_eq!(d, saved);
    }
    let huge = vec![vec![Complex { re: 1e308, im: 0.0 }; 64]];
    assert!(d.process(&huge).is_err());
    assert_eq!(d, saved);
    assert!(d.process(&[]).unwrap().is_empty());
    assert_eq!(d, saved);
    for (b, seed, cfg) in [(0, 0, 0), (31, 0, 0), (65, 0, 0), (64, 10, 0), (64, 0, 3)] {
        assert!(Decorrelator::new(b, seed, cfg, Mode::Real).is_err());
    }
    assert!(Decorrelator::new(128, 0, 0, Mode::Real).is_ok());
    for (cld, rho) in [
        (f64::NAN, 0.0),
        (151.0, 0.0),
        (0.0, f64::INFINITY),
        (0.0, -1.0),
    ] {
        assert!(Ott::physical(cld, rho, false).is_err());
    }
    assert!(Ott::from_indices(16, 0, false).is_err());
    assert!(Ott::from_indices(0, 8, false).is_err());
    let o = Ott::from_indices(0, 0, false).unwrap();
    assert!(
        o.apply(
            Complex {
                re: f64::NAN,
                im: 0.0
            },
            Complex::default(),
            Complex::default()
        )
        .is_err()
    );
}
#[test]
fn every_filter_has_unit_response_strict_schur_margin_and_zero_row_delay() {
    for mode in [Mode::Real, Mode::Complex] {
        for seed in 0..10 {
            for cfg in 0..3 {
                for bin in 0..64 {
                    let d = design(64, seed, cfg, mode, bin).unwrap();
                    assert!(matches!(d.delay, 1 | 2 | 7));
                    assert!(matches!(d.denominator.len(), 4 | 7 | 16));
                    if seed >= 4 {
                        assert_eq!(d.denominator[0].re, 1.0);
                        assert!(
                            d.denominator[1..]
                                .iter()
                                .all(|x| x.re == 0.0 && x.im == 0.0)
                        );
                    }
                    let mut polynomial = d.denominator.clone();
                    while polynomial.len() > 1 {
                        let n = polynomial.len() - 1;
                        let k = polynomial[n];
                        let magnitude = k.re * k.re + k.im * k.im;
                        assert!(magnitude < 0.251, "reflection margin lost: {magnitude}");
                        polynomial = (0..n)
                            .map(|j| {
                                let reverse = polynomial[n - j];
                                let product = Complex {
                                    re: k.re * reverse.re + k.im * reverse.im,
                                    im: k.im * reverse.re - k.re * reverse.im,
                                };
                                Complex {
                                    re: (polynomial[j].re - product.re) / (1.0 - magnitude),
                                    im: (polynomial[j].im - product.im) / (1.0 - magnitude),
                                }
                            })
                            .collect();
                    }
                    for frequency in [0.0, 0.13, 0.7, 1.5, 2.9] {
                        let eval = |coefficients: &[Complex]| {
                            let mut re = 0.0;
                            let mut im = 0.0;
                            for (j, c) in coefficients.iter().enumerate() {
                                let (s, co) = (-frequency * j as f64).sin_cos();
                                re += c.re * co - c.im * s;
                                im += c.re * s + c.im * co;
                            }
                            re * re + im * im
                        };
                        let a = eval(&d.denominator);
                        let b = eval(&d.numerator);
                        assert!((a - b).abs() < 2e-12 * (1.0 + a));
                    }
                }
            }
        }
    }
}
