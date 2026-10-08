//! Owned SBR inverse-filter bandwidth history (6.18.6.2, table 175).
use super::{Result, aac_sbr_controls::InverseFilterMode, invalid};

#[derive(Clone, Debug, PartialEq)]
pub struct Chirp {
    previous: Vec<InverseFilterMode>,
    bandwidth: Vec<f64>,
}
impl Chirp {
    pub fn new(noise_bands: usize) -> Result<Self> {
        if !(1..=5).contains(&noise_bands) {
            return Err(invalid("invalid SBR chirp band count"));
        }
        Ok(Self {
            previous: vec![InverseFilterMode::Off; noise_bands],
            bandwidth: vec![0.0; noise_bands],
        })
    }
    /// Clear frame history after the enclosing decoder resets its frequency tables.
    pub fn reset(&mut self) {
        self.previous.fill(InverseFilterMode::Off);
        self.bandwidth.fill(0.0);
    }
    pub fn bandwidth(&self) -> &[f64] {
        &self.bandwidth
    }
    /// Modes are indexed by noise frequency band, shared for coupled stereo.
    /// A geometry change requires a fresh state, rather than reusing unrelated bands.
    pub fn update(&mut self, modes: &[InverseFilterMode]) -> Result<&[f64]> {
        if modes.len() != self.previous.len() {
            return Err(invalid("SBR chirp modes do not match noise bands"));
        }
        for ((old_mode, old_bw), &mode) in
            self.previous.iter_mut().zip(&mut self.bandwidth).zip(modes)
        {
            let target = match mode {
                InverseFilterMode::Off => {
                    if *old_mode == InverseFilterMode::Low {
                        0.6
                    } else {
                        0.0
                    }
                }
                InverseFilterMode::Low => {
                    if *old_mode == InverseFilterMode::Off {
                        0.6
                    } else {
                        0.75
                    }
                }
                InverseFilterMode::Intermediate => 0.9,
                InverseFilterMode::High => 0.98,
            };
            *old_bw = smooth(target, *old_bw);
            *old_mode = mode;
        }
        Ok(&self.bandwidth)
    }
}
fn smooth(target: f64, previous: f64) -> f64 {
    let value = if target < previous {
        0.75 * target + 0.25 * previous
    } else {
        0.90625 * target + 0.09375 * previous
    };
    if value < 0.015625 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MODES: [InverseFilterMode; 4] = [
        InverseFilterMode::Off,
        InverseFilterMode::Low,
        InverseFilterMode::Intermediate,
        InverseFilterMode::High,
    ];
    #[test]
    fn independent_decimal_sequences_cover_all_transitions_and_decay() {
        let bytes = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-sbr-chirp-decimal.f64le"
        );
        assert_eq!(bytes.len(), 1024 * 13 * 8);
        for sequence in 0..1024 {
            let mut state = Chirp::new(1).unwrap();
            for frame in 0..13 {
                let mode = if frame < 5 {
                    MODES[(sequence >> (2 * (4 - frame))) & 3]
                } else {
                    MODES[0]
                };
                let offset = (sequence * 13 + frame) * 8;
                let expected = f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
                let actual = state.update(&[mode]).unwrap()[0];
                assert!(
                    (actual - expected).abs() < 3e-16,
                    "sequence {sequence} frame {frame}: {actual} != {expected}"
                );
                assert!((0.0..=0.98).contains(&actual));
            }
            assert_eq!(state.bandwidth(), &[0.0]);
        }
        // Strict less-than at the normative zero threshold.
        assert_eq!(smooth(0.0, 0.0625), 0.015625);
        assert_eq!(smooth(0.0, 0.0625 - f64::EPSILON), 0.0);
    }
    #[test]
    fn bands_are_independent_and_checkpoint_reset_and_refusal_preserve_history() {
        assert!(Chirp::new(0).is_err());
        assert!(Chirp::new(6).is_err());
        for bands in 1..=5 {
            let mut state = Chirp::new(bands).unwrap();
            let mut singles = vec![Chirp::new(1).unwrap(); bands];
            for frame in 0..20 {
                let modes: Vec<_> = (0..bands).map(|band| MODES[(band + frame) % 4]).collect();
                let values = state.update(&modes).unwrap().to_vec();
                for band in 0..bands {
                    assert_eq!(
                        values[band],
                        singles[band].update(&[modes[band]]).unwrap()[0]
                    );
                }
            }
            let checkpoint = state.clone();
            for bad_count in [0, bands + 1] {
                assert!(state.update(&vec![MODES[3]; bad_count]).is_err());
                assert_eq!(state, checkpoint);
            }
            let modes = vec![MODES[1]; bands];
            let first = state.update(&modes).unwrap().to_vec();
            state = checkpoint;
            assert_eq!(state.update(&modes).unwrap(), first);
            state.reset();
            assert_eq!(state, Chirp::new(bands).unwrap());
            for &value in state.update(&vec![MODES[3]; bands]).unwrap() {
                assert!((value - 0.888125).abs() < 2e-16);
            }
        }
    }
}
