//! Owned AAC synthesis building block: consecutive long sine-window blocks.
//! Short-block transitions and KBD windows are not implemented here yet; this
//! component is not used as a fallback for unsupported packet syntax.
use super::aac_imdct::Imdct;
use crate::{Result, invalid};
use std::f64::consts::PI;

pub struct LongSineSynthesis {
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
        let n = self.overlap.len();
        if pcm.len() != n {
            return Err(invalid("AAC synthesis output size mismatch"));
        }
        self.transform.inverse(spectrum, &mut self.scratch)?;
        for i in 0..n {
            pcm[i] = self.overlap[i] + self.scratch[i] * self.window[i];
            self.overlap[i] = self.scratch[n + i] * self.window[n + i];
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
