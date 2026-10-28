//! Owned not-a-knot cubic interpolation of the normative 512-point ELD window.
//! Each 128-value segment is independent; discontinuities and zero regions
//! must not leak across segment boundaries. No codec implementation is reused.
use super::{Result, aac_eld_windows, invalid};

fn curvature(y: &[f64]) -> [f64; 128] {
    // Half-second derivatives. Not-a-knot endpoint equations eliminate the
    // outer unknowns; solve the remaining uniform tridiagonal system exactly
    // in f64 rather than using pre-rounded elimination constants.
    let mut c = [0.0; 128];
    let rhs = |k: usize| 3.0 * (y[k + 1] - 2.0 * y[k] + y[k - 1]);
    c[1] = rhs(1) / 6.0;
    c[126] = rhs(126) / 6.0;
    let mut diagonal = [4.0; 124];
    let mut values = [0.0; 124];
    for (i, value) in values.iter_mut().enumerate() {
        *value = rhs(i + 2);
    }
    values[0] -= c[1];
    values[123] -= c[126];
    for i in 1..124 {
        let multiplier = 1.0 / diagonal[i - 1];
        diagonal[i] -= multiplier;
        values[i] -= multiplier * values[i - 1];
    }
    c[125] = values[123] / diagonal[123];
    for i in (0..123).rev() {
        c[i + 2] = (values[i] - c[i + 3]) / diagonal[i];
    }
    c[0] = 2.0 * c[1] - c[2];
    c[127] = 2.0 * c[126] - c[125];
    c
}
fn evaluate(y: &[f64], c: &[f64; 128], x: f64) -> f64 {
    let k = (x.floor() as usize).min(126);
    let t = x - k as f64;
    let slope = y[k + 1] - y[k] - (c[k + 1] + 2.0 * c[k]) / 3.0;
    y[k] + t * (slope + t * (c[k] + t * (c[k + 1] - c[k]) / 3.0))
}
pub(crate) fn window(samples: usize) -> Result<Vec<f64>> {
    if !matches!(samples, 60 | 64 | 120 | 128 | 240 | 256) {
        return Err(invalid("unsupported AAC ELD downscaled window"));
    }
    let per_segment = samples / 4;
    let mut output = Vec::new();
    output
        .try_reserve_exact(4 * samples)
        .map_err(|_| invalid("AAC ELD window allocation failed"))?;
    for y in aac_eld_windows::WINDOW_512.chunks_exact(128) {
        let c = curvature(y);
        for i in 0..per_segment {
            let x = (i as f64 + 0.5) * 128.0 / per_segment as f64 - 0.5;
            output.push(evaluate(y, &c, x));
        }
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cubic_segments_reproduce_polynomials_without_crossing_boundaries() {
        for power in 0..=3 {
            let y: Vec<_> = (0..128).map(|i| (i as f64 / 128.0).powi(power)).collect();
            let c = curvature(&y);
            for length in [15, 16, 30, 32, 60, 64] {
                for i in 0..length {
                    let x = (i as f64 + 0.5) * 128.0 / length as f64 - 0.5;
                    assert!((evaluate(&y, &c, x) - (x / 128.0).powi(power)).abs() < 2e-14);
                }
            }
        }
        for n in [60, 64, 120, 128, 240, 256] {
            let w = window(n).unwrap();
            assert_eq!(w.len(), 4 * n);
            assert!(w[..n / 4].iter().all(|v| *v == 0.0));
            assert!(w.iter().all(|v| v.is_finite()));
        }
    }
    #[test]
    fn adapted_windows_match_independent_dense_solve() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/aac-eld-short");
        for n in [60, 64, 120, 128, 240, 256] {
            let path = if n < 120 {
                root.join("../aac-eld-sbr-eighth")
            } else {
                root.clone()
            };
            let bytes = std::fs::read(path.join(format!("window-{n}.f64le"))).unwrap();
            let actual = window(n).unwrap();
            assert_eq!(bytes.len(), actual.len() * 8);
            for (a, b) in actual.iter().zip(bytes.chunks_exact(8)) {
                assert!((*a - f64::from_le_bytes(b.try_into().unwrap())).abs() < 2e-12);
            }
        }
    }
}
