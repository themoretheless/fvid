//! Owned inverse MDCT kernel for the AAC synthesis work in progress.
//! This is not a packet decoder and is not yet wired into AAC playback.
//! Windowed normalization: 2/N; windowing and overlap-add belong to the caller.
//! Mathematical basis: Shao and Johnson, https://arxiv.org/abs/0708.4399, section VII.
use crate::{Result, invalid};
use std::f64::consts::PI;

#[derive(Clone)]
pub struct Imdct {
    coefficients: usize,
    recurrence: Vec<[f64; 3]>,
}
impl Imdct {
    /// AAC-LC long/short transforms; 960/120 support the alternate frame length.
    pub fn new(coefficients: usize) -> Result<Self> {
        if !matches!(coefficients, 120 | 128 | 960 | 1024) {
            return Err(invalid("unsupported AAC IMDCT length"));
        }
        let recurrence = (0..2 * coefficients)
            .map(|sample| {
                let phase =
                    PI / coefficients as f64 * (sample as f64 + 0.5 + coefficients as f64 / 2.0);
                [(phase * 0.5).cos(), (phase * 1.5).cos(), 2.0 * phase.cos()]
            })
            .collect();
        Ok(Self {
            coefficients,
            recurrence,
        })
    }
    /// Reuses the plan and caller's output. The cosine recurrence avoids
    /// trigonometric evaluation inside the coefficient loop. Complexity remains
    /// quadratic; fast-transform optimization and AAC window switching are pending.
    pub fn inverse(&self, spectrum: &[f32], output: &mut [f64]) -> Result<()> {
        if spectrum.len() != self.coefficients
            || output.len() != self.coefficients * 2
            || spectrum.iter().any(|x| !x.is_finite())
        {
            return Err(invalid("invalid AAC IMDCT input"));
        }
        for (sample, &[first, second, factor]) in output.iter_mut().zip(&self.recurrence) {
            let mut previous = first;
            let mut current = second;
            let mut sum = f64::from(spectrum[0]) * first;
            for &coefficient in &spectrum[1..] {
                sum += f64::from(coefficient) * current;
                (previous, current) = (current, factor * current - previous);
            }
            *sample = sum * (2.0 / self.coefficients as f64);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_aac_length_matches_direct_cosine_basis() {
        for n in [120, 128, 960, 1024] {
            let plan = Imdct::new(n).unwrap();
            let mut spectrum = vec![0.0; n];
            let mut output = vec![0.0; 2 * n];
            for bin in [0, n / 3, n - 1] {
                spectrum.fill(0.0);
                spectrum[bin] = 1.0;
                plan.inverse(&spectrum, &mut output).unwrap();
                for (sample, &actual) in output.iter().enumerate() {
                    let expected = 2.0 / n as f64
                        * (PI / n as f64
                            * (sample as f64 + 0.5 + n as f64 / 2.0)
                            * (bin as f64 + 0.5))
                            .cos();
                    assert!(
                        (actual - expected).abs() < 1e-10,
                        "n={n} bin={bin} sample={sample}"
                    );
                }
            }
        }
    }
    #[test]
    fn sine_window_overlap_cancels_aliasing() {
        let n = 128;
        let signal: Vec<_> = (0..5 * n)
            .map(|i| (i as f64 * 0.17).sin() * 0.4 + (i as f64 * 0.037).cos() * 0.2)
            .collect();
        let window: Vec<_> = (0..2 * n)
            .map(|i| (PI / (2 * n) as f64 * (i as f64 + 0.5)).sin())
            .collect();
        let plan = Imdct::new(n).unwrap();
        let mut reconstructed = vec![0.0; signal.len()];
        let mut spectrum = vec![0.0f32; n];
        let mut block = vec![0.0; 2 * n];
        for start in (0..=3 * n).step_by(n) {
            for (k, coefficient) in spectrum.iter_mut().enumerate() {
                *coefficient = (0..2 * n)
                    .map(|i| {
                        signal[start + i]
                            * window[i]
                            * (PI / n as f64 * (i as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5))
                                .cos()
                    })
                    .sum::<f64>() as f32;
            }
            plan.inverse(&spectrum, &mut block).unwrap();
            for i in 0..2 * n {
                reconstructed[start + i] += block[i] * window[i];
            }
        }
        for i in n..4 * n {
            assert!((signal[i] - reconstructed[i]).abs() < 1e-7, "sample={i}");
        }
    }
    #[test]
    fn invalid_input_does_not_change_output() {
        assert!(Imdct::new(usize::MAX).is_err());
        let plan = Imdct::new(128).unwrap();
        let mut output = [123.0; 256];
        assert!(plan.inverse(&[0.0; 127], &mut output).is_err());
        assert!(plan.inverse(&[f32::NAN; 128], &mut output).is_err());
        assert!(output.iter().all(|x| *x == 123.0));
    }
}
