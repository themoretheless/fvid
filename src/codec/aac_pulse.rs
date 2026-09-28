//! AAC pulse reconstruction on long-window quantized coefficients.
use super::{aac_synthesis::WindowSequence, bits::BitReader};
use crate::{Result, invalid};

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

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut n = 0;
        for &(value, width) in fields {
            for bit in (0..width).rev() {
                if n % 8 == 0 {
                    out.push(0);
                }
                *out.last_mut().unwrap() |= ((value >> bit & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        out
    }
    #[test]
    fn signed_zero_and_repeated_pulses_apply_before_quantization() {
        let data = pack(&[
            (3, 2),
            (1, 6),
            (0, 5),
            (3, 4),
            (1, 5),
            (2, 4),
            (1, 5),
            (4, 4),
            (0, 5),
            (5, 4),
        ]);
        let mut bits = BitReader::new(&data);
        let pulse = PulseData::read(&mut bits, WindowSequence::OnlyLong, &[0, 4, 1024]).unwrap();
        assert_eq!(bits.position(), 44);
        let mut coefficients = vec![0; 1024];
        coefficients[4] = 2;
        coefficients[5] = -1;
        pulse.apply(&mut coefficients).unwrap();
        assert_eq!(&coefficients[4..7], &[5, -3, -9]);
        let mut spectrum = [0.0; 3];
        super::super::aac_quant::inverse_quantize(&coefficients[4..7], 100, &mut spectrum).unwrap();
        assert!(spectrum[0] > 0.0 && spectrum[1] < 0.0 && spectrum[2] < spectrum[1]);
    }
    #[test]
    fn maximum_pulses_remain_reconstructible() {
        let data = pack(&[
            (3, 2),
            (0, 6),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
        ]);
        let pulse = PulseData::read(
            &mut BitReader::new(&data),
            WindowSequence::LongStart,
            &[0, 960],
        )
        .unwrap();
        for initial in [-8191, 8191] {
            let mut values = vec![0; 960];
            values[0] = initial;
            pulse.apply(&mut values).unwrap();
            assert_eq!(values[0].abs(), 8251);
            super::super::aac_quant::inverse_quantize(&values, 255, &mut vec![0.0; 960]).unwrap();
        }
    }
    #[test]
    fn invalid_offsets_and_truncation_preserve_cursor() {
        for data in [
            pack(&[(0, 2), (1, 6), (1, 5), (1, 4)]),
            pack(&[(0, 2), (2, 6), (0, 5), (1, 4)]),
            vec![0],
        ] {
            let mut bits = BitReader::new(&data);
            assert!(
                PulseData::read(&mut bits, WindowSequence::OnlyLong, &[0, 1023, 1024]).is_err()
            );
            assert_eq!(bits.position(), 0);
        }
        assert!(
            PulseData::read(
                &mut BitReader::new(&[0; 8]),
                WindowSequence::EightShort,
                &[0, 1024]
            )
            .is_err()
        );
    }
}
