//! Owned inverse MDCT via a chirp convolution and radix-2 FFT.
//! Windowed normalization is 2/N; windowing and overlap-add belong to the caller.
use super::{Result, invalid};
use std::{f64::consts::PI, sync::Arc};
type Complex = [f64; 2];
fn mul(a: Complex, b: Complex) -> Complex {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}
fn phase(angle: f64) -> Complex {
    let (sin, cos) = angle.sin_cos();
    [cos, sin]
}
struct Tables {
    coefficients: usize,
    chirp: Vec<Complex>,
    kernel: Vec<Complex>,
    roots: Vec<Complex>,
    reverse: Vec<usize>,
}
#[derive(Clone)]
pub struct Imdct {
    tables: Arc<Tables>,
}
impl Imdct {
    pub(crate) fn visit_retained(
        &self,
        footprint: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        if footprint.shared(&self.tables)? {
            footprint.vector(&self.tables.chirp)?;
            footprint.vector(&self.tables.kernel)?;
            footprint.vector(&self.tables.roots)?;
            footprint.vector(&self.tables.reverse)?;
        }
        Ok(())
    }

    /// AAC-LC long/short transforms and AAC-SSR quarter-band transforms.
    pub fn new(coefficients: usize) -> Result<Self> {
        if !matches!(coefficients, 32 | 120 | 128 | 256 | 960 | 1024) {
            return Err(invalid("unsupported AAC IMDCT length"));
        }
        let size = (2 * coefficients - 1).next_power_of_two();
        let roots = (0..size / 2)
            .map(|k| phase(-2.0 * PI * k as f64 / size as f64))
            .collect();
        let reverse = (0..size)
            .map(|k| k.reverse_bits() >> (usize::BITS - size.trailing_zeros()))
            .collect();
        let chirp = (0..coefficients)
            .map(|k| phase(PI / (2 * coefficients) as f64 * (k as f64 + 0.5).powi(2)))
            .collect();
        let mut tables = Tables {
            coefficients,
            chirp,
            // FFT initialization only reads roots/reverse, so no placeholder
            // kernel allocation is needed before installing the transformed data.
            kernel: Vec::new(),
            roots,
            reverse,
        };
        let mut kernel = vec![[0.0; 2]; size];
        for k in 0..coefficients {
            kernel[k] = phase(-PI / (2 * coefficients) as f64 * (k * k) as f64);
            if k != 0 {
                kernel[size - k] = kernel[k];
            }
        }
        tables.fft(&mut kernel, false);
        tables.kernel = kernel;
        Ok(Self {
            tables: Arc::new(tables),
        })
    }
    /// Number of complex scratch cells needed by `inverse_with_scratch`.
    pub fn scratch_len(&self) -> usize {
        self.tables.kernel.len()
    }
    /// Unnormalized forward MDCT, the cosine transpose of the 2/N inverse.
    /// O(N log N), no allocation. Invalid/overflowing input leaves output intact.
    pub fn forward_with_scratch(&self, input: &[f64], output: &mut [f64], scratch: &mut [Complex]) -> Result<()> {
        let t = &self.tables;
        let n = t.coefficients;
        if input.len() != 2*n || output.len() != n || scratch.len() != self.scratch_len() || input.iter().any(|x| !x.is_finite()) {
            return Err(invalid("invalid AAC MDCT input"));
        }
        scratch.fill([0.0; 2]);
        for (i, value) in input.iter().enumerate() {
            let j = i + n/2;
            let (index, sign) = if j < n { (j, 1.0) } else if j < 2*n { (2*n-1-j, -1.0) } else { (j-2*n, -1.0) };
            scratch[index][0] += sign * value;
        }
        for k in 0..n { scratch[k] = mul(scratch[k], t.chirp[k]); }
        t.fft(scratch, false);
        for (value, kernel) in scratch.iter_mut().zip(&t.kernel) { *value = mul(*value, *kernel); }
        t.fft(scratch, true);
        for k in 0..n { scratch[k] = mul(scratch[k], t.chirp[k]); }
        if scratch[..n].iter().any(|x| !x[0].is_finite()) { return Err(invalid("AAC MDCT output overflow")); }
        for (out, value) in output.iter_mut().zip(&scratch[..n]) { *out = value[0]; }
        Ok(())
    }
    /// Convenience entry point. Synthesis uses caller-owned scratch instead.
    pub fn inverse(&self, spectrum: &[f32], output: &mut [f64]) -> Result<()> {
        self.inverse_with_scratch(spectrum, output, &mut vec![[0.0; 2]; self.scratch_len()])
    }
    /// O(N log N), no allocation. Invalid input leaves output unchanged.
    pub fn inverse_with_scratch(
        &self,
        spectrum: &[f32],
        output: &mut [f64],
        scratch: &mut [Complex],
    ) -> Result<()> {
        let t = &self.tables;
        let n = t.coefficients;
        if spectrum.len() != n
            || output.len() != 2 * n
            || scratch.len() != self.scratch_len()
            || spectrum.iter().any(|x| !x.is_finite())
        {
            return Err(invalid("invalid AAC IMDCT input"));
        }
        scratch.fill([0.0; 2]);
        // (k+.5)(m+.5) = ((k+.5)^2+(m+.5)^2-(k-m)^2)/2.
        // Thus DCT-IV is the real part of this chirp convolution.
        for k in 0..n {
            scratch[k] = mul([f64::from(spectrum[k]), 0.0], t.chirp[k]);
        }
        t.fft(scratch, false);
        for (value, kernel) in scratch.iter_mut().zip(&t.kernel) {
            *value = mul(*value, *kernel);
        }
        t.fft(scratch, true);
        for k in 0..n {
            scratch[k] = mul(scratch[k], t.chirp[k]);
        }
        // Extend DCT-IV by its odd/even symmetries to the shifted 2N IMDCT.
        for (sample, value) in output.iter_mut().enumerate() {
            let j = sample + n / 2;
            let (index, sign) = if j < n {
                (j, 1.0)
            } else if j < 2 * n {
                (2 * n - 1 - j, -1.0)
            } else {
                (j - 2 * n, -1.0)
            };
            *value = scratch[index][0] * sign * (2.0 / n as f64);
        }
        Ok(())
    }
}
impl Tables {
    fn fft(&self, data: &mut [Complex], inverse: bool) {
        let n = data.len();
        for (i, &j) in self.reverse.iter().enumerate() {
            if i < j {
                data.swap(i, j);
            }
        }
        let mut width = 2;
        while width <= n {
            let half = width / 2;
            for block in data.chunks_exact_mut(width) {
                for j in 0..half {
                    let mut root = self.roots[j * (n / width)];
                    if inverse {
                        root[1] = -root[1];
                    }
                    let a = block[j];
                    let b = mul(block[j + half], root);
                    block[j] = [a[0] + b[0], a[1] + b[1]];
                    block[j + half] = [a[0] - b[0], a[1] - b[1]];
                }
            }
            width *= 2;
        }
        if inverse {
            for value in data {
                value[0] /= n as f64;
                value[1] /= n as f64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn forward_mdct_all_lengths_match_dense_direct_cosine_sum() {
        use super::*;
        for n in [32, 120, 128, 256, 960, 1024] {
            let transform = Imdct::new(n).unwrap();
            let input: Vec<_> = (0..2*n).map(|i| ((i*17)%31) as f64 - 15.25).collect();
            let mut output = vec![0.0; n];
            let mut scratch = vec![[0.0;2]; transform.scratch_len()];
            transform.forward_with_scratch(&input, &mut output, &mut scratch).unwrap();
            for (k, value) in output.iter().enumerate() {
                let direct: f64 = input.iter().enumerate().map(|(i,x)| x * (PI/n as f64 * (i as f64+0.5+n as f64/2.0) * (k as f64+0.5)).cos()).sum();
                assert!((value-direct).abs() < 2e-7 + direct.abs()*2e-11, "n={n} k={k}: {value} vs {direct}");
            }
            output.fill(19.0);
            assert!(transform.forward_with_scratch(&input, &mut output, &mut scratch[..1]).is_err());
            assert!(output.iter().all(|x|*x==19.0));
        }
    }
    use super::*;
    #[test]
    fn every_aac_length_matches_direct_cosine_basis() {
        for n in [32, 120, 128, 256, 960, 1024] {
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
    fn dense_spectra_match_direct_basis_with_reused_scratch() {
        for n in [32, 120, 128, 256, 960, 1024] {
            let plan = Imdct::new(n).unwrap();
            let mut scratch = vec![[999.0; 2]; plan.scratch_len()];
            let mut output = vec![0.0; 2 * n];
            let spectrum: Vec<f32> = (0..n)
                .map(|i| ((i as f64 * 0.137).sin() + (i as f64 * 0.021).cos()) as f32)
                .collect();
            plan.inverse_with_scratch(&spectrum, &mut output, &mut scratch)
                .unwrap();
            for (sample, &actual) in output.iter().enumerate() {
                let expected = spectrum
                    .iter()
                    .enumerate()
                    .map(|(k, &x)| {
                        f64::from(x)
                            * (PI / n as f64
                                * (sample as f64 + 0.5 + n as f64 / 2.0)
                                * (k as f64 + 0.5))
                                .cos()
                    })
                    .sum::<f64>()
                    * 2.0
                    / n as f64;
                assert!(
                    (actual - expected).abs() < 1e-10,
                    "n={n} sample={sample}: {actual} vs {expected}"
                );
            }
            plan.inverse_with_scratch(&vec![0.0; n], &mut output, &mut scratch)
                .unwrap();
            assert!(output.iter().all(|&x| x == 0.0));
            output.fill(123.0);
            assert!(
                plan.inverse_with_scratch(&spectrum, &mut output, &mut scratch[..1])
                    .is_err()
            );
            assert!(output.iter().all(|&x| x == 123.0));
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
