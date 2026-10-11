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

/// Local layer context for Annex F.7.3.6.1. Cross-layer access-unit reset
/// agreement still belongs to the stateful decoder.
#[derive(Clone, Copy, Debug)]
pub struct ResetContext {
    pub reset_info_present: bool,
    pub poc_bits: u8,
    pub nal: NalHeader,
    pub lsb_aligned: bool,
    pub direct_reference_layers: usize,
    pub discardable: bool,
    pub poc_lsb_absent: bool,
    pub slice_lsb: u32,
}

/// Annex F.7.3.6.1 syntax for a resolved layer. The byte slice is the
/// declared header-extension payload only, never CABAC or byte-alignment data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetSyntax {
    pub reset: Option<Reset>,
    pub period: Option<u8>,
    pub msb_cycle: Option<u32>,
}
impl ResetSyntax {
    pub fn parse_base(
        extension: &[u8],
        reset_info_present: bool,
        poc_bits: u8,
        nal: NalHeader,
        lsb_aligned: bool,
    ) -> Result<Self> {
        nal.require_base_layer()?;
        Self::parse_context(
            extension,
            ResetContext {
                reset_info_present,
                poc_bits,
                nal,
                lsb_aligned,
                direct_reference_layers: 0,
                discardable: false,
                poc_lsb_absent: false,
                slice_lsb: 0,
            },
        )
    }
    pub fn parse_context(extension: &[u8], context: ResetContext) -> Result<Self> {
        let ResetContext {
            reset_info_present,
            poc_bits,
            nal,
            lsb_aligned,
            direct_reference_layers,
            discardable,
            poc_lsb_absent,
            slice_lsb,
        } = context;
        if direct_reference_layers > 62
            || (nal.layer_id == 0 && direct_reference_layers != 0)
            || slice_lsb >= (1u32 << poc_bits.min(31))
        {
            return Err(invalid("invalid HEVC POC reset layer context"));
        }
        if !nal.is_vcl() || !(4..=16).contains(&poc_bits) {
            return Err(invalid("invalid HEVC POC reset slice context"));
        }
        let mut b = super::bits::BitReader::new(extension);
        let idc = if reset_info_present { b.read(2)? } else { 0 };
        let period = if idc != 0 {
            Some(b.read(6)? as u8)
        } else {
            None
        };
        if matches!(idc, 1 | 2) && (!updates_previous(nal) || discardable) {
            return Err(invalid(
                "HEVC POC reset requires a reference picture at temporal ID zero",
            ));
        }
        if idc == 2 && nal.layer_id == 0 && !nal.is_idr() {
            return Err(invalid(
                "HEVC internal base-layer full POC reset requires IDR",
            ));
        }
        if idc == 3 && matches!(nal.unit_type, 16..=18 | 21) {
            return Err(invalid("HEVC CRA/BLA cannot use signalled POC reset"));
        }
        if idc == 2 && poc_lsb_absent && slice_lsb != 0 {
            return Err(invalid(
                "HEVC full POC reset has nonzero inferred slice LSB",
            ));
        }
        let reset = match idc {
            0 => None,
            1 => Some(Reset::Msb),
            2 => Some(Reset::Full),
            _ => Some(Reset::Signalled {
                full: b.bit()?,
                lsb: b.read(poc_bits)?,
            }),
        };
        // Equation F-55: an aligned dependent CRA/BLA reads a presence flag;
        // an independent or unaligned CRA/BLA infers presence instead.
        let msb_required =
            matches!(nal.unit_type, 16..=18 | 21) && (!lsb_aligned || direct_reference_layers == 0);
        let cycle_present = if extension.is_empty() {
            false
        } else if msb_required {
            true
        } else if lsb_aligned {
            b.bit()?
        } else {
            false
        };
        let msb_cycle = if cycle_present {
            let cycle = b.unsigned_golomb()?;
            if u64::from(cycle) > (1u64 << (32 - poc_bits)) {
                return Err(invalid("HEVC reset POC cycle out of range"));
            }
            Some(cycle)
        } else {
            None
        };
        // Remaining extension_data_bit values are reserved at this syntax boundary.
        Ok(Self {
            reset,
            period,
            msb_cycle,
        })
    }
}

/// Per-layer Annex F resetting-period state. `advance` returns a candidate
/// state so a decoder can publish it only after the complete picture succeeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetPeriods {
    seen_picture: bool,
    period: u8,
    previous_initial_reset: bool,
    initial_reset: Option<(bool, u32)>,
    initial_is_previous_t0: bool,
}
impl ResetPeriods {
    pub fn advance(self, syntax: ResetSyntax, nal: NalHeader, slice_lsb: u32) -> Result<(Self, bool)> {
        if syntax.period.is_some() != syntax.reset.is_some()
            || syntax.period.is_some_and(|p| p > 63) {
            return Err(invalid("inconsistent HEVC POC reset period syntax"));
        }
        let period = syntax.period.unwrap_or(self.period);
        let initial = matches!(syntax.reset, Some(Reset::Msb | Reset::Full));
        if initial && self.previous_initial_reset && period == self.period {
            return Err(invalid("consecutive HEVC initial resets reuse a period ID"));
        }
        // An idc 1/2 picture starts a new period even if a nonconsecutive
        // earlier period used the same six-bit ID (F.7.4.7.1 note 5).
        let resetting = initial || syntax.reset.is_some()
            && (!self.seen_picture || period != self.period);
        let mut next = self;
        if resetting {
            next.initial_reset = None;
            next.initial_is_previous_t0 = false;
        }
        if let Some(Reset::Signalled { full, lsb }) = syntax.reset {
            if let Some((anchor_full, anchor_lsb)) = next.initial_reset {
                if full != anchor_full || lsb != anchor_lsb || !next.initial_is_previous_t0 {
                    return Err(invalid("HEVC signalled reset disagrees with period anchor"));
                }
            }
        }
        if initial {
            next.initial_reset = Some((syntax.reset == Some(Reset::Full), slice_lsb));
            next.initial_is_previous_t0 = true;
        } else if updates_previous(nal) {
            next.initial_is_previous_t0 = false;
        }
        next.seen_picture = true;
        next.period = period;
        next.previous_initial_reset = initial;
        Ok((next, resetting))
    }
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

    fn nal(unit_type: u8) -> NalHeader {
        NalHeader { unit_type, layer_id: 0, temporal_id: 0 }
    }

    fn reset_syntax(reset: Reset, period: u8) -> ResetSyntax {
        ResetSyntax { reset: Some(reset), period: Some(period), msb_cycle: None }
    }

    #[test]
    fn reset_period_decrements_once_and_infers_missing_period_id() {
        let start = reset_syntax(Reset::Msb, 7);
        let (state, resetting) = ResetPeriods::default().advance(start, nal(1), 14).unwrap();
        assert!(resetting);
        let signalled = reset_syntax(Reset::Signalled { full: false, lsb: 14 }, 7);
        let mut higher = nal(1);
        higher.temporal_id = 1;
        let (state, resetting) = state.advance(signalled, higher, 1).unwrap();
        assert!(!resetting);
        let (state, resetting) = state.advance(ResetSyntax::default(), higher, 2).unwrap();
        assert!(!resetting);
        assert_eq!(state.period, 7);
        let (_, resetting) = state.advance(signalled, higher, 3).unwrap();
        assert!(!resetting);
    }

    #[test]
    fn reset_period_random_access_entry_and_id_reuse() {
        let initial = reset_syntax(Reset::Full, 63);
        let (state, _) = ResetPeriods::default().advance(initial, nal(19), 0).unwrap();
        assert!(state.advance(initial, nal(19), 0).is_err());
        let (state, resetting) = state.advance(ResetSyntax::default(), nal(1), 1).unwrap();
        assert!(!resetting);
        assert!(state.advance(initial, nal(19), 0).unwrap().1);
        let entry = reset_syntax(Reset::Signalled { full: true, lsb: 4 }, 0);
        let (state, resetting) = ResetPeriods::default().advance(entry, nal(1), 5).unwrap();
        assert!(resetting, "first extracted picture resets even with inferred initial ID zero");
        assert!(!state.advance(entry, nal(1), 6).unwrap().1);
        assert!(state.advance(reset_syntax(Reset::Signalled { full: true, lsb: 4 }, 1), nal(1), 7).unwrap().1);
    }

    #[test]
    fn reset_period_rejects_anchor_mismatch_without_changing_source() {
        let (state, _) = ResetPeriods::default().advance(reset_syntax(Reset::Msb, 2), nal(1), 7).unwrap();
        let saved = state;
        for reset in [Reset::Signalled { full: true, lsb: 7 }, Reset::Signalled { full: false, lsb: 6 }] {
            assert!(state.advance(reset_syntax(reset, 2), nal(1), 8).is_err());
            assert_eq!(state, saved);
        }
        let (state, _) = state.advance(ResetSyntax::default(), nal(1), 8).unwrap();
        assert!(state.advance(reset_syntax(Reset::Signalled { full: false, lsb: 7 }, 2), nal(1), 9).is_err());
        assert!(ResetPeriods::default().advance(ResetSyntax { period: Some(1), ..ResetSyntax::default() }, nal(1), 0).is_err());
    }

    #[test]
    fn reset_extension_decodes_all_operations_and_keeps_future_tail() {
        let parse = |bytes: &[u8], ty| ResetSyntax::parse_base(bytes, true, 4, nal(ty), false).unwrap();
        assert_eq!(parse(&[0x41], 1), ResetSyntax { reset: Some(Reset::Msb), period: Some(1), msb_cycle: None });
        assert_eq!(parse(&[0x81], 19).reset, Some(Reset::Full));
        assert_eq!(parse(&[0xc2, 0xf0], 1).reset, Some(Reset::Signalled { full: true, lsb: 14 }));
        assert_eq!(parse(&[0xc2, 0x70], 1).reset, Some(Reset::Signalled { full: false, lsb: 14 }));
        assert_eq!(parse(&[0xc2, 0xf7, 0xff], 1), parse(&[0xc2, 0xf0], 1));
        assert_eq!(parse(&[0], 1), ResetSyntax::default());
    }

    #[test]
    fn reset_extension_infers_and_reads_cycle_presence() {
        assert_eq!(ResetSyntax::parse_base(&[0x41, 0x20], true, 4, nal(21), false).unwrap().msb_cycle, Some(3));
        assert_eq!(ResetSyntax::parse_base(&[0x24], true, 4, nal(1), true).unwrap().msb_cycle, Some(3));
        assert_eq!(ResetSyntax::parse_base(&[], false, 4, nal(21), false).unwrap(), ResetSyntax::default());
        assert_eq!(ResetSyntax::parse_base(&[0x20], false, 4, nal(21), false).unwrap().msb_cycle, Some(3));
    }

    #[test]
    fn reset_extension_never_reads_beyond_declared_payload() {
        for (bytes, ty) in [(&[][..], 1), (&[0xc2][..], 1), (&[0x41][..], 21), (&[0x20][..], 1)] {
            let aligned = bytes == [0x20];
            assert!(ResetSyntax::parse_base(bytes, true, 16, nal(ty), aligned).is_err());
        }
    }

    #[test]
    fn reset_extension_enforces_base_layer_nal_constraints() {
        for ty in [0, 2, 4, 6, 7, 8, 9] {
            assert!(ResetSyntax::parse_base(&[0x41], true, 4, nal(ty), false).is_err());
        }
        assert!(ResetSyntax::parse_base(&[0x81], true, 4, nal(1), false).is_err());
        for ty in [16, 17, 18, 21] {
            assert!(ResetSyntax::parse_base(&[0xc2, 0xf0], true, 4, nal(ty), false).is_err());
        }
        let mut temporal = nal(1);
        temporal.temporal_id = 1;
        assert!(ResetSyntax::parse_base(&[0x41], true, 4, temporal, false).is_err());
    }

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
