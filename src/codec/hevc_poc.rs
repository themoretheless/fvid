//! HEVC picture-order derivation shared by software and hardware decoders.
use super::{hevc_nal::NalHeader, hevc_sps::Sps};
use crate::{Result, invalid};

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
    let mut msb = previous - old;
    if lsb < old && old - lsb >= modulus / 2 {
        msb = msb
            .checked_add(modulus)
            .ok_or_else(|| invalid("HEVC POC overflow"))?;
    } else if lsb > old && lsb - old > modulus / 2 {
        msb = msb
            .checked_sub(modulus)
            .ok_or_else(|| invalid("HEVC POC overflow"))?;
    }
    msb.checked_add(lsb)
        .ok_or_else(|| invalid("HEVC POC overflow"))
}

pub fn updates_previous(nal: NalHeader) -> bool {
    nal.temporal_id == 0 && !matches!(nal.unit_type, 0 | 2 | 4 | 6..=9)
}
