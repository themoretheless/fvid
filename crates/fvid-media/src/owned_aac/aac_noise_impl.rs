
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
