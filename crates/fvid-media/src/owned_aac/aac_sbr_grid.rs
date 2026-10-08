//! Bounded legacy SBR grid syntax. Time geometry validation is a separate step.
use super::{Result, bits::BitReader, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameClass {
    FixFix,
    FixVar,
    VarFix,
    VarVar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSyntax {
    pub class: FrameClass,
    /// Offsets from nominal start and end, in time slots.
    pub leading_offset: u8,
    pub trailing_offset: u8,
    pub leading_relative: Vec<u8>,
    pub trailing_relative: Vec<u8>,
    pub pointer: u8,
    /// Ordered by envelope, including reversal of FIXVAR transmitted fields.
    pub high_resolution: Vec<bool>,
}
impl GridSyntax {
    /// Parse syntax transactionally, without interpreting time-slot boundaries.
    /// The bounded two-bit counts limit storage to eight envelope flags.
    pub fn read(bits: &mut BitReader<'_>, end: usize) -> Result<Self> {
        if end < bits.position() || end - bits.position() > bits.remaining() {
            return Err(invalid("invalid SBR grid payload boundary"));
        }
        let mut trial = bits.clone();
        let mut read = |n: u8| -> Result<u8> {
            if usize::from(n) > end.saturating_sub(trial.position()) {
                return Err(invalid("truncated SBR grid"));
            }
            Ok(trial.read(n)? as u8)
        };
        let class = match read(2)? {
            0 => FrameClass::FixFix,
            1 => FrameClass::FixVar,
            2 => FrameClass::VarFix,
            _ => FrameClass::VarVar,
        };
        let mut result = Self {
            class,
            leading_offset: 0,
            trailing_offset: 0,
            leading_relative: Vec::new(),
            trailing_relative: Vec::new(),
            pointer: 0,
            high_resolution: Vec::new(),
        };
        if class == FrameClass::FixFix {
            let count = 1usize << read(2)?;
            result.high_resolution = vec![read(1)? != 0; count];
        } else {
            if matches!(class, FrameClass::VarFix | FrameClass::VarVar) {
                result.leading_offset = read(2)?;
            }
            if matches!(class, FrameClass::FixVar | FrameClass::VarVar) {
                result.trailing_offset = read(2)?;
            }
            let lead = if matches!(class, FrameClass::VarFix | FrameClass::VarVar) {
                read(2)?
            } else {
                0
            };
            let trail = if matches!(class, FrameClass::FixVar | FrameClass::VarVar) {
                read(2)?
            } else {
                0
            };
            for _ in 0..lead {
                result.leading_relative.push(2 * read(2)? + 2);
            }
            for _ in 0..trail {
                result.trailing_relative.push(2 * read(2)? + 2);
            }
            let count = usize::from(lead + trail) + 1;
            // ceil(log2(count + 1)) = bit length of count, no floating point.
            let pointer_bits = (usize::BITS - count.leading_zeros()) as u8;
            result.pointer = read(pointer_bits)?;
            for _ in 0..count {
                result.high_resolution.push(read(1)? != 0);
            }
            if class == FrameClass::FixVar {
                result.high_resolution.reverse();
            }
        }
        *bits = trial;
        Ok(result)
    }
    pub fn noise_envelopes(&self) -> usize {
        if self.high_resolution.len() == 1 {
            1
        } else {
            2
        }
    }
    /// FIXFIX with one envelope forces the finer 1.5 dB envelope resolution.
    pub fn amplitude_resolution(&self, header_resolution: bool) -> bool {
        header_resolution && !(self.class == FrameClass::FixFix && self.high_resolution.len() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(s: &str, offset: usize) -> Vec<u8> {
        let text = "1".repeat(offset) + s + "1111111111111111";
        let mut bytes = vec![0; text.len().div_ceil(8)];
        for (i, b) in text.bytes().enumerate() {
            if b == b'1' {
                bytes[i / 8] |= 1 << (7 - i % 8);
            }
        }
        bytes
    }
    #[test]
    fn all_classes_match_hand_authored_fields_and_rollback_at_every_bit() {
        let samples = [
            (
                "00001",
                FrameClass::FixFix,
                0,
                0,
                vec![],
                vec![],
                0,
                vec![true],
            ),
            (
                "00101",
                FrameClass::FixFix,
                0,
                0,
                vec![],
                vec![],
                0,
                vec![true; 4],
            ),
            (
                "010110000110101",
                FrameClass::FixVar,
                0,
                1,
                vec![],
                vec![2, 4],
                2,
                vec![true, false, true],
            ),
            (
                "101001100101",
                FrameClass::VarFix,
                2,
                0,
                vec![6],
                vec![],
                1,
                vec![false, true],
            ),
            (
                "1101100101101101101",
                FrameClass::VarVar,
                1,
                2,
                vec![6],
                vec![8],
                1,
                vec![true, false, true],
            ),
        ];
        for (s, class, lead, trail, lr, tr, pointer, flags) in samples {
            for offset in 0..8 {
                let bytes = pack(s, offset);
                let mut bits = BitReader::new(&bytes);
                bits.skip(offset).unwrap();
                let end = offset + s.len();
                for limit in offset..end {
                    assert!(GridSyntax::read(&mut bits, limit).is_err());
                    assert_eq!(bits.position(), offset);
                }
                let g = GridSyntax::read(&mut bits, end).unwrap();
                assert_eq!(bits.position(), end, "{s}");
                assert_eq!(
                    (g.class, g.leading_offset, g.trailing_offset, g.pointer),
                    (class, lead, trail, pointer),
                    "{s}"
                );
                assert_eq!(g.leading_relative, lr);
                assert_eq!(g.trailing_relative, tr);
                assert_eq!(g.high_resolution, flags);
                assert_eq!(g.noise_envelopes(), if flags.len() == 1 { 1 } else { 2 });
                assert!(!g.amplitude_resolution(false));
                assert_eq!(
                    g.amplitude_resolution(true),
                    !(class == FrameClass::FixFix && flags.len() == 1)
                );
                assert_eq!(bits.read(8).unwrap(), 255);
            }
        }
    }
}
