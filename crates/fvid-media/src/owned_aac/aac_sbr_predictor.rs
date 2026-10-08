//! Complex covariance prediction for legacy SBR HF generation (6.18.6.2).
use super::{Result, aac_sbr_qmf::Complex, invalid};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Predictor {
    pub first: Complex,
    pub second: Complex,
}
fn multiply(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}
fn conjugate(a: Complex) -> Complex {
    Complex {
        re: a.re,
        im: -a.im,
    }
}
fn magnitude_squared(a: Complex) -> f64 {
    a.re * a.re + a.im * a.im
}
/// `samples` is one source QMF band, oldest first: two past samples followed
/// by `2*time_slots+6` samples beginning at tHFAdj. The enclosing frame buffer
/// selects this window; prediction itself has no hidden frame history.
/// A common normalization preserves the covariance ratios and avoids overflow.
pub fn predict(samples: &[Complex], time_slots: u8) -> Result<Predictor> {
    if !matches!(time_slots, 15 | 16) || samples.len() != 2 * usize::from(time_slots) + 8 {
        return Err(invalid("invalid SBR predictor window"));
    }
    if samples
        .iter()
        .any(|x| !x.re.is_finite() || !x.im.is_finite())
    {
        return Err(invalid("non-finite SBR predictor samples"));
    }
    let scale = samples
        .iter()
        .fold(0.0f64, |s, x| s.max(x.re.abs()).max(x.im.abs()));
    if scale == 0.0 {
        return Ok(Predictor::default());
    }
    let mut cross = [Complex::default(); 3]; // phi(0,1), phi(0,2), phi(1,2)
    let mut energy = [0.0; 2]; // phi(1,1), phi(2,2)
    for window in samples.windows(3) {
        let normalized = |i: usize| Complex {
            re: window[i].re / scale,
            im: window[i].im / scale,
        };
        let x = normalized(2);
        let y = normalized(1);
        let z = normalized(0);
        for (sum, value) in cross.iter_mut().zip([
            multiply(x, conjugate(y)),
            multiply(x, conjugate(z)),
            multiply(y, conjugate(z)),
        ]) {
            sum.re += value.re;
            sum.im += value.im;
        }
        energy[0] += magnitude_squared(y);
        energy[1] += magnitude_squared(z);
    }
    let determinant = energy[0] * energy[1] - magnitude_squared(cross[2]) / (1.0 + 1e-6);
    let second = if determinant == 0.0 {
        Complex::default()
    } else {
        let product = multiply(cross[0], cross[2]);
        Complex {
            re: (product.re - cross[1].re * energy[0]) / determinant,
            im: (product.im - cross[1].im * energy[0]) / determinant,
        }
    };
    let first = if energy[0] == 0.0 {
        Complex::default()
    } else {
        let product = multiply(second, conjugate(cross[2]));
        Complex {
            re: -(cross[0].re + product.re) / energy[0],
            im: -(cross[0].im + product.im) / energy[0],
        }
    };
    if ![first.re, first.im, second.re, second.im]
        .iter()
        .all(|x| x.is_finite())
    {
        return Err(invalid("non-finite SBR predictor coefficients"));
    }
    if magnitude_squared(first) >= 16.0 || magnitude_squared(second) >= 16.0 {
        Ok(Predictor::default())
    } else {
        Ok(Predictor { first, second })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_decimal_covariance_reference_and_scale_phase_invariance() {
        let data = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-predictor-decimal.f64le"
        );
        let mut offset = 0;
        let read = |offset: &mut usize| {
            let x = f64::from_le_bytes(data[*offset..*offset + 8].try_into().unwrap());
            *offset += 8;
            x
        };
        for slots in [15, 16] {
            for _case in 0..6 {
                let samples: Vec<_> = (0..2 * slots + 8)
                    .map(|_| Complex {
                        re: read(&mut offset),
                        im: read(&mut offset),
                    })
                    .collect();
                let expected = Predictor {
                    first: Complex {
                        re: read(&mut offset),
                        im: read(&mut offset),
                    },
                    second: Complex {
                        re: read(&mut offset),
                        im: read(&mut offset),
                    },
                };
                let actual = predict(&samples, slots).unwrap();
                for (a, b) in [
                    actual.first.re,
                    actual.first.im,
                    actual.second.re,
                    actual.second.im,
                ]
                .into_iter()
                .zip([
                    expected.first.re,
                    expected.first.im,
                    expected.second.re,
                    expected.second.im,
                ]) {
                    assert!(
                        (a - b).abs() < 2e-9,
                        "slots {slots} case {_case}: {a} != {b}"
                    );
                }
                for scale in [2f64.powi(-500), 2f64.powi(500)] {
                    let transformed: Vec<_> = samples
                        .iter()
                        .map(|x| Complex {
                            re: -x.im * scale,
                            im: x.re * scale,
                        })
                        .collect();
                    assert_eq!(predict(&transformed, slots).unwrap(), actual);
                }
            }
        }
        assert_eq!(offset, data.len());
    }
    #[test]
    fn degenerate_unstable_and_invalid_inputs_have_explicit_results() {
        let zero = vec![Complex::default(); 40];
        assert_eq!(predict(&zero, 16).unwrap(), Predictor::default());
        let mut current_only = zero.clone();
        current_only[39].re = 1.0;
        assert_eq!(predict(&current_only, 16).unwrap(), Predictor::default());
        let unstable: Vec<_> = (0..40)
            .map(|i| Complex {
                re: 5f64.powi(i),
                im: 0.0,
            })
            .collect();
        assert_eq!(predict(&unstable, 16).unwrap(), Predictor::default());
        let extremes = vec![
            Complex {
                re: f64::MAX,
                im: f64::MAX
            };
            40
        ];
        let p = predict(&extremes, 16).unwrap();
        assert!((p.first.re + 1.0).abs() < 1e-9);
        for slots in [0, 14, 17, 255] {
            assert!(predict(&zero, slots).is_err());
        }
        assert!(predict(&zero[..39], 16).is_err());
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut samples = zero.clone();
            samples[39].im = bad;
            assert!(predict(&samples, 16).is_err());
        }
    }
}
