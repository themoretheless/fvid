//! HEVC picture-order derivation shared by software and hardware decoders.
use super::{hevc_nal::NalHeader, hevc_sps::Sps};
use crate::{Result, invalid};

/// H.265 equation (5-11). The two half-range comparisons are asymmetric.
fn current_msb(lsb: i32, previous_lsb: i32, previous_msb: i32, modulus: i32) -> Result<i32> {
    if previous_lsb - lsb >= modulus / 2 {
        previous_msb.checked_add(modulus)
    } else if lsb - previous_lsb > modulus / 2 {
        previous_msb.checked_sub(modulus)
    } else {
        Some(previous_msb)
    }
    .ok_or_else(|| invalid("HEVC POC overflow"))
}

pub fn derive(sps: &Sps, nal: NalHeader, lsb: u32, previous: Option<i32>) -> Result<i32> {
    if !(4..=16).contains(&sps.poc_bits) || lsb >= (1u32 << sps.poc_bits) {
        return Err(invalid("HEVC POC configuration/LSB out of range"));
    }
    if nal.is_idr() {
        return Ok(0);
    }
    let lsb = lsb as i32;
    if matches!(nal.unit_type, 16..=18) || previous.is_none() && nal.is_irap() {
        return Ok(lsb);
    }
    let previous =
        previous.ok_or_else(|| invalid("HEVC stream must start at a random-access picture"))?;
    let modulus = 1i32 << sps.poc_bits;
    let old = previous.rem_euclid(modulus);
    let msb = current_msb(lsb, old, previous - old, modulus)?;
    msb.checked_add(lsb)
        .ok_or_else(|| invalid("HEVC POC overflow"))
}

pub fn updates_previous(nal: NalHeader) -> bool {
    nal.temporal_id == 0 && !matches!(nal.unit_type, 0 | 2 | 4 | 6..=9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_msb_matches_normative_equation_for_every_lsb_pair() {
        for bits in 4..=8 {
            let modulus = 1i32 << bits;
            for previous_msb in [-3 * modulus, 0, 5 * modulus] {
                for old in 0..modulus {
                    for lsb in 0..modulus {
                        // Independent oracle: choose the congruent POC nearest
                        // the previous POC; equal distances choose the larger POC.
                        let previous = previous_msb + old;
                        let expected = [previous_msb - modulus, previous_msb, previous_msb + modulus]
                            .into_iter()
                            .min_by_key(|&msb| ((msb + lsb - previous).abs(), -(msb + lsb)))
                            .unwrap();
                        assert_eq!(current_msb(lsb, old, previous_msb, modulus).unwrap(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn current_msb_half_range_ties_have_opposite_directions() {
        assert_eq!(current_msb(0, 8, 32, 16).unwrap(), 48);
        assert_eq!(current_msb(8, 0, 32, 16).unwrap(), 32);
        assert_eq!(current_msb(9, 0, 32, 16).unwrap(), 16);
    }

    #[test]
    fn current_msb_checks_both_signed_boundaries() {
        assert!(current_msb(0, 8, i32::MAX - 15, 16).is_err());
        assert!(current_msb(9, 0, i32::MIN, 16).is_err());
        assert_eq!(current_msb(8, 0, i32::MIN, 16).unwrap(), i32::MIN);
        assert_eq!(current_msb(0, 0, i32::MAX - 15, 16).unwrap(), i32::MAX - 15);
    }
}
