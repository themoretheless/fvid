//! AAC ordinary spectral-band inverse quantization, before window synthesis.
//! Noise and intensity codebooks require their own reconstruction operations.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_quant_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cubes_reconstruct_exact_fourth_powers_and_octaves() {
        for root in -20i16..=20 {
            let q = root * root * root;
            for sf in [0, 96, 100, 104, 252] {
                let mut out = [0.0];
                inverse_quantize(&[q], sf, &mut out).unwrap();
                let expected = (f64::from(root).signum()
                    * f64::from(root).powi(4)
                    * 2.0f64.powi((i32::from(sf) - 100) / 4)) as f32;
                assert_eq!(out[0], expected);
            }
        }
    }
    #[test]
    fn entire_escape_range_matches_independent_power_evaluation() {
        let input: Vec<_> = (-8191i16..=8191).collect();
        let mut output = vec![0.0; input.len()];
        for sf in [0, 99, 100, 101, 102, 103, 255] {
            inverse_quantize(&input, sf, &mut output).unwrap();
            for (&q, &actual) in input.iter().zip(&output) {
                let expected = f64::from(q).signum()
                    * f64::from(q).abs().powf(4.0 / 3.0)
                    * 2.0f64.powf((f64::from(sf) - 100.0) / 4.0);
                assert!(actual.is_finite());
                assert!((f64::from(actual) - expected).abs() <= expected.abs() * 1e-7 + 1e-20);
            }
        }
    }
    #[test]
    fn invalid_input_does_not_partially_write_band() {
        for q in [8252, -8252, i16::MIN, i16::MAX] {
            let mut out = [17.0; 2];
            assert!(inverse_quantize(&[1, q], 100, &mut out).is_err());
            assert_eq!(out, [17.0; 2]);
        }
        for sf in [-1, 256] {
            let mut out = [17.0];
            assert!(inverse_quantize(&[1], sf, &mut out).is_err());
            assert_eq!(out, [17.0]);
        }
        assert!(inverse_quantize(&[1], 100, &mut []).is_err());
    }
}
