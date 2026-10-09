//! Ordinary AAC LTP time-domain history. Profile dispatch/analysis is separate.
use super::{
    Result,
    aac_ltp_syntax::{LtpData, Usage},
    invalid, unsupported,
};

const GAINS: [f64; 8] = [
    0.570829, 0.696616, 0.813004, 0.911304, 0.984900, 1.067894, 1.194601, 1.369533,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LtpHistory {
    samples: Vec<i16>,
    frame_samples: usize,
}
impl LtpHistory {
    pub fn new(frame_samples: usize) -> Result<Self> {
        if !matches!(frame_samples, 960 | 1024) {
            return Err(invalid("invalid AAC LTP history geometry"));
        }
        Ok(Self {
            samples: vec![0; 4 * frame_samples],
            frame_samples,
        })
    }
    /// Raw synthesis units, before PCM normalization. Quantize to signed 16-bit
    /// using nearest-even rounding and saturation. Validate before mutation.
    pub fn update_raw(&mut self, pcm: &[f64], overlap: &[f64]) -> Result<()> {
        let n = self.frame_samples;
        if pcm.len() != n || overlap.len() != n || pcm.iter().chain(overlap).any(|x| !x.is_finite())
        {
            return Err(invalid("invalid AAC LTP history samples"));
        }
        self.samples.copy_within(n..2 * n, 0);
        for (slot, value) in self.samples[n..3 * n]
            .iter_mut()
            .zip(pcm.iter().chain(overlap))
        {
            *slot = value.round_ties_even().clamp(-32768.0, 32767.0) as i16;
        }
        // The final frame stays zero: the unobserved future is never extrapolated.
        Ok(())
    }
    /// Prepare a two-frame input for long-window LTP analysis. All geometry
    /// checks precede output writes; this does not advance the history.
    pub fn estimate_long(&self, data: &LtpData, output: &mut [f64]) -> Result<()> {
        let n = self.frame_samples;
        let Usage::Bands(used) = &data.usage else {
            return Err(unsupported(
                "AAC short-window LTP analysis is not integrated",
            ));
        };
        if used.len() > 40 || usize::from(data.lag) > 2 * n || output.len() != 2 * n {
            return Err(invalid("invalid AAC LTP estimate geometry"));
        }
        let gain = GAINS
            .get(usize::from(data.coefficient_index))
            .ok_or_else(|| invalid("invalid AAC LTP coefficient index"))?;
        let start = 2 * n - usize::from(data.lag);
        for (out, value) in output.iter_mut().zip(&self.samples[start..start + 2 * n]) {
            *out = f64::from(*value) * gain;
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        self.samples.fill(0);
    }
    /// Restore a checkpoint without allocating; a mismatch leaves state intact.
    pub fn restore(&mut self, saved: &Self) -> Result<()> {
        if self.frame_samples != saved.frame_samples {
            return Err(invalid("AAC LTP history frame-size mismatch"));
        }
        self.samples.copy_from_slice(&saved.samples);
        Ok(())
    }
    pub fn storage_bytes(&self) -> usize {
        self.samples.capacity() * std::mem::size_of::<i16>()
    }
}
