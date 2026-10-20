//! FVid-owned Vorbis inverse MDCT using a radix-two complex FFT.
use crate::{Result, invalid};

/// Unnormalized Vorbis synthesis transform. For output length N and N/2
/// coefficients, computes sum X[k] cos(2*pi/N*(t+1/2+N/4)*(k+1/2)).
/// Caller supplies 2*N complex scratch values; no frame allocation is hidden.
pub fn inverse(coefficients: &[f32], output: &mut [f32], scratch: &mut [(f64, f64)]) -> Result<()> {
    let n = output.len();
    if !n.is_power_of_two()
        || !(64..=8192).contains(&n)
        || coefficients.len() != n / 2
        || scratch.len() < 2 * n
    {
        return Err(invalid("invalid Vorbis IMDCT geometry or scratch"));
    }
    let scratch = &mut scratch[..2 * n];
    scratch.fill((0., 0.));
    for (to, &value) in scratch.iter_mut().zip(coefficients) {
        to.0 = f64::from(value);
    }
    // FFT of zero-padded coefficients. Odd frequency samples and a half-bin
    // phase rotation evaluate the MDCT cosine basis without a quadratic matrix.
    let length = scratch.len();
    let mut reversed = 0usize;
    for i in 1..length {
        let mut bit = length >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;
        if i < reversed {
            scratch.swap(i, reversed);
        }
    }
    let mut stage = 2;
    while stage <= length {
        let phase = -2. * std::f64::consts::PI / stage as f64;
        let (sine, cosine) = phase.sin_cos();
        for block in scratch.chunks_exact_mut(stage) {
            let (mut wr, mut wi) = (1., 0.);
            for i in 0..stage / 2 {
                let (ar, ai) = block[i];
                let (br, bi) = block[i + stage / 2];
                let (tr, ti) = (wr * br - wi * bi, wr * bi + wi * br);
                block[i] = (ar + tr, ai + ti);
                block[i + stage / 2] = (ar - tr, ai - ti);
                (wr, wi) = (wr * cosine - wi * sine, wr * sine + wi * cosine);
            }
        }
        stage *= 2;
    }
    for (t, sample) in output.iter_mut().enumerate() {
        let frequency = 2 * t + 1 + n / 2;
        let (real, imaginary) = scratch[frequency % length];
        let phase = -std::f64::consts::PI * frequency as f64 / length as f64;
        let (sine, cosine) = phase.sin_cos();
        *sample = (real * cosine - imaginary * sine) as f32;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn direct(coefficients: &[f32], t: usize) -> f32 {
        let n = 2 * coefficients.len();
        coefficients
            .iter()
            .enumerate()
            .map(|(k, &x)| {
                f64::from(x)
                    * (2. * std::f64::consts::PI / n as f64
                        * (t as f64 + 0.5 + n as f64 / 4.)
                        * (k as f64 + 0.5))
                        .cos()
            })
            .sum::<f64>() as f32
    }
    #[test]
    fn fft_imdct_matches_direct_cosine_synthesis_at_every_legal_block_size() {
        for exponent in 6..=13 {
            let n = 1usize << exponent;
            let coefficients: Vec<_> = (0..n / 2)
                .map(|k| (((k * 37 + 11) % 101) as f32 - 50.) / 101.)
                .collect();
            let mut output = vec![0.; n];
            let mut scratch = vec![(0., 0.); 2 * n];
            inverse(&coefficients, &mut output, &mut scratch).unwrap();
            let indices: Vec<_> = if n <= 256 {
                (0..n).collect()
            } else {
                vec![
                    0,
                    1,
                    n / 4 - 1,
                    n / 4,
                    n / 2 - 1,
                    n / 2,
                    3 * n / 4,
                    n - 2,
                    n - 1,
                ]
            };
            for t in indices {
                let expected = direct(&coefficients, t);
                assert!(
                    (output[t] - expected).abs() < 0.00002,
                    "n={n} t={t}: {} != {expected}",
                    output[t]
                );
            }
        }
    }
    #[test]
    fn impulse_basis_zero_input_and_invalid_geometry_preserve_contract() {
        let mut coefficients = [0.; 32];
        coefficients[17] = 1.;
        let mut output = [0.; 64];
        let mut scratch = [(0., 0.); 128];
        inverse(&coefficients, &mut output, &mut scratch).unwrap();
        for (t, &value) in output.iter().enumerate() {
            assert!((value - direct(&coefficients, t)).abs() < 0.000001);
        }
        coefficients.fill(0.);
        inverse(&coefficients, &mut output, &mut scratch).unwrap();
        assert_eq!(output, [0.; 64]);
        output.fill(42.);
        assert!(inverse(&coefficients, &mut output, &mut scratch[..127]).is_err());
        assert_eq!(output, [42.; 64]);
    }
}
