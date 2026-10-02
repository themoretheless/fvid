//! Owned AAC perceptual-noise band synthesis in spectral units.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_noise_impl.rs");

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
