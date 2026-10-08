//! Owned legacy SBR delta-direction and inverse-filter syntax.
use super::{Result, aac_sbr_grid::GridSyntax, bits::BitReader, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaDirection {
    Frequency,
    Time,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaFlags {
    pub envelope: Vec<DeltaDirection>,
    pub noise: Vec<DeltaDirection>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InverseFilterMode {
    Off,
    Low,
    Intermediate,
    High,
}

// Keep fields inside the enclosing extension, including unaligned boundaries.
fn field(bits: &mut BitReader<'_>, end: usize, width: u8) -> Result<u8> {
    if end < bits.position()
        || end - bits.position() > bits.remaining()
        || usize::from(width) > end - bits.position()
    {
        return Err(invalid("truncated or invalid SBR control payload"));
    }
    Ok(bits.read(width)? as u8)
}
impl DeltaFlags {
    /// Parse sbr_dtdf. Validate the grid before consuming any payload bits.
    pub fn read(
        bits: &mut BitReader<'_>,
        end: usize,
        grid: &GridSyntax,
        slots: u8,
    ) -> Result<Self> {
        let time = grid.time_grid(slots)?;
        let mut trial = bits.clone();
        let mut directions = |count: usize| -> Result<Vec<DeltaDirection>> {
            (0..count)
                .map(|_| {
                    Ok(if field(&mut trial, end, 1)? == 0 {
                        DeltaDirection::Frequency
                    } else {
                        DeltaDirection::Time
                    })
                })
                .collect()
        };
        let result = Self {
            envelope: directions(time.envelope.len() - 1)?,
            noise: directions(time.noise.len() - 1)?,
        };
        *bits = trial;
        Ok(result)
    }
}

/// Parse sbr_invf. Noise band count comes from the frequency tables, not from
/// noise envelope count. In coupled stereo this list is shared by both channels.
pub fn read_inverse_filter(
    bits: &mut BitReader<'_>,
    end: usize,
    noise_bands: usize,
) -> Result<Vec<InverseFilterMode>> {
    if !(1..=5).contains(&noise_bands) {
        return Err(invalid("invalid SBR inverse-filter band count"));
    }
    let mut trial = bits.clone();
    let mut modes = Vec::with_capacity(noise_bands);
    for _ in 0..noise_bands {
        modes.push(match field(&mut trial, end, 2)? {
            0 => InverseFilterMode::Off,
            1 => InverseFilterMode::Low,
            2 => InverseFilterMode::Intermediate,
            _ => InverseFilterMode::High,
        });
    }
    *bits = trial;
    Ok(modes)
}

#[cfg(test)]
mod tests {
    use super::super::aac_sbr_grid::FrameClass;
    use super::*;
    fn pack(value: u32, width: usize, offset: usize) -> Vec<u8> {
        let mut bytes = vec![255; (offset + width + 16).div_ceil(8)];
        for i in 0..width {
            let bit = offset + i;
            if value & (1 << (width - 1 - i)) == 0 {
                bytes[bit / 8] &= !(1 << (7 - bit % 8));
            }
        }
        bytes
    }
    fn grid(count: usize) -> GridSyntax {
        let (class, lead, trail) = if count == 1 {
            (FrameClass::FixFix, vec![], vec![])
        } else if count == 5 {
            (FrameClass::VarVar, vec![2, 2], vec![2, 2])
        } else {
            (FrameClass::VarFix, vec![2; count - 1], vec![])
        };
        GridSyntax {
            class,
            leading_offset: 0,
            trailing_offset: 0,
            leading_relative: lead,
            trailing_relative: trail,
            pointer: 0,
            high_resolution: vec![true; count],
        }
    }
    #[test]
    fn every_delta_pattern_at_every_bit_offset_has_exact_order_and_boundary() {
        for count in 1..=5 {
            let noise = if count == 1 { 1 } else { 2 };
            let width = count + noise;
            for value in 0..(1u32 << width) {
                for offset in 0..8 {
                    let bytes = pack(value, width, offset);
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(offset).unwrap();
                    for end in offset..offset + width {
                        assert!(DeltaFlags::read(&mut bits, end, &grid(count), 16).is_err());
                        assert_eq!(bits.position(), offset);
                    }
                    let flags =
                        DeltaFlags::read(&mut bits, offset + width, &grid(count), 16).unwrap();
                    let expected: Vec<_> = (0..width)
                        .map(|i| {
                            if (value >> (width - i - 1)) & 1 == 0 {
                                DeltaDirection::Frequency
                            } else {
                                DeltaDirection::Time
                            }
                        })
                        .collect();
                    assert_eq!(flags.envelope, expected[..count]);
                    assert_eq!(flags.noise, expected[count..]);
                    assert_eq!(bits.position(), offset + width);
                    assert_eq!(bits.read(8).unwrap(), 255);
                }
            }
        }
    }
    #[test]
    fn all_inverse_filter_sequences_and_truncations_are_transactional() {
        let modes = [
            InverseFilterMode::Off,
            InverseFilterMode::Low,
            InverseFilterMode::Intermediate,
            InverseFilterMode::High,
        ];
        for count in 1..=5 {
            let width = 2 * count;
            for value in 0..(1u32 << width) {
                for offset in 0..8 {
                    let bytes = pack(value, width, offset);
                    let mut bits = BitReader::new(&bytes);
                    bits.skip(offset).unwrap();
                    for end in offset..offset + width {
                        assert!(read_inverse_filter(&mut bits, end, count).is_err());
                        assert_eq!(bits.position(), offset);
                    }
                    let actual = read_inverse_filter(&mut bits, offset + width, count).unwrap();
                    let expected: Vec<_> = (0..count)
                        .map(|i| modes[((value >> (2 * (count - i - 1))) & 3) as usize])
                        .collect();
                    assert_eq!(actual, expected);
                    assert_eq!(bits.position(), offset + width);
                    assert_eq!(bits.read(8).unwrap(), 255);
                }
            }
        }
    }
    #[test]
    fn invalid_external_geometry_and_payload_limits_do_not_advance() {
        let mut bits = BitReader::new(&[255; 4]);
        bits.skip(3).unwrap();
        for end in [0, 2, 33, usize::MAX] {
            assert!(DeltaFlags::read(&mut bits, end, &grid(1), 16).is_err());
            assert!(read_inverse_filter(&mut bits, end, 1).is_err());
            assert_eq!(bits.position(), 3);
        }
        for count in [0, 6, usize::MAX] {
            assert!(read_inverse_filter(&mut bits, 32, count).is_err());
        }
        let mut broken = grid(2);
        broken.leading_relative[0] = 255;
        assert!(DeltaFlags::read(&mut bits, 32, &broken, 16).is_err());
        assert_eq!(bits.position(), 3);
    }
}
