//! AAC-LC individual-channel window and section syntax.
//! Spectral Huffman decoding and sample-rate-specific band tables are separate.
use super::{
    aac_synthesis::{WindowSequence, WindowShape},
    bits::BitReader,
};
use crate::{Result, invalid, unsupported};

#[derive(Debug, PartialEq, Eq)]
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
mod tests {
    use super::*;
    fn packed(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut count = 0;
        for &(value, width) in fields {
            for shift in (0..width).rev() {
                if count % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= (((value >> shift) & 1) as u8) << (7 - count % 8);
                count += 1;
            }
        }
        (data, count)
    }
    #[test]
    fn every_short_grouping_partitions_eight_windows() {
        for mask in 0..128 {
            let (data, count) = packed(&[(0, 1), (2, 2), (1, 1), (12, 4), (mask, 7)]);
            let mut bits = BitReader::new(&data);
            let info = IcsInfo::read(&mut bits, (49, 14)).unwrap();
            assert_eq!(bits.position(), count);
            assert_eq!(info.shape, WindowShape::Kbd);
            assert_eq!(info.group_lengths.iter().sum::<u8>(), 8);
            assert_eq!(info.group_lengths.len(), 8 - mask.count_ones() as usize);
            let mut boundaries = 0;
            for &length in info.group_lengths.iter().take(info.group_lengths.len() - 1) {
                boundaries += length;
                assert_eq!(mask & (1 << (7 - boundaries)), 0);
            }
        }
    }
    #[test]
    fn long_syntax_and_errors_preserve_cursor() {
        for sequence in [0, 1, 3] {
            let (data, count) = packed(&[(0, 1), (sequence, 2), (0, 1), (49, 6), (0, 1)]);
            let mut bits = BitReader::new(&data);
            assert_eq!(
                IcsInfo::read(&mut bits, (49, 14)).unwrap().group_lengths,
                [1]
            );
            assert_eq!(bits.position(), count);
        }
        for fields in [
            vec![(1, 1)],
            vec![(0, 1), (0, 2), (0, 1), (50, 6), (0, 1)],
            vec![(0, 1), (0, 2), (0, 1), (40, 6), (1, 1)],
        ] {
            let (data, _) = packed(&fields);
            let mut bits = BitReader::new(&data);
            assert!(IcsInfo::read(&mut bits, (49, 14)).is_err());
            assert_eq!(bits.position(), 0);
        }
        let mut bits = BitReader::new(&[0]);
        assert!(IcsInfo::read(&mut bits, (49, 14)).is_err());
        assert_eq!(bits.position(), 0);
    }
    #[test]
    fn short_sections_keep_group_boundaries_and_special_codebooks() {
        let info = IcsInfo {
            sequence: WindowSequence::EightShort,
            shape: WindowShape::Sine,
            max_sfb: 8,
            group_lengths: vec![3, 5],
        };
        let (data, count) = packed(&[(13, 4), (7, 3), (1, 3), (14, 4), (4, 3), (15, 4), (4, 3)]);
        let mut bits = BitReader::new(&data);
        assert_eq!(
            info.read_sections(&mut bits).unwrap(),
            vec![vec![13; 8], vec![14, 14, 14, 14, 15, 15, 15, 15]]
        );
        assert_eq!(bits.position(), count);
    }
    #[test]
    fn sections_expand_escape_lengths_and_reject_invalid_runs() {
        let info = IcsInfo {
            sequence: WindowSequence::OnlyLong,
            shape: WindowShape::Sine,
            max_sfb: 40,
            group_lengths: vec![1],
        };
        let (data, count) = packed(&[(5, 4), (31, 5), (4, 5), (0, 4), (5, 5)]);
        let mut bits = BitReader::new(&data);
        let books = info.read_sections(&mut bits).unwrap();
        assert_eq!(books[0], [vec![5; 35], vec![0; 5]].concat());
        assert_eq!(bits.position(), count);
        for fields in [
            vec![(12, 4), (1, 5)],
            vec![(5, 4), (0, 5)],
            vec![(5, 4), (31, 5), (10, 5)],
            vec![(5, 4), (31, 5)],
        ] {
            let (data, _) = packed(&fields);
            let mut bits = BitReader::new(&data);
            assert!(info.read_sections(&mut bits).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
