use super::{
    aac_synthesis::{WindowSequence, WindowShape},
    bits::BitReader,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IcsInfo {
    pub sequence: WindowSequence,
    pub shape: WindowShape,
    pub max_sfb: u8,
    pub group_lengths: Vec<u8>,
}
impl IcsInfo {
    /// Parse LC window syntax. `bands` is the number of scale-factor bands in
    /// the selected sample-rate/frame-length table (long, short).
    /// Failure leaves the caller's bit cursor unchanged.
    pub fn read(bits: &mut BitReader<'_>, bands: (u8, u8)) -> Result<Self> {
        let mut cursor = bits.clone();
        if cursor.bit()? {
            return Err(invalid("AAC reserved ICS bit is set"));
        }
        let sequence = match cursor.read(2)? {
            0 => WindowSequence::OnlyLong,
            1 => WindowSequence::LongStart,
            2 => WindowSequence::EightShort,
            _ => WindowSequence::LongStop,
        };
        let shape = if cursor.bit()? {
            WindowShape::Kbd
        } else {
            WindowShape::Sine
        };
        let short = sequence == WindowSequence::EightShort;
        let max_sfb = cursor.read(if short { 4 } else { 6 })? as u8;
        let limit = if short { bands.1 } else { bands.0 };
        if max_sfb > limit {
            return Err(invalid("AAC max_sfb exceeds band table"));
        }
        let mut group_lengths = vec![1];
        if short {
            for _ in 0..7 {
                if cursor.bit()? {
                    *group_lengths.last_mut().unwrap() += 1;
                } else {
                    group_lengths.push(1);
                }
            }
        } else if cursor.bit()? {
            return Err(unsupported("prediction is not allowed in AAC-LC"));
        }
        *bits = cursor;
        Ok(Self {
            sequence,
            shape,
            max_sfb,
            group_lengths,
        })
    }

    /// Map decoded coefficients in group/band/window order into full per-window
    /// spectra. Input contains every band below max_sfb, including zero/noise/
    /// intensity placeholders; higher bands are zero-filled. No allocations.
    pub fn deinterleave(
        &self,
        offsets: &[usize],
        grouped: &[f32],
        output: &mut [f32],
    ) -> Result<()> {
        self.deinterleave_values(offsets, grouped, output, 0.0, |value| value.is_finite())
    }

    pub(crate) fn deinterleave_quantized(
        &self,
        offsets: &[usize],
        grouped: &[i16],
        output: &mut [i16],
    ) -> Result<()> {
        self.deinterleave_values(offsets, grouped, output, 0, |_| true)
    }

    fn deinterleave_values<T: Copy>(
        &self,
        offsets: &[usize],
        grouped: &[T],
        output: &mut [T],
        zero: T,
        valid: impl Fn(&T) -> bool,
    ) -> Result<()> {
        if !matches!(output.len(), 960 | 1024) {
            return Err(invalid("AAC spectrum requires 960 or 1024 samples"));
        }
        let windows = if self.sequence == WindowSequence::EightShort {
            8
        } else {
            1
        };
        let size = output.len() / windows;
        if self.group_lengths.is_empty()
            || self.group_lengths.len() > windows
            || self.group_lengths.iter().any(|&n| n == 0)
            || self
                .group_lengths
                .iter()
                .map(|&n| n as usize)
                .sum::<usize>()
                != windows
        {
            return Err(invalid("invalid AAC window groups"));
        }
        if offsets.first() != Some(&0)
            || offsets.last() != Some(&size)
            || offsets.windows(2).any(|p| p[0] >= p[1])
            || self.max_sfb as usize >= offsets.len()
        {
            return Err(invalid("invalid AAC scale-factor band offsets"));
        }
        let coded = offsets[self.max_sfb as usize];
        if grouped.len() != coded * windows || grouped.iter().any(|x| !valid(x)) {
            return Err(invalid("AAC grouped spectrum size or value is invalid"));
        }
        // All geometry is checked before touching the caller's output.
        output.fill(zero);
        let mut source = 0;
        let mut first_window = 0;
        for &count in &self.group_lengths {
            for band in offsets.windows(2).take(self.max_sfb as usize) {
                let width = band[1] - band[0];
                for window in first_window..first_window + count as usize {
                    let start = window * size + band[0];
                    output[start..start + width].copy_from_slice(&grouped[source..source + width]);
                    source += width;
                }
            }
            first_window += count as usize;
        }
        Ok(())
    }

    /// One codebook per scale-factor band in each window group. Codebook 12
    /// is reserved; 0 is zero, 13 noise, and 14/15 intensity stereo.
    pub fn read_sections(&self, bits: &mut BitReader<'_>) -> Result<Vec<Vec<u8>>> {
        let mut cursor = bits.clone();
        let width = if self.sequence == WindowSequence::EightShort {
            3
        } else {
            5
        };
        let escape = (1 << width) - 1;
        let mut groups = Vec::with_capacity(self.group_lengths.len());
        for _ in &self.group_lengths {
            let mut codebooks = Vec::with_capacity(self.max_sfb as usize);
            while codebooks.len() < self.max_sfb as usize {
                let codebook = cursor.read(4)? as u8;
                if codebook == 12 {
                    return Err(invalid("reserved AAC section codebook"));
                }
                let mut length = 0usize;
                loop {
                    let increment = cursor.read(width)?;
                    length += increment as usize;
                    if length > self.max_sfb as usize - codebooks.len() {
                        return Err(invalid("AAC section exceeds max_sfb"));
                    }
                    if increment != escape {
                        break;
                    }
                }
                if length == 0 {
                    return Err(invalid("empty AAC section"));
                }
                codebooks.resize(codebooks.len() + length, codebook);
            }
            groups.push(codebooks);
        }
        *bits = cursor;
        Ok(groups)
    }
}

#[cfg(test)]
mod quantized_deinterleave_tests {
    use super::*;

    #[test]
    fn quantized_mapping_matches_independent_window_coordinates() {
        for samples in [960, 1024] {
            for groups in [vec![1, 3, 4], vec![8]] {
                let size = samples / 8;
                let offsets = [0, 4, 12, size];
                let info = IcsInfo {
                    sequence: WindowSequence::EightShort,
                    shape: WindowShape::Sine,
                    max_sfb: 2,
                    group_lengths: groups,
                };
                let mut grouped = Vec::new();
                let mut expected = vec![0i16; samples];
                let mut first = 0;
                for &count in &info.group_lengths {
                    for band in offsets.windows(2).take(2) {
                        for window in first..first + usize::from(count) {
                            for coefficient in band[0]..band[1] {
                                let value = if coefficient == 0 {
                                    i16::MIN
                                } else if coefficient == 1 {
                                    i16::MAX
                                } else {
                                    -(window as i16 * 100 + coefficient as i16)
                                };
                                grouped.push(value);
                                expected[window * size + coefficient] = value;
                            }
                        }
                    }
                    first += usize::from(count);
                }
                let mut output = vec![123; samples];
                info.deinterleave_quantized(&offsets, &grouped, &mut output)
                    .unwrap();
                assert_eq!(output, expected);
                let mut floating = vec![123.0; samples];
                let grouped_float: Vec<_> = grouped.iter().map(|&value| f32::from(value)).collect();
                info.deinterleave(&offsets, &grouped_float, &mut floating)
                    .unwrap();
                assert_eq!(
                    floating,
                    expected
                        .iter()
                        .map(|&value| f32::from(value))
                        .collect::<Vec<_>>()
                );
                info.deinterleave_quantized(&offsets, &grouped[..grouped.len() - 1], &mut output)
                    .unwrap_err();
                assert_eq!(output, expected);
                let mut invalid = grouped_float;
                invalid[0] = f32::NAN;
                info.deinterleave(&offsets, &invalid, &mut floating)
                    .unwrap_err();
                assert_eq!(
                    floating,
                    expected
                        .iter()
                        .map(|&value| f32::from(value))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
