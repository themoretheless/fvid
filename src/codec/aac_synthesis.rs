//! Owned AAC long/short sine and KBD synthesis. Packet decoding is separate.
use super::aac_imdct::Imdct;
use crate::{Result, invalid};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowSequence {
    OnlyLong,
    LongStart,
    EightShort,
    LongStop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowShape {
    Sine,
    Kbd,
}

// KBD half-window: square root of the normalized cumulative Kaiser window.
// Include both Kaiser endpoints in the denominator for power complementarity.
fn kbd_window(n: usize, alpha: f64) -> Vec<f64> {
    fn i0(x: f64) -> f64 {
        let mut sum = 1.0;
        let mut term = 1.0;
        for k in 1..100 {
            term *= x * x / (4.0 * (k * k) as f64);
            sum += term;
            if term < sum * f64::EPSILON {
                break;
            }
        }
        sum
    }
    let mut cumulative = Vec::with_capacity(n);
    let mut total = 0.0;
    for i in 0..=n {
        let x = 2.0 * i as f64 / n as f64 - 1.0;
        total += i0(PI * alpha * (1.0 - x * x).max(0.0).sqrt());
        if i < n {
            cumulative.push(total);
        }
    }
    for value in &mut cumulative {
        *value = (*value / total).sqrt();
    }
    let mut full = cumulative.clone();
    full.extend(cumulative.into_iter().rev());
    full
}

pub struct LongSineSynthesis {
    kbd_long: Vec<f64>,
    kbd_short: Vec<f64>,
    previous_shape: WindowShape,
    short_transform: Imdct,
    short_window: Vec<f64>,
    short_scratch: Vec<f64>,
    transform: Imdct,
    window: Vec<f64>,
    scratch: Vec<f64>,
    overlap: Vec<f64>,
}
impl LongSineSynthesis {
    pub fn new(frame_samples: usize) -> Result<Self> {
        if !matches!(frame_samples, 960 | 1024) {
            return Err(invalid("AAC long synthesis requires 960 or 1024 samples"));
        }
        Ok(Self {
            kbd_long: kbd_window(frame_samples, 4.0),
            kbd_short: kbd_window(frame_samples / 8, 6.0),
            previous_shape: WindowShape::Sine,
            transform: Imdct::new(frame_samples)?,
            short_transform: Imdct::new(frame_samples / 8)?,
            short_window: (0..frame_samples / 4)
                .map(|i| (PI / (frame_samples / 4) as f64 * (i as f64 + 0.5)).sin())
                .collect(),
            short_scratch: vec![0.0; frame_samples / 4],
            window: (0..2 * frame_samples)
                .map(|i| (PI / (2 * frame_samples) as f64 * (i as f64 + 0.5)).sin())
                .collect(),
            scratch: vec![0.0; 2 * frame_samples],
            overlap: vec![0.0; frame_samples],
        })
    }
    /// Consume one long-window spectrum and emit one PCM block. There are no
    /// per-frame allocations. Invalid input leaves output and overlap unchanged.
    /// Scaling follows Imdct (2/N); packet spectral normalization is separate.
    pub fn synthesize(&mut self, spectrum: &[f32], pcm: &mut [f64]) -> Result<()> {
        self.synthesize_sequence(WindowSequence::OnlyLong, spectrum, pcm)
    }
    /// Short spectra are eight consecutive, deinterleaved windows. Grouped AAC
    /// spectral data must be deinterleaved by the packet decoder first.
    pub fn synthesize_sequence(
        &mut self,
        sequence: WindowSequence,
        spectrum: &[f32],
        pcm: &mut [f64],
    ) -> Result<()> {
        self.synthesize_shaped(sequence, WindowShape::Sine, spectrum, pcm)
    }
    /// The previous frame's shape applies only to the leading half-window
    /// (the first short block for EightShort). A reset restores sine history.
    pub fn synthesize_shaped(
        &mut self,
        sequence: WindowSequence,
        shape: WindowShape,
        spectrum: &[f32],
        pcm: &mut [f64],
    ) -> Result<()> {
        let n = self.overlap.len();
        let short = n / 8;
        let offset = (n - short) / 2;
        if pcm.len() != n || spectrum.len() != n || spectrum.iter().any(|x| !x.is_finite()) {
            return Err(invalid("AAC synthesis invalid spectrum or output size"));
        }
        let long = if shape == WindowShape::Sine {
            &self.window
        } else {
            &self.kbd_long
        };
        let short_window = if shape == WindowShape::Sine {
            &self.short_window
        } else {
            &self.kbd_short
        };
        let previous_long = if self.previous_shape == WindowShape::Sine {
            &self.window
        } else {
            &self.kbd_long
        };
        let previous_short = if self.previous_shape == WindowShape::Sine {
            &self.short_window
        } else {
            &self.kbd_short
        };
        if sequence == WindowSequence::EightShort {
            self.scratch.fill(0.0);
            for (block, coefficients) in spectrum.chunks_exact(short).enumerate() {
                self.short_transform
                    .inverse(coefficients, &mut self.short_scratch)?;
                for i in 0..2 * short {
                    self.scratch[offset + block * short + i] += self.short_scratch[i]
                        * if block == 0 && i < short {
                            previous_short[i]
                        } else {
                            short_window[i]
                        };
                }
            }
        } else {
            self.transform.inverse(spectrum, &mut self.scratch)?;
            for i in 0..2 * n {
                let weight = match sequence {
                    WindowSequence::LongStart if i >= n => {
                        let j = i - n;
                        if j < offset {
                            1.0
                        } else if j < offset + short {
                            short_window[short + j - offset]
                        } else {
                            0.0
                        }
                    }
                    WindowSequence::LongStop if i < n => {
                        if i < offset {
                            0.0
                        } else if i < offset + short {
                            previous_short[i - offset]
                        } else {
                            1.0
                        }
                    }
                    _ => {
                        if i < n {
                            previous_long[i]
                        } else {
                            long[i]
                        }
                    }
                };
                self.scratch[i] *= weight;
            }
        }
        for i in 0..n {
            pcm[i] = self.overlap[i] + self.scratch[i];
            self.overlap[i] = self.scratch[n + i];
        }
        self.previous_shape = shape;
        Ok(())
    }
    /// Discard the previous frame's contribution after a seek/reset.
    pub fn reset(&mut self) {
        self.overlap.fill(0.0);
        self.previous_shape = WindowShape::Sine;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_blocks_reconstruct_an_independently_analysed_signal() {
        for n in [960, 1024] {
            let source: Vec<_> = (0..4 * n)
                .map(|i| (i as f64 * 0.017).sin() * 0.5 + (i as f64 * 0.093).cos() * 0.1)
                .collect();
            let mut synth = LongSineSynthesis::new(n).unwrap();
            let mut spectrum = vec![0.0; n];
            let mut output = vec![0.0; n];
            for start in (0..=2 * n).step_by(n) {
                for (k, coefficient) in spectrum.iter_mut().enumerate() {
                    *coefficient = (0..2 * n)
                        .map(|i| {
                            let window = (PI / (2 * n) as f64 * (i as f64 + 0.5)).sin();
                            source[start + i]
                                * window
                                * (PI / n as f64
                                    * (i as f64 + 0.5 + n as f64 / 2.0)
                                    * (k as f64 + 0.5))
                                    .cos()
                        })
                        .sum::<f64>() as f32;
                }
                synth.synthesize(&spectrum, &mut output).unwrap();
                if start != 0 {
                    for i in 0..n {
                        assert!(
                            (output[i] - source[start + i]).abs() < 1e-7,
                            "n={n}, sample={}",
                            start + i
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn transitions_reconstruct_signal_with_eight_short_windows() {
        use WindowSequence::*;
        for pattern in 0..3 {
            for n in [960, 1024] {
                let sequences = [
                    OnlyLong, LongStart, EightShort, EightShort, LongStop, OnlyLong,
                ];
                let source: Vec<_> = (0..(sequences.len() + 1) * n)
                    .map(|i| (i as f64 * 0.137).sin() * 0.3 + (i as f64 * 0.031).cos() * 0.2)
                    .collect();
                let mut synthesis = LongSineSynthesis::new(n).unwrap();
                let mut pcm = vec![0.0; n];
                let short = n / 8;
                let offset = (n - short) / 2;
                let mut previous_shape = WindowShape::Sine;
                for (frame, sequence) in sequences.into_iter().enumerate() {
                    let shape = if pattern == 0 || pattern == 2 && frame % 2 == 0 {
                        WindowShape::Sine
                    } else {
                        WindowShape::Kbd
                    };
                    let mut spectrum = Vec::with_capacity(n);
                    let blocks = if sequence == EightShort { 8 } else { 1 };
                    let size = if sequence == EightShort { short } else { n };
                    for block in 0..blocks {
                        let origin = frame * n
                            + if blocks == 8 {
                                offset + block * short
                            } else {
                                0
                            };
                        let mut windowed = vec![0.0; 2 * size];
                        for i in 0..2 * size {
                            // Independently construct the analysis window by joining
                            // rising/falling half-sines with constant/zero segments.
                            let (position, length) = match sequence {
                                LongStart if i >= n => {
                                    ((i - n) as isize - offset as isize + short as isize, short)
                                }
                                LongStop if i < n => (i as isize - offset as isize, short),
                                _ => (i as isize, size),
                            };
                            let weight = if position < 0 {
                                0.0
                            } else if position >= 2 * length as isize {
                                0.0
                            } else {
                                let half_shape = if block == 0 && i < size {
                                    previous_shape
                                } else {
                                    shape
                                };
                                if half_shape == WindowShape::Sine {
                                    (PI * (position as f64 + 0.5) / (2 * length) as f64).sin()
                                } else {
                                    kbd_window(length, if length == n { 4.0 } else { 6.0 })
                                        [position as usize]
                                }
                            };
                            let weight = if sequence == LongStart && i >= n && i < n + offset
                                || sequence == LongStop && i >= offset + short && i < n
                            {
                                1.0
                            } else {
                                weight
                            };
                            windowed[i] = source[origin + i] * weight;
                        }
                        for k in 0..size {
                            spectrum.push(
                                windowed
                                    .iter()
                                    .enumerate()
                                    .map(|(i, value)| {
                                        value
                                            * (PI / size as f64
                                                * (i as f64 + 0.5 + size as f64 / 2.0)
                                                * (k as f64 + 0.5))
                                                .cos()
                                    })
                                    .sum::<f64>() as f32,
                            );
                        }
                    }
                    synthesis
                        .synthesize_shaped(sequence, shape, &spectrum, &mut pcm)
                        .unwrap();
                    previous_shape = shape;
                    if frame > 0 {
                        for i in 0..n {
                            assert!(
                                (pcm[i] - source[frame * n + i]).abs() < 1e-7,
                                "n={n} frame={frame} sample={i}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn kbd_windows_are_symmetric_and_power_complementary() {
        for (n, alpha) in [(120, 6.0), (128, 6.0), (960, 4.0), (1024, 4.0)] {
            let window = kbd_window(n, alpha);
            for i in 0..n {
                assert_eq!(window[i], window[2 * n - 1 - i]);
                assert!((window[i].powi(2) + window[n + i].powi(2) - 1.0).abs() < 3e-15);
            }
            // Independent integral representation of I0, using midpoint
            // quadrature rather than the production power series.
            let weights: Vec<f64> = (0..=n)
                .map(|i| {
                    let x = 2.0 * i as f64 / n as f64 - 1.0;
                    let z = PI * alpha * (1.0 - x * x).max(0.0).sqrt();
                    (0..256)
                        .map(|j| (z * (PI * (j as f64 + 0.5) / 256.0).cos()).exp())
                        .sum::<f64>()
                        / 256.0
                })
                .collect();
            let sum: f64 = weights.iter().sum();
            let mut prefix = 0.0;
            for i in 0..n {
                prefix += weights[i];
                assert!((window[i] - (prefix / sum).sqrt()).abs() < 2e-14);
            }
        }
    }
    #[test]
    fn invalid_frame_does_not_change_window_history() {
        let mut synth = LongSineSynthesis::new(960).unwrap();
        let mut output = vec![0.0; 960];
        synth
            .synthesize_shaped(
                WindowSequence::OnlyLong,
                WindowShape::Kbd,
                &[0.0; 960],
                &mut output,
            )
            .unwrap();
        output.fill(17.0);
        assert!(
            synth
                .synthesize_shaped(
                    WindowSequence::EightShort,
                    WindowShape::Sine,
                    &[f32::NAN; 960],
                    &mut output
                )
                .is_err()
        );
        assert_eq!(synth.previous_shape, WindowShape::Kbd);
        assert!(output.iter().all(|x| *x == 17.0));
        synth.reset();
        assert_eq!(synth.previous_shape, WindowShape::Sine);
    }
    #[test]
    fn failure_is_transactional_and_reset_discards_overlap() {
        let n = 1024;
        let mut synth = LongSineSynthesis::new(n).unwrap();
        let mut reference = LongSineSynthesis::new(n).unwrap();
        let mut input = vec![0.0; n];
        input[17] = 3.0;
        let mut output = vec![0.0; n];
        let mut expected = vec![0.0; n];
        synth.synthesize(&input, &mut output).unwrap();
        reference.synthesize(&input, &mut expected).unwrap();
        output.fill(123.0);
        assert!(synth.synthesize(&[f32::NAN; 1024], &mut output).is_err());
        assert!(output.iter().all(|x| *x == 123.0));
        assert!(synth.synthesize(&input, &mut [0.0; 1]).is_err());
        synth.synthesize(&input, &mut output).unwrap();
        reference.synthesize(&input, &mut expected).unwrap();
        assert_eq!(output, expected);
        synth.reset();
        reference = LongSineSynthesis::new(n).unwrap();
        synth.synthesize(&input, &mut output).unwrap();
        reference.synthesize(&input, &mut expected).unwrap();
        assert_eq!(output, expected);
        assert!(LongSineSynthesis::new(128).is_err());
    }
}
