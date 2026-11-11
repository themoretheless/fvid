//! CAVLC residual control syntax shared by intra and inter macroblocks.
use super::bits::BitReader;
use crate::{Result, invalid};
const INTRA: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];
const INTER: [u8; 48] = [
    0, 16, 1, 2, 4, 8, 32, 3, 5, 10, 12, 15, 47, 7, 11, 13, 14, 6, 9, 31, 35, 37, 42, 44, 33, 34,
    36, 40, 39, 43, 45, 46, 17, 18, 20, 24, 19, 21, 26, 28, 23, 27, 29, 30, 22, 25, 38, 41,
];
/// Table 9-4, including monochrome/separate-plane versus chroma DC/AC mapping.
pub fn coded_block_pattern(code: u32, intra: bool, chroma_array_type: u8) -> Result<u8> {
    let table: &[u8] = match (intra, chroma_array_type) {
        (true, 1 | 2) => &INTRA,
        (false, 1 | 2) => &INTER,
        (true, 0 | 3) => &[15, 0, 7, 11, 13, 14, 3, 5, 10, 12, 1, 2, 4, 8, 6, 9],
        (false, 0 | 3) => &[0, 1, 2, 4, 8, 3, 5, 10, 12, 15, 7, 11, 13, 14, 6, 9],
        _ => return Err(invalid("invalid AVC chroma array type")),
    };
    table
        .get(code as usize)
        .copied()
        .ok_or_else(|| invalid("AVC coded block pattern out of range"))
}
pub fn update_qp(previous: i32, delta: i32, depth: u8) -> Result<i32> {
    if !(8..=14).contains(&depth) {
        return Err(invalid("invalid AVC QP bit depth"));
    }
    let offset = 6 * (i32::from(depth) - 8);
    if !(-offset..=51).contains(&previous)
        || !(-(26 + offset / 2)..=25 + offset / 2).contains(&delta)
    {
        return Err(invalid("AVC QP or mb_qp_delta out of range"));
    }
    Ok((previous + delta + 52 + 2 * offset) % (52 + offset) - offset)
}
#[derive(Debug)]
pub struct InterResidualControl {
    pub pattern: u8,
    pub transform8: bool,
    pub qp: i32,
}
/// The caller computes transform8_allowed from PPS, subpartition sizes and
/// direct_8x8_inference_flag. Skipped macroblocks do not invoke this parser.
pub fn read_inter_control(
    bits: &mut BitReader<'_>,
    previous_qp: i32,
    depth: u8,
    chroma_array_type: u8,
    transform8_allowed: bool,
) -> Result<InterResidualControl> {
    let mut input = bits.clone();
    update_qp(previous_qp, 0, depth)?;
    let pattern = coded_block_pattern(input.unsigned_golomb()?, false, chroma_array_type)?;
    let transform8 = pattern & 15 != 0 && transform8_allowed && input.bit()?;
    let qp = if pattern != 0 {
        update_qp(previous_qp, input.signed_golomb()?, depth)?
    } else {
        previous_qp
    };
    *bits = input;
    Ok(InterResidualControl {
        pattern,
        transform8,
        qp,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mappings_and_qp_wrap_all_depths() {
        for intra in [false, true] {
            for chroma in 0..4 {
                let count = if chroma == 1 || chroma == 2 { 48 } else { 16 };
                let mut values: Vec<_> = (0..count)
                    .map(|n| coded_block_pattern(n, intra, chroma).unwrap())
                    .collect();
                values.sort();
                assert_eq!(values, (0..count as u8).collect::<Vec<_>>());
                assert!(coded_block_pattern(count, intra, chroma).is_err());
            }
        }
        for depth in 8..=14 {
            let min = -6 * (i32::from(depth) - 8);
            assert_eq!(update_qp(51, 1, depth).unwrap(), min);
            assert_eq!(update_qp(min, -1, depth).unwrap(), 51);
        }
    }
    #[test]
    fn inter_control_presence_and_error_rollback() {
        let mut b = BitReader::new(&[0x80]);
        let c = read_inter_control(&mut b, 26, 8, 1, true).unwrap();
        assert_eq!(c.pattern, 0);
        assert!(!c.transform8);
        assert_eq!(b.position(), 1);
        // ue(2)=011 => CBP=1; transform8=1; se(0)=1.
        let mut b = BitReader::new(&[0x78]);
        let c = read_inter_control(&mut b, 26, 8, 1, true).unwrap();
        assert_eq!(c.pattern, 1);
        assert!(c.transform8);
        assert_eq!(c.qp, 26);
        assert_eq!(b.position(), 5);
        let mut b = BitReader::new(&[0x60]);
        assert!(read_inter_control(&mut b, 26, 8, 1, true).is_err());
        assert_eq!(b.position(), 0);
    }
}
