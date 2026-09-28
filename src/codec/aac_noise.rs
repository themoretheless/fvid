//! Owned AAC perceptual-noise band synthesis in spectral units.
use crate::{Result, invalid};

/// Clone the generator before a correlated stereo band to reproduce its shape
/// at another energy. Commit generator state only after the whole packet works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoiseState(u32);
impl Default for NoiseState {
    fn default() -> Self {
        Self(0x6d2b79f5)
    }
}
impl NoiseState {
    /// Energy is the decoded AAC noise scalefactor (-100..155). The squared
    /// spectral norm is 2^(energy/2), independent of band width.
    pub fn band(&mut self, energy: i16, output: &mut [f32]) -> Result<()> {
        if !(-100..=155).contains(&energy) || output.is_empty() || output.len() > 1024 {
            return Err(invalid("invalid AAC noise band size or energy"));
        }
        let mut norm = 0.0f64;
        for value in output.iter_mut() {
            // Full-period xorshift32 from a nonzero state. Only a fixed number
            // of bits is converted, avoiding precision-dependent integer casts.
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            *value = ((self.0 >> 16) as f32 + 0.5) / 32768.0 - 1.0;
            norm += f64::from(*value).powi(2);
        }
        // All 16-bit midpoint values are exact in f32 and nonzero, so
        // even a one-coefficient band has positive norm.
        let scale = 2.0f64.powf(f64::from(energy) / 4.0) / norm.sqrt();
        for value in output {
            *value = (f64::from(*value) * scale) as f32;
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_noise_energy_and_band_width_has_expected_norm() {
        for energy in -100..=155 {
            for width in [1, 4, 8, 12, 28, 64, 120, 128, 960, 1024] {
                let mut output = vec![0.0; width];
                NoiseState::default().band(energy, &mut output).unwrap();
                let actual = output.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>();
                let expected = 2.0f64.powf(f64::from(energy) / 2.0);
                assert!((actual / expected - 1.0).abs() < 2e-7);
            }
        }
    }
    #[test]
    fn cloning_correlates_shapes_and_reset_repeats_initial_band() {
        let mut state = NoiseState::default();
        let mut correlated = state.clone();
        let mut left = [0.0; 64];
        let mut right = [0.0; 64];
        state.band(0, &mut left).unwrap();
        correlated.band(4, &mut right).unwrap();
        for i in 0..64 {
            assert_eq!(right[i], 2.0 * left[i]);
        }
        assert_eq!(state, correlated);
        state.band(0, &mut right).unwrap();
        assert_ne!(left, right);
        state.reset();
        state.band(0, &mut right).unwrap();
        assert_eq!(left, right);
    }
    #[test]
    fn errors_preserve_state_and_output() {
        let mut state = NoiseState::default();
        let previous = state.clone();
        let mut output = [17.0; 4];
        assert!(state.band(156, &mut output).is_err());
        assert_eq!(state, previous);
        assert_eq!(output, [17.0; 4]);
        assert!(state.band(0, &mut []).is_err());
        assert_eq!(state, previous);
    }
}
