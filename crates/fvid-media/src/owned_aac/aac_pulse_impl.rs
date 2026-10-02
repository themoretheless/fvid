use super::{aac_synthesis::WindowSequence, bits::BitReader};

pub struct PulseData {
    positions: [usize; 4],
    amplitudes: [i16; 4],
    count: usize,
    frame_samples: usize,
}
impl PulseData {
    /// Called after pulse_data_present. Rejects short windows and invalid
    /// cumulative offsets without advancing the bit cursor.
    pub fn read(
        bits: &mut BitReader<'_>,
        sequence: WindowSequence,
        offsets: &[usize],
    ) -> Result<Self> {
        let frame_samples = offsets.last().copied().unwrap_or(0);
        if sequence == WindowSequence::EightShort
            || !matches!(frame_samples, 960 | 1024)
            || offsets.first() != Some(&0)
            || offsets.windows(2).any(|p| p[0] >= p[1])
        {
            return Err(invalid("invalid AAC pulse window or band table"));
        }
        let mut cursor = bits.clone();
        let count = cursor.read(2)? as usize + 1;
        let band = cursor.read(6)? as usize;
        if band >= offsets.len() - 1 {
            return Err(invalid("AAC pulse start band exceeds table"));
        }
        let mut value = Self {
            positions: [0; 4],
            amplitudes: [0; 4],
            count,
            frame_samples,
        };
        let mut position = offsets[band];
        for i in 0..count {
            position += cursor.read(5)? as usize;
            if position >= frame_samples {
                return Err(invalid("AAC pulse exceeds spectrum"));
            }
            value.positions[i] = position;
            value.amplitudes[i] = cursor.read(4)? as i16;
        }
        *bits = cursor;
        Ok(value)
    }
    /// Repeated positions accumulate; a zero coefficient takes the negative
    /// branch. Up to four amplitude-15 pulses can extend magnitude to 8251.
    pub fn apply(&self, coefficients: &mut [i16]) -> Result<()> {
        if coefficients.len() != self.frame_samples
            || coefficients.iter().any(|&v| i32::from(v).abs() > 8191)
        {
            return Err(invalid("invalid AAC pre-pulse spectrum"));
        }
        for i in 0..self.count {
            let value = &mut coefficients[self.positions[i]];
            if *value > 0 {
                *value += self.amplitudes[i];
            } else {
                *value -= self.amplitudes[i];
            }
        }
        Ok(())
    }
}
