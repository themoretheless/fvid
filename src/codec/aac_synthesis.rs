//! Owned AAC synthesis building block: long/short sine-window sequences.
//! KBD windows are not implemented here yet; this
//! component is not used as a fallback for unsupported packet syntax.
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

pub struct LongSineSynthesis {
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
        let n = self.overlap.len();
        let short = n / 8;
        let offset = (n - short) / 2;
        if pcm.len() != n || spectrum.len() != n || spectrum.iter().any(|x| !x.is_finite()) {
            return Err(invalid("AAC synthesis invalid spectrum or output size"));
        }
        if sequence == WindowSequence::EightShort {
            self.scratch.fill(0.0);
            for (block, coefficients) in spectrum.chunks_exact(short).enumerate() {
                self.short_transform
                    .inverse(coefficients, &mut self.short_scratch)?;
                for i in 0..2 * short {
                    self.scratch[offset + block * short + i] +=
                        self.short_scratch[i] * self.short_window[i];
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
                            self.short_window[short + j - offset]
                        } else {
                            0.0
                        }
                    }
                    WindowSequence::LongStop if i < n => {
                        if i < offset {
                            0.0
                        } else if i < offset + short {
                            self.short_window[i - offset]
                        } else {
                            1.0
                        }
                    }
                    _ => self.window[i],
                };
                self.scratch[i] *= weight;
            }
        }
        for i in 0..n {
            pcm[i] = self.overlap[i] + self.scratch[i];
            self.overlap[i] = self.scratch[n + i];
        }
        Ok(())
    }
    /// Discard the previous frame's contribution after a seek/reset.
    pub fn reset(&mut self) {
        self.overlap.fill(0.0);
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
            for (frame, sequence) in sequences.into_iter().enumerate() {
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
                            (PI * (position as f64 + 0.5) / (2 * length) as f64).sin()
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
                    .synthesize_sequence(sequence, &spectrum, &mut pcm)
                    .unwrap();
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
