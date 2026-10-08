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
/// Validated envelope/noise borders expressed in SBR time slots (not QMF samples).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeGrid {
    pub envelope: Vec<u8>,
    pub noise: Vec<u8>,
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
    /// Reconstruct the normative grid for 960/1024-sample AAC core frames.
    pub fn time_grid(&self, time_slots: u8) -> Result<TimeGrid> {
        let count = self.high_resolution.len();
        let lead = self.leading_relative.len();
        let trail = self.trailing_relative.len();
        if !matches!(time_slots, 15 | 16)
            || count == 0
            || self.leading_offset > 3
            || self.trailing_offset > 3
            || lead > 3
            || trail > 3
            || usize::from(self.pointer) > count
            || self
                .leading_relative
                .iter()
                .chain(&self.trailing_relative)
                .any(|x| !matches!(*x, 2 | 4 | 6 | 8))
        {
            return Err(invalid("invalid SBR time grid parameters"));
        }
        let shape_valid = match self.class {
            FrameClass::FixFix => {
                matches!(count, 1 | 2 | 4)
                    && lead == 0
                    && trail == 0
                    && self.leading_offset == 0
                    && self.trailing_offset == 0
                    && self.pointer == 0
                    && self
                        .high_resolution
                        .iter()
                        .all(|v| *v == self.high_resolution[0])
            }
            FrameClass::FixVar => {
                count <= 4 && lead == 0 && self.leading_offset == 0 && count == trail + 1
            }
            FrameClass::VarFix => {
                count <= 4 && trail == 0 && self.trailing_offset == 0 && count == lead + 1
            }
            FrameClass::VarVar => count <= 5 && count == lead + trail + 1,
        };
        if !shape_valid {
            return Err(invalid("inconsistent SBR grid class or envelope count"));
        }
        let mut envelope = vec![0; count + 1];
        envelope[0] = self.leading_offset;
        envelope[count] = time_slots + self.trailing_offset;
        if self.class == FrameClass::FixFix {
            // NINT(numTimeSlots / count), including 15-slot core frames.
            let step = (usize::from(time_slots) + count / 2) / count;
            for (i, b) in envelope.iter_mut().enumerate().take(count).skip(1) {
                *b = (i * step) as u8;
            }
        } else {
            for (i, &step) in self.leading_relative.iter().enumerate() {
                envelope[i + 1] = envelope[i]
                    .checked_add(step)
                    .ok_or_else(|| invalid("overflowing SBR leading border"))?;
            }
            for (i, &step) in self.trailing_relative.iter().enumerate() {
                envelope[count - i - 1] = envelope[count - i]
                    .checked_sub(step)
                    .ok_or_else(|| invalid("negative SBR trailing border"))?;
            }
        }
        if envelope.windows(2).any(|w| w[0] >= w[1]) {
            return Err(invalid("overlapping SBR envelope borders"));
        }
        let noise = if count == 1 {
            envelope.clone()
        } else {
            let pointer = usize::from(self.pointer);
            let middle = match self.class {
                FrameClass::FixFix => count / 2,
                FrameClass::VarFix => match pointer {
                    0 => 1,
                    1 => count - 1,
                    _ => pointer - 1,
                },
                FrameClass::FixVar | FrameClass::VarVar => {
                    if pointer <= 1 {
                        count - 1
                    } else {
                        count + 1 - pointer
                    }
                }
            };
            if middle == 0 || middle >= count {
                return Err(invalid("invalid SBR noise split"));
            }
            vec![envelope[0], envelope[middle], envelope[count]]
        };
        Ok(TimeGrid { envelope, noise })
    }

    /// Read and validate atomically: semantic failure also restores bit position.
    pub fn read_validated(
        bits: &mut BitReader<'_>,
        end: usize,
        time_slots: u8,
    ) -> Result<(Self, TimeGrid)> {
        let mut trial = bits.clone();
        let syntax = Self::read(&mut trial, end)?;
        let grid = syntax.time_grid(time_slots)?;
        *bits = trial;
        Ok((syntax, grid))
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
    #[test]
    fn exact_time_borders_and_noise_splits_for_both_core_lengths() {
        for slots in [15, 16] {
            for (s, envelope, noise) in [
                ("00001", vec![0, slots], vec![0, slots]),
                ("00101", vec![0, 4, 8, 12, slots], vec![0, 8, slots]),
                (
                    "010110000110101",
                    vec![0, slots - 5, slots - 1, slots + 1],
                    vec![0, slots - 1, slots + 1],
                ),
                ("101001100101", vec![2, 8, slots], vec![2, 8, slots]),
                (
                    "1101100101101101101",
                    vec![1, 7, slots - 6, slots + 2],
                    vec![1, slots - 6, slots + 2],
                ),
            ] {
                let bytes = pack(s, 0);
                let mut bits = BitReader::new(&bytes);
                let (_, g) = GridSyntax::read_validated(&mut bits, s.len(), slots).unwrap();
                assert_eq!(g.envelope, envelope);
                assert_eq!(g.noise, noise);
                assert_eq!(bits.position(), s.len());
            }
        }
    }

    #[test]
    fn malformed_geometry_rolls_back_and_public_values_cannot_panic() {
        // FIXFIX eight envelopes; VARFIX three relative steps of eight slots.
        for s in ["00111", "1000111111110001111"] {
            let bytes = pack(s, 3);
            let mut probe = BitReader::new(&bytes);
            probe.skip(3).unwrap();
            let parsed = GridSyntax::read(&mut probe, 3 + s.len()).unwrap();
            assert_eq!(probe.position(), 3 + s.len());
            assert!(parsed.time_grid(16).is_err());
            let mut bits = BitReader::new(&bytes);
            bits.skip(3).unwrap();
            assert!(GridSyntax::read_validated(&mut bits, 3 + s.len(), 16).is_err());
            assert_eq!(bits.position(), 3);
        }
        let bytes = pack("1101100101101101101", 0);
        let base = GridSyntax::read(&mut BitReader::new(&bytes), 19).unwrap();
        for value in 0..=255 {
            let mut g = base.clone();
            g.leading_offset = value;
            if value > 3 {
                assert!(g.time_grid(16).is_err());
            }
            let mut g = base.clone();
            g.trailing_relative[0] = value;
            if !matches!(value, 2 | 4 | 6 | 8) {
                assert!(g.time_grid(16).is_err());
            }
            let mut g = base.clone();
            g.pointer = value;
            if value > 3 {
                assert!(g.time_grid(16).is_err());
            }
        }
        for slots in [0, 1, 14, 17, 255] {
            assert!(base.time_grid(slots).is_err());
        }
    }
}
