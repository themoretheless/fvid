//! Inter macroblock partition syntax, H.264 tables 7-13/14/17/18.
use super::{avc_slice::SliceType, bits::BitReader};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prediction {
    L0,
    L1,
    Bi,
    Direct,
}
impl Prediction {
    fn uses(self, list: usize) -> bool {
        self == Self::Bi || self == if list == 0 { Self::L0 } else { Self::L1 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub origin: [u8; 2],
    pub size: [u8; 2],
    pub prediction: Prediction,
    /// Shared reference-index syntax group (macroblock partition index).
    pub group: usize,
    pub references: [Option<u8>; 2],
    pub differences: [[i32; 2]; 2],
}
#[derive(Debug, PartialEq, Eq)]
pub enum MacroblockType {
    /// Index in the I macroblock type table, including I_PCM (25).
    Intra(u8),
    Inter {
        partitions: Vec<Partition>,
        reference_zero: bool,
    },
    Subdivided {
        reference_zero: bool,
    },
}
fn part(origin: [u8; 2], size: [u8; 2], prediction: Prediction, group: usize) -> Partition {
    Partition {
        origin,
        size,
        prediction,
        group,
        references: [None; 2],
        differences: [[0; 2]; 2],
    }
}
pub fn macroblock_type(slice: SliceType, code: u32) -> Result<MacroblockType> {
    use Prediction::*;
    let offset = match slice {
        SliceType::P | SliceType::Sp => 5,
        SliceType::B => 23,
        SliceType::I => 0,
        SliceType::Si => return Err(crate::unsupported("SI macroblock syntax is not implemented")),
    };
    if code >= offset {
        let intra = code - offset;
        return if intra <= 25 {
            Ok(MacroblockType::Intra(intra as u8))
        } else {
            Err(invalid("AVC macroblock type out of range"))
        };
    }
    let mut partitions = Vec::new();
    if slice != SliceType::B {
        if code >= 3 {
            return Ok(MacroblockType::Subdivided {
                reference_zero: code == 4,
            });
        }
        let size = match code {
            0 => [16, 16],
            1 => [16, 8],
            _ => [8, 16],
        };
        partitions.push(part([0, 0], size, L0, 0));
        if code != 0 {
            partitions.push(part(if code == 1 { [0, 8] } else { [8, 0] }, size, L0, 1));
        }
    } else {
        match code {
            0 => {
                for group in 0..4 {
                    append_sub(&mut partitions, group, Direct, [4, 4]);
                }
            }
            1..=3 => partitions.push(part([0, 0], [16, 16], [L0, L1, Bi][code as usize - 1], 0)),
            22 => {
                return Ok(MacroblockType::Subdivided {
                    reference_zero: false,
                });
            }
            _ => {
                let modes = [
                    [L0, L0],
                    [L1, L1],
                    [L0, L1],
                    [L1, L0],
                    [L0, Bi],
                    [L1, Bi],
                    [Bi, L0],
                    [Bi, L1],
                    [Bi, Bi],
                ][(code as usize - 4) / 2];
                let size = if code % 2 == 0 { [16, 8] } else { [8, 16] };
                partitions.push(part([0, 0], size, modes[0], 0));
                partitions.push(part(
                    if code % 2 == 0 { [0, 8] } else { [8, 0] },
                    size,
                    modes[1],
                    1,
                ));
            }
        }
    }
    Ok(MacroblockType::Inter {
        partitions,
        reference_zero: false,
    })
}
fn append_sub(out: &mut Vec<Partition>, group: usize, prediction: Prediction, size: [u8; 2]) {
    let base = [(group % 2) as u8 * 8, (group / 2) as u8 * 8];
    for y in (0..8).step_by(size[1] as usize) {
        for x in (0..8).step_by(size[0] as usize) {
            out.push(part([base[0] + x, base[1] + y], size, prediction, group));
        }
    }
}
pub fn sub_partitions(slice: SliceType, codes: [u32; 4]) -> Result<Vec<Partition>> {
    use Prediction::*;
    let mut out = Vec::with_capacity(16);
    for (group, code) in codes.into_iter().enumerate() {
        let (prediction, size) = match slice {
            SliceType::P | SliceType::Sp => (
                L0,
                match code {
                    0 => [8, 8],
                    1 => [8, 4],
                    2 => [4, 8],
                    3 => [4, 4],
                    _ => return Err(invalid("AVC P sub-macroblock type out of range")),
                },
            ),
            SliceType::B => match code {
                0 => (Direct, [4, 4]),
                1..=3 => ([L0, L1, Bi][code as usize - 1], [8, 8]),
                4..=9 => (
                    [L0, L1, Bi][(code as usize - 4) / 2],
                    if code % 2 == 0 { [8, 4] } else { [4, 8] },
                ),
                10..=12 => ([L0, L1, Bi][code as usize - 10], [4, 4]),
                _ => return Err(invalid("AVC B sub-macroblock type out of range")),
            },
            _ => return Err(invalid("sub-macroblocks require an inter slice")),
        };
        append_sub(&mut out, group, prediction, size);
    }
    Ok(out)
}
/// Read progressive CAVLC inter prediction syntax following mb_type. Intra types
/// must be dispatched to the intra reader. Direct vectors are derived separately.
/// On an error the bit reader is restored to its initial position.
pub fn read_prediction(
    bits: &mut BitReader<'_>,
    slice: SliceType,
    code: u32,
    active: [u32; 2],
) -> Result<Vec<Partition>> {
    let mut input = bits.clone();
    let (mut partitions, zero) = match macroblock_type(slice, code)? {
        MacroblockType::Intra(_) => return Err(invalid("expected AVC inter macroblock")),
        MacroblockType::Inter {
            partitions,
            reference_zero,
        } => (partitions, reference_zero),
        MacroblockType::Subdivided { reference_zero } => {
            let mut codes = [0; 4];
            for c in &mut codes {
                *c = input.unsigned_golomb()?;
            }
            (sub_partitions(slice, codes)?, reference_zero)
        }
    };
    // ref_idx_l0 for all partitions precedes ref_idx_l1, then all MVDs.
    for list in 0..2 {
        for group in 0..4 {
            if !partitions
                .iter()
                .any(|p| p.group == group && p.prediction.uses(list))
            {
                continue;
            }
            let count = active[list];
            if count == 0 || count > 32 {
                return Err(invalid("AVC active reference count out of range"));
            }
            let index = if count == 1 || (zero && list == 0) {
                0
            } else if count == 2 {
                u32::from(!input.bit()?)
            } else {
                input.unsigned_golomb()?
            };
            if index >= count {
                return Err(invalid("AVC reference index out of range"));
            }
            for p in partitions
                .iter_mut()
                .filter(|p| p.group == group && p.prediction.uses(list))
            {
                p.references[list] = Some(index as u8);
            }
        }
    }
    for list in 0..2 {
        for p in &mut partitions {
            if p.prediction.uses(list) {
                p.differences[list] = [input.signed_golomb()?, input.signed_golomb()?];
            }
        }
    }
    *bits = input;
    Ok(partitions)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn coverage(parts: &[Partition]) {
        let mut pixels = [0; 256];
        for p in parts {
            for y in p.origin[1]..p.origin[1] + p.size[1] {
                for x in p.origin[0]..p.origin[0] + p.size[0] {
                    pixels[y as usize * 16 + x as usize] += 1;
                }
            }
        }
        assert_eq!(pixels, [1; 256]);
    }
    #[test]
    fn partition_tables_cover_every_pixel_once() {
        for (slice, end) in [(SliceType::P, 5), (SliceType::B, 23)] {
            for code in 0..end {
                match macroblock_type(slice, code).unwrap() {
                    MacroblockType::Inter { partitions, .. } => coverage(&partitions),
                    MacroblockType::Subdivided { .. } => {
                        for sub in 0..if slice == SliceType::P { 4 } else { 13 } {
                            coverage(&sub_partitions(slice, [sub; 4]).unwrap());
                        }
                    }
                    _ => panic!(),
                }
            }
            assert_eq!(
                macroblock_type(slice, end).unwrap(),
                MacroblockType::Intra(0)
            );
            assert_eq!(
                macroblock_type(slice, end + 25).unwrap(),
                MacroblockType::Intra(25)
            );
            assert!(macroblock_type(slice, end + 26).is_err());
        }
        let MacroblockType::Inter { partitions, .. } = macroblock_type(SliceType::B, 19).unwrap()
        else {
            panic!()
        };
        assert_eq!(partitions[0].prediction, Prediction::Bi);
        assert_eq!(partitions[1].prediction, Prediction::L1);
        assert_eq!(partitions[1].origin, [8, 0]);
    }
    #[test]
    fn cavlc_reference_and_motion_order() {
        // B_Bi_16x16, two active references: ref0=1, ref1=0,
        // MVD L0=(+1,-1), L1=(0,+2): 0 1 010 011 1 00100.
        let mut bits = BitReader::new(&[0x53, 0x90]);
        let p = read_prediction(&mut bits, SliceType::B, 3, [2, 2]).unwrap();
        assert_eq!(p[0].references, [Some(1), Some(0)]);
        assert_eq!(p[0].differences, [[1, -1], [0, 2]]);
        assert_eq!(bits.position(), 14);
        // All P_8x8ref0 subtypes are 8x8 (four ue(0)), followed by 8 se(0).
        let mut bits = BitReader::new(&[0xff, 0xf0]);
        let p = read_prediction(&mut bits, SliceType::P, 4, [4, 0]).unwrap();
        assert_eq!(bits.position(), 12);
        assert!(p.iter().all(|p| p.references == [Some(0), None]));
        let mut bits = BitReader::new(&[0x53]);
        assert!(read_prediction(&mut bits, SliceType::B, 3, [2, 2]).is_err());
        assert_eq!(bits.position(), 0);
        let mut bits = BitReader::new(&[]);
        let direct = read_prediction(&mut bits, SliceType::B, 0, [1, 1]).unwrap();
        assert_eq!(direct.len(), 16);
        assert_eq!(bits.position(), 0);
    }
}

/// Whether inter partition geometry permits transform_size_8x8_flag syntax.
/// Direct 4x4 entries represent inferred subpartitions and obey the SPS flag.
pub fn allows_transform8(partitions: &[Partition], direct8_inference: bool) -> bool {
    !partitions.is_empty()
        && partitions.iter().all(|p| {
            if p.prediction == Prediction::Direct {
                direct8_inference
            } else {
                p.size[0] >= 8 && p.size[1] >= 8
            }
        })
}

pub struct InterHeader {
    pub mb_type: u32,
    pub partitions: Vec<Partition>,
    pub residual: super::avc_residual_syntax::InterResidualControl,
}
pub struct InterSyntax {
    pub slice: SliceType,
    pub active_references: [u32; 2],
    pub previous_qp: i32,
    pub bit_depth: u8,
    pub chroma_array_type: u8,
    pub transform8_enabled: bool,
    pub direct8_inference: bool,
}
/// Read one non-skipped inter macroblock through mb_qp_delta. Coefficients follow
/// at the returned bit position. Intra macroblocks must use the intra path.
/// Error restores the cursor, allowing dispatch based on the same mb_type bits.
pub fn read_inter_header(bits: &mut BitReader<'_>, syntax: &InterSyntax) -> Result<InterHeader> {
    let mut input = bits.clone();
    let mb_type = input.unsigned_golomb()?;
    let partitions = read_prediction(&mut input, syntax.slice, mb_type, syntax.active_references)?;
    let allowed =
        syntax.transform8_enabled && allows_transform8(&partitions, syntax.direct8_inference);
    let residual = super::avc_residual_syntax::read_inter_control(
        &mut input,
        syntax.previous_qp,
        syntax.bit_depth,
        syntax.chroma_array_type,
        allowed,
    )?;
    *bits = input;
    Ok(InterHeader {
        mb_type,
        partitions,
        residual,
    })
}
#[cfg(test)]
mod header_tests {
    use super::*;
    #[test]
    fn inter_header_presence_and_transaction() {
        let syntax = InterSyntax {
            slice: SliceType::P,
            active_references: [1, 0],
            previous_qp: 26,
            bit_depth: 8,
            chroma_array_type: 1,
            transform8_enabled: true,
            direct8_inference: true,
        };
        // mb_type=0, MVD=(0,0), CBP code=2 => 1, transform8=1, delta=0.
        let mut b = BitReader::new(&[0xef]);
        let header = read_inter_header(&mut b, &syntax).unwrap();
        assert_eq!(header.mb_type, 0);
        assert_eq!(header.residual.pattern, 1);
        assert!(header.residual.transform8);
        assert_eq!(b.position(), 8);
        let mut b = BitReader::new(&[0xec]);
        assert!(read_inter_header(&mut b, &syntax).is_err());
        assert_eq!(b.position(), 0);
        // A P-slice intra macroblock must not consume the dispatcher cursor.
        let mut b = BitReader::new(&[0x30]);
        assert!(read_inter_header(&mut b, &syntax).is_err());
        assert_eq!(b.position(), 0);
    }
    #[test]
    fn transform8_subpartition_rules() {
        for code in 0..4 {
            let p = sub_partitions(SliceType::P, [code; 4]).unwrap();
            assert_eq!(allows_transform8(&p, true), code == 0);
        }
        for code in 0..13 {
            let p = sub_partitions(SliceType::B, [code; 4]).unwrap();
            assert_eq!(allows_transform8(&p, true), code <= 3);
            assert_eq!(allows_transform8(&p, false), (1..=3).contains(&code));
        }
    }
}
