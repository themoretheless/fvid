//! Owned AAC Main frequency-domain prediction (ISO/IEC 13818-7, clause 13).
//! This DSP bank is not yet wired into raw-data-block playback. Its snapshots
//! are plain clones; the caller must retain one bank per channel/CCE identity.
use super::{Result, invalid};
const ATTENUATION: f32 = 0.953125;
const ADAPTATION: f32 = 0.90625;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Predictor {
    delay: [f32; 2],
    correlation: [f32; 2],
    variance: [f32; 2],
}
impl Default for Predictor {
    fn default() -> Self {
        Self {
            delay: [0.0; 2],
            correlation: [0.0; 2],
            variance: [1.0; 2],
        }
    }
}
fn truncate(value: f32) -> f32 {
    f32::from_bits(value.to_bits() & 0xffff0000)
}
// Reduced precision, nearest with halfway magnitudes rounded upwards.
fn round_estimate(value: f32) -> f32 {
    let bits = value.to_bits();
    f32::from_bits((bits & 0xffff0000).wrapping_add(if bits & 0x8000 != 0 { 0x10000 } else { 0 }))
}
fn round_even(value: f32) -> f32 {
    let bits = value.to_bits();
    let low = bits & 0xffff;
    let upper = bits >> 16;
    f32::from_bits((upper + u32::from(low > 0x8000 || (low == 0x8000 && upper & 1 != 0))) << 16)
}
fn inverse_variance(value: f32) -> f32 {
    let bits = value.to_bits();
    let exponent = (bits >> 23) & 255;
    if exponent <= 127 {
        return 0.0;
    }
    // Round the inverse mantissa before exact exponent scaling, as specified
    // by the two inverse tables. This order also preserves gradual underflow.
    static MANTISSA: std::sync::OnceLock<[f32; 128]> = std::sync::OnceLock::new();
    let table = MANTISSA.get_or_init(|| {
        std::array::from_fn(|i| {
            round_even(ATTENUATION / f32::from_bits(0x3f800000 | ((i as u32) << 16)))
        })
    });
    let scale = if exponent == 254 {
        f32::from_bits(1 << 22) // 2^-127 (subnormal)
    } else {
        f32::from_bits((254 - exponent) << 23)
    };
    table[((bits >> 16) & 127) as usize] * scale
}

impl Predictor {
    fn reconstruct(&mut self, residual: f32, enabled: bool) -> Result<f32> {
        let first = self.correlation[0] * inverse_variance(self.variance[0]);
        let second = self.correlation[1] * inverse_variance(self.variance[1]);
        let estimate = round_estimate(first * self.delay[0] + second * self.delay[1]);
        let value = if enabled {
            residual + estimate
        } else {
            residual
        };
        let error = value - first * self.delay[0];
        let next = Self {
            delay: [
                truncate(ATTENUATION * value),
                truncate(ATTENUATION * (self.delay[0] - first * value)),
            ],
            correlation: [
                truncate(ADAPTATION * self.correlation[0] + self.delay[0] * value),
                truncate(ADAPTATION * self.correlation[1] + self.delay[1] * error),
            ],
            variance: [
                truncate(
                    ADAPTATION * self.variance[0]
                        + 0.5 * (self.delay[0] * self.delay[0] + value * value),
                ),
                truncate(
                    ADAPTATION * self.variance[1]
                        + 0.5 * (self.delay[1] * self.delay[1] + error * error),
                ),
            ],
        };
        if !value.is_finite()
            || next
                .delay
                .iter()
                .chain(&next.correlation)
                .chain(&next.variance)
                .any(|v| !v.is_finite())
        {
            return Err(invalid("AAC Main prediction arithmetic overflow"));
        }
        *self = next;
        Ok(value)
    }
}

/// Spectral history for one channel. Processing is transactional, including
/// numeric failure, band validation and post-frame interleaved reset.
#[derive(Clone, Debug, PartialEq)]
pub struct MainPredictor {
    lines: Vec<Predictor>,
}
impl MainPredictor {
    pub fn new(lines: usize) -> Result<Self> {
        if lines == 0 || lines > 1024 {
            return Err(invalid("invalid AAC Main predictor extent"));
        }
        Ok(Self {
            lines: vec![Predictor::default(); lines],
        })
    }
    pub fn reset(&mut self) {
        self.lines.fill(Predictor::default());
    }
    /// Long-window spectra are in inverse-quantized AAC units. `offsets`
    /// covers all eligible bands, including those above max_sfb. `used` may
    /// stop at max_sfb; omitted flags are off, but histories still adapt.
    /// PNS/intensity exclusion and tool ordering belong to the channel layer.
    pub fn process(
        &mut self,
        spectrum: &mut [f32],
        offsets: &[usize],
        used: &[bool],
        reset_group: Option<u8>,
    ) -> Result<()> {
        if offsets.len() < 2
            || offsets[0] != 0
            || *offsets.last().unwrap() != self.lines.len()
            || offsets.windows(2).any(|p| p[0] >= p[1])
            || used.len() >= offsets.len()
            || spectrum.len() < self.lines.len()
            || spectrum.iter().any(|v| !v.is_finite())
            || reset_group.is_some_and(|g| !(1..=30).contains(&g))
        {
            return Err(invalid("invalid AAC Main predictor frame layout"));
        }
        let mut next = self.clone();
        let mut reconstructed = spectrum[..self.lines.len()].to_vec();
        for (band, range) in offsets.windows(2).enumerate() {
            for index in range[0]..range[1] {
                reconstructed[index] = next.lines[index]
                    .reconstruct(spectrum[index], used.get(band).copied().unwrap_or(false))?;
            }
        }
        if let Some(group) = reset_group {
            for index in (usize::from(group - 1)..next.lines.len()).step_by(30) {
                next.lines[index] = Predictor::default();
            }
        }
        spectrum[..self.lines.len()].copy_from_slice(&reconstructed);
        *self = next;
        Ok(())
    }
    /// EIGHT_SHORT_SEQUENCE resets the entire bank without modifying spectra.
    pub fn short_window(&mut self) {
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reduced_precision_distinguishes_ties_even_and_away_from_zero() {
        for sign in [0, 0x80000000] {
            assert_eq!(
                round_even(f32::from_bits(sign | 0x3f808000)).to_bits(),
                sign | 0x3f800000
            );
            assert_eq!(
                round_estimate(f32::from_bits(sign | 0x3f808000)).to_bits(),
                sign | 0x3f810000
            );
            assert_eq!(
                round_even(f32::from_bits(sign | 0x3f818000)).to_bits(),
                sign | 0x3f820000
            );
            assert_eq!(
                truncate(f32::from_bits(sign | 0x3f81ffff)).to_bits(),
                sign | 0x3f810000
            );
        }
        assert_eq!(inverse_variance(1.99), 0.0);
    }
    #[test]
    fn inverse_tables_preserve_the_largest_stored_variance_and_subnormals() {
        let variance = f32::from_bits(0x7f7f0000);
        let inverse = inverse_variance(variance);
        assert!(inverse > 0.0 && inverse.is_subnormal());
        let mantissa = f32::from_bits(0x3fff0000);
        assert_eq!(
            inverse,
            round_even(ATTENUATION / mantissa) * f32::from_bits(1 << 22)
        );
        for exponent in 128..252 {
            for mantissa in 0..128 {
                let value = f32::from_bits((exponent << 23) | (mantissa << 16));
                assert_eq!(inverse_variance(value), round_even(ATTENUATION / value));
            }
        }
    }
    #[test]
    fn disabled_prediction_adapts_and_checkpoint_replays_exactly() {
        let mut bank = MainPredictor::new(4).unwrap();
        for value in [32.0, 31.0, 30.0] {
            let mut spectrum = [value; 4];
            bank.process(&mut spectrum, &[0, 4], &[], None).unwrap();
            assert_eq!(spectrum, [value; 4]);
        }
        let checkpoint = bank.clone();
        let mut predicted = [0.0; 4];
        bank.process(&mut predicted, &[0, 4], &[true], None)
            .unwrap();
        assert!(predicted.iter().all(|v| *v != 0.0));
        let mut restored = checkpoint;
        let mut replay = [0.0; 4];
        restored
            .process(&mut replay, &[0, 4], &[true], None)
            .unwrap();
        assert_eq!(replay, predicted);
        assert_eq!(restored, bank);
        bank.short_window();
        assert_eq!(bank, MainPredictor::new(4).unwrap());
    }
    #[test]
    fn reset_groups_are_post_frame_and_cover_every_thirtieth_line() {
        let mut bank = MainPredictor::new(64).unwrap();
        bank.process(&mut [16.0; 64], &[0, 64], &[], None).unwrap();
        let mut baseline = bank.clone();
        let mut a = [1.0; 64];
        let mut b = a;
        bank.process(&mut a, &[0, 64], &[true], Some(2)).unwrap();
        baseline.process(&mut b, &[0, 64], &[true], None).unwrap();
        assert_eq!(a, b);
        for i in 0..64 {
            assert_eq!(
                bank.lines[i],
                if i % 30 == 1 {
                    Predictor::default()
                } else {
                    baseline.lines[i]
                }
            );
        }
    }
    #[test]
    fn invalid_frames_and_overflow_preserve_spectra_and_history() {
        let mut bank = MainPredictor::new(4).unwrap();
        bank.process(&mut [32.0; 4], &[0, 4], &[], None).unwrap();
        let saved = bank.clone();
        for (offsets, used, reset) in [
            (vec![0, 4], vec![true], Some(0)),
            (vec![0, 4], vec![true], Some(31)),
            (vec![0, 3], vec![true], None),
            (vec![0, 0, 4], vec![], None),
            (vec![0, 4], vec![true, false], None),
        ] {
            let mut spectrum = [1.0; 4];
            assert!(bank.process(&mut spectrum, &offsets, &used, reset).is_err());
            assert_eq!(spectrum, [1.0; 4]);
            assert_eq!(bank, saved);
        }
        let mut spectrum = [f32::MAX; 4];
        assert!(bank.process(&mut spectrum, &[0, 4], &[true], None).is_err());
        assert_eq!(spectrum, [f32::MAX; 4]);
        assert_eq!(bank, saved);
    }
}
