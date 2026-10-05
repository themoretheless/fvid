//! H.265 7.3.6.1/7.4.7.1 long-term RPS syntax and 8.3.2 POC derivation.
//! Syntax support alone does not qualify long-term picture decoding.
use super::bits::BitReader;
use crate::{Result, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongTermReference {
    pub poc_lsb: u32,
    pub used: bool,
    /// Accumulated cycles; None requires an unambiguous DPB LSB lookup.
    pub msb_cycles: Option<u64>,
}
impl LongTermReference {
    pub fn full_poc(&self, current: i32, poc_bits: u8) -> Result<Option<i32>> {
        if !(4..=16).contains(&poc_bits) || self.poc_lsb >= 1u32 << poc_bits {
            return Err(invalid("invalid HEVC long-term POC geometry"));
        }
        let Some(cycles) = self.msb_cycles else {
            return Ok(None);
        };
        let modulus = 1i64 << poc_bits;
        let distance = i64::try_from(cycles)
            .ok()
            .and_then(|v| v.checked_mul(modulus))
            .ok_or_else(|| invalid("HEVC long-term POC overflow"))?;
        let base = i64::from(current) - i64::from(current).rem_euclid(modulus);
        let value = base
            .checked_sub(distance)
            .and_then(|v| v.checked_add(i64::from(self.poc_lsb)))
            .ok_or_else(|| invalid("HEVC long-term POC overflow"))?;
        Ok(Some(
            i32::try_from(value).map_err(|_| invalid("HEVC long-term POC overflow"))?,
        ))
    }
}
fn count(b: &mut BitReader<'_>, maximum: u32) -> Result<u32> {
    let value = b.unsigned_golomb()?;
    if value > maximum {
        return Err(invalid("HEVC long-term syntax exceeds range"));
    }
    Ok(value)
}
pub fn read_long_term(
    b: &mut BitReader<'_>,
    sps: &[(u16, bool)],
    poc_bits: u8,
    max_references: u8,
) -> Result<Vec<LongTermReference>> {
    if !(4..=16).contains(&poc_bits) || sps.len() > 32 {
        return Err(invalid("invalid HEVC long-term SPS geometry"));
    }
    let from_sps = if sps.is_empty() {
        0
    } else {
        count(b, sps.len() as u32)?
    };
    if from_sps > u32::from(max_references) {
        return Err(invalid("HEVC long-term reference count exceeds DPB"));
    }
    let explicit = count(b, u32::from(max_references) - from_sps)?;
    let mut references = Vec::new();
    let mut accumulated = 0u64;
    for i in 0..from_sps + explicit {
        let (poc_lsb, used) = if i < from_sps {
            let index = if sps.len() > 1 {
                b.read((usize::BITS - (sps.len() - 1).leading_zeros()) as u8)? as usize
            } else {
                0
            };
            let &(poc, used) = sps
                .get(index)
                .ok_or_else(|| invalid("HEVC long-term SPS index out of range"))?;
            (u32::from(poc), used)
        } else {
            (b.read(poc_bits)?, b.bit()?)
        };
        if poc_lsb >= 1u32 << poc_bits {
            return Err(invalid("invalid HEVC long-term SPS POC"));
        }
        let present = b.bit()?;
        let cycles = if present {
            u64::from(count(b, 1u32 << (32 - poc_bits))?)
        } else {
            0
        };
        // The accumulator resets at the SPS/explicit boundary, even when the
        // first explicit entry omits its MSB syntax (inferred delta is zero).
        accumulated = if i == 0 || i == from_sps {
            cycles
        } else {
            accumulated
                .checked_add(cycles)
                .ok_or_else(|| invalid("HEVC long-term cycle overflow"))?
        };
        references.push(LongTermReference {
            poc_lsb,
            used,
            msb_cycles: present.then_some(accumulated),
        });
    }
    Ok(references)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(value: &str) -> Vec<u8> {
        let mut bytes = vec![0; value.len().div_ceil(8)];
        for (i, value) in value.bytes().enumerate() {
            assert!(matches!(value, b'0' | b'1'));
            bytes[i / 8] |= (value - b'0') << (7 - i % 8);
        }
        bytes
    }
    #[test]
    fn empty_sps_and_explicit_lsb_only_reference() {
        let empty = bits("1");
        assert!(
            read_long_term(&mut BitReader::new(&empty), &[], 4, 15)
                .unwrap()
                .is_empty()
        );
        // num_long_term_pics=1, lsb=3, used=1, MSB absent.
        let data = bits("010001110");
        let refs = read_long_term(&mut BitReader::new(&data), &[], 4, 15).unwrap();
        assert_eq!(
            refs,
            vec![LongTermReference {
                poc_lsb: 3,
                used: true,
                msb_cycles: None
            }]
        );
        assert_eq!(refs[0].full_poc(35, 4).unwrap(), None);
    }
    #[test]
    fn cycles_accumulate_but_reset_between_sps_and_explicit_entries() {
        // SPS count=2, explicit count=1; both SPS entries select index 0.
        // SPS cycle deltas 1,2 accumulate to 1,3. Explicit delta 0 resets to 0.
        let data = bits("01101001010010110011111");
        let refs =
            read_long_term(&mut BitReader::new(&data), &[(3, true), (7, false)], 4, 15).unwrap();
        assert_eq!(
            refs.iter().map(|r| r.msb_cycles).collect::<Vec<_>>(),
            vec![Some(1), Some(3), Some(0)]
        );
        assert_eq!(refs[0].full_poc(35, 4).unwrap(), Some(19));
        assert_eq!(refs[1].full_poc(35, 4).unwrap(), Some(-13));
        assert_eq!(refs[2].full_poc(35, 4).unwrap(), Some(35));
    }
    #[test]
    fn malformed_geometry_counts_indexes_and_poc_overflow_are_errors() {
        let data = bits("010");
        assert!(read_long_term(&mut BitReader::new(&data), &[], 4, 0).is_err());
        assert!(read_long_term(&mut BitReader::new(&data), &[], 3, 15).is_err());
        // one SPS reference, zero explicit, index 3 into a three-entry SPS.
        let data = bits("010111");
        assert!(read_long_term(&mut BitReader::new(&data), &[(0, true); 3], 4, 15).is_err());
        let reference = LongTermReference {
            poc_lsb: 0,
            used: true,
            msb_cycles: Some(u64::MAX),
        };
        assert!(reference.full_poc(0, 16).is_err());
        let reference = LongTermReference {
            poc_lsb: 0,
            used: true,
            msb_cycles: Some(i64::MAX as u64 / 16),
        };
        assert!(reference.full_poc(i32::MIN, 4).is_err());
    }
}
