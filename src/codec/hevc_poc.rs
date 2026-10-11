//! HEVC picture-order derivation shared by software and hardware decoders.
use super::{hevc_nal::NalHeader, hevc_sps::Sps};
use crate::{Result, invalid};

/// H.265 equation (5-11). The two half-range comparisons are asymmetric.
fn current_msb(lsb: i32, previous_lsb: i32, previous_msb: i32, modulus: i32) -> Result<i32> {
    i32::try_from(current_msb_wide(lsb.into(), previous_lsb.into(), previous_msb.into(), modulus.into()))
        .map_err(|_| invalid("HEVC POC overflow"))
}
fn current_msb_wide(lsb: i64, previous_lsb: i64, previous_msb: i64, modulus: i64) -> i64 {
    previous_msb + if previous_lsb - lsb >= modulus / 2 {
        modulus
    } else if lsb - previous_lsb > modulus / 2 {
        -modulus
    } else { 0 }
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

/// Reset operation from Annex F, after the caller has established that this
/// picture starts a resetting period in its layer. Period tracking and DPB
/// mutation deliberately belong to the stateful decoder, not this calculation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    Msb,
    Full,
    Signalled { full: bool, lsb: u32 },
}

/// Equations (F-60) and (F-61): current POC and the amount to subtract from
/// retained reference POCs. `previous == None` means no picture in this layer
/// has been decoded, so there is no reference decrement. This does not admit
/// reset-coded streams by itself: callers must validate NAL/period constraints.
pub fn derive_reset(
    bits: u8,
    slice_lsb: u32,
    previous: Option<i32>,
    reset: Reset,
    msb_cycle: Option<u32>,
) -> Result<(i32, i64)> {
    if !(4..=16).contains(&bits) {
        return Err(invalid("HEVC reset POC width out of range"));
    }
    let modulus = 1i32 << bits;
    let (full, anchor) = match reset {
        Reset::Msb => (false, slice_lsb),
        Reset::Full => (true, slice_lsb),
        Reset::Signalled { full, lsb } => (full, lsb),
    };
    if slice_lsb >= modulus as u32 || anchor >= modulus as u32 {
        return Err(invalid("HEVC reset POC LSB out of range"));
    }
    let cycle_msb = msb_cycle.map(|cycle| {
        if u64::from(cycle) > (1u64 << (32 - bits)) {
            return Err(invalid("HEVC reset POC cycle out of range"));
        }
        Ok(i64::from(cycle) * i64::from(modulus))
    }).transpose()?;
    let delta = if let Some(previous) = previous {
        let old = previous.rem_euclid(modulus);
        let msb = match cycle_msb {
            Some(msb) => msb,
            None => current_msb_wide(anchor.into(), old.into(), i64::from(previous) - i64::from(old), modulus.into()),
        };
        msb.checked_add(if full { anchor.into() } else { 0 })
            .ok_or_else(|| invalid("HEVC reset POC decrement overflow"))?
    } else {
        0
    };
    let poc = match reset {
        Reset::Msb => slice_lsb as i32,
        Reset::Full => 0,
        Reset::Signalled { .. } => current_msb(
            slice_lsb as i32, if full { 0 } else { anchor as i32 }, 0, modulus,
        )?.checked_add(slice_lsb as i32)
            .ok_or_else(|| invalid("HEVC reset POC overflow"))?,
    };
    Ok((poc, delta))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_translation_preserves_reference_distances_across_wrap() {
        // Before reset: previous=30, current=33, references=30/29.
        // After MSB reset: current=1, references=-2/-3.
        let (poc, delta) = derive_reset(4, 1, Some(30), Reset::Msb, None).unwrap();
        assert_eq!((poc, delta), (1, 32));
        for reference in [30, 29, 17, -2] {
            assert_eq!(i64::from(33 - reference), i64::from(poc) - (i64::from(reference) - delta));
        }
        assert_eq!(derive_reset(4, 1, Some(30), Reset::Full, None).unwrap(), (0, 33));
    }

    #[test]
    fn signalled_reset_entry_and_explicit_cycle() {
        let partial = Reset::Signalled { full: false, lsb: 14 };
        assert_eq!(derive_reset(4, 1, Some(13), partial, None).unwrap(), (17, 0));
        assert_eq!(derive_reset(4, 1, None, partial, None).unwrap(), (17, 0));
        let full = Reset::Signalled { full: true, lsb: 14 };
        assert_eq!(derive_reset(4, 1, Some(13), full, None).unwrap(), (1, 14));
        assert_eq!(derive_reset(4, 1, Some(13), full, Some(3)).unwrap(), (1, 62));
        assert_eq!(derive_reset(4, 1, None, full, Some(3)).unwrap(), (1, 0));
    }

    #[test]
    fn reset_rejects_invalid_width_lsb_and_overflow() {
        for bits in [0, 3, 17, 32, 255] {
            assert!(derive_reset(bits, 0, None, Reset::Msb, None).is_err());
        }
        assert!(derive_reset(4, 16, None, Reset::Msb, None).is_err());
        assert!(derive_reset(4, 0, None, Reset::Signalled { full: false, lsb: 16 }, None).is_err());
        assert!(derive_reset(16, 0, Some(0), Reset::Full, Some(65537)).is_err());
        // Delta itself is not a signed 32-bit POC. Retained references can
        // remain representable after a decrement larger than i32::MAX.
        assert_eq!(derive_reset(16, 0, Some(0), Reset::Full, Some(32768)).unwrap(), (0, 2147483648));
        assert_eq!(derive_reset(4, 0, Some(i32::MAX), Reset::Msb, None).unwrap(), (0, 2147483648));
    }

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
