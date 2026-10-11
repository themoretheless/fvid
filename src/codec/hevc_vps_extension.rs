//! Annex F.7.3.2.1.1 for an internal, single-layer VPS.
//! Invoked only when Annex F semantics are needed; Annex A keeps ignoring tails.
use super::{bits::BitReader, hevc_profile::ProfileTierLevel};
use crate::{invalid, Result};

fn bounded(b: &mut BitReader<'_>, maximum: u32) -> Result<u32> {
    let value = b.unsigned_golomb()?;
    if value > maximum {
        return Err(invalid("HEVC VPS extension value out of range"));
    }
    Ok(value)
}

pub(super) fn poc_lsb_aligned(bytes: &[u8], start: usize, sublayers: u8) -> Result<bool> {
    let mut b = BitReader::new(bytes);
    b.skip(start)?;
    while b.position() % 8 != 0 {
        if !b.bit()? {
            return Err(invalid("HEVC VPS extension alignment bit is not one"));
        }
    }
    // MaxLayersMinus1 == 0: no extra PTL, layer IDs, dependencies or OLSs.
    let splitting = b.bit()?;
    let mut count = 0u8;
    for _ in 0..16 {
        count += u8::from(b.bit()?);
    }
    if splitting && count == 0 {
        return Err(invalid("HEVC VPS splitting has no scalability dimensions"));
    }
    let mut offset = 0u32;
    for _ in 0..count - u8::from(splitting) {
        offset += b.read(3)? + 1;
    }
    if splitting && offset >= 6 {
        return Err(invalid("HEVC VPS split dimensions exceed layer ID"));
    }
    b.bit()?; // vps_nuh_layer_id_present_flag: no additional layer IDs
    let view_bits = b.read(4)? as u8;
    b.read(view_bits)?; // NumViews == 1
    if b.bit()? && b.read(3)? > u32::from(sublayers) {
        return Err(invalid("HEVC VPS extension exceeds temporal sublayers"));
    }
    b.bit()?; // max_tid_ref_present_flag: no dependency edges
    b.bit()?; // default_ref_layers_active_flag
    let profiles = bounded(&mut b, 63)?;
    for _ in 2..=profiles {
        let present = b.bit()?;
        ProfileTierLevel::read(&mut b, present, sublayers)?;
    }
    let formats = bounded(&mut b, 255)?;
    let mut previous = None;
    for index in 0..=formats {
        let width = b.read(16)?;
        let height = b.read(16)?;
        if width == 0 || height == 0 {
            return Err(invalid("zero HEVC VPS representation dimensions"));
        }
        let format = if b.bit()? {
            let chroma = b.read(2)?;
            let separate = chroma == 3 && b.bit()?;
            let luma = b.read(4)?;
            let chroma_depth = b.read(4)?;
            if luma > 8 || chroma_depth > 8 {
                return Err(invalid("HEVC VPS representation depth exceeds 16 bits"));
            }
            (chroma, separate)
        } else {
            previous
                .ok_or_else(|| invalid("first HEVC VPS representation format lacks chroma/depth"))?
        };
        previous = Some(format);
        if b.bit()? {
            let left = u64::from(b.unsigned_golomb()?);
            let right = u64::from(b.unsigned_golomb()?);
            let top = u64::from(b.unsigned_golomb()?);
            let bottom = u64::from(b.unsigned_golomb()?);
            let chroma = if format.1 { 0 } else { format.0 };
            let sx = if matches!(chroma, 1 | 2) { 2 } else { 1 };
            let sy = if chroma == 1 { 2 } else { 1 };
            if (left + right) * sx >= u64::from(width) || (top + bottom) * sy >= u64::from(height) {
                return Err(invalid("HEVC VPS representation crop removes picture"));
            }
        }
        debug_assert!(index <= 255);
    }
    if formats > 0 {
        b.bit()?;
    } // rep_format_idx_present_flag: base index inferred zero
    b.bit()?; // max_one_active_ref_layer_flag
    let aligned = b.bit()?;
    // No poc_lsb_not_present flags or additional-layer DPB entries.
    let dependency_width = bounded(&mut b, 30)? as u8 + 2;
    if b.bit()? && b.read(dependency_width)? == u32::MAX {
        return Err(invalid("invalid HEVC VPS direct dependency type"));
    }
    let ignored_bytes = bounded(&mut b, 4096)? as usize;
    b.skip(ignored_bytes * 8)?;
    if b.bit()? {
        return Err(crate::unsupported(
            "HEVC POC reset VPS VUI semantics are not implemented",
        ));
    }
    if b.bit()? {
        while b.more_rbsp_data() {
            b.bit()?;
        }
    }
    b.finish_rbsp()?;
    Ok(aligned)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ue(n: u32) -> String {
        let value = format!("{:b}", u64::from(n) + 1);
        format!("{}{}", "0".repeat(value.len() - 1), value)
    }
    fn packed(mut value: String) -> Vec<u8> {
        while value.len() % 8 != 0 {
            value.push('0');
        }
        value
            .as_bytes()
            .chunks(8)
            .map(|v| v.iter().fold(0, |a, c| (a << 1) | u8::from(*c == b'1')))
            .collect()
    }
    fn syntax(aligned: bool) -> String {
        // Three prefix bits, five alignment ones; no extra layers/OLSs/PTLs.
        format!(
            "00011111{}0000{}{:016b}{:016b}1010000000000{}101001",
            "0".repeat(18),
            "00011",
            64,
            64,
            u8::from(aligned)
        )
    }
    #[test]
    fn single_layer_alignment_and_every_byte_truncation() {
        for aligned in [false, true] {
            let data = packed(syntax(aligned));
            assert_eq!(poc_lsb_aligned(&data, 3, 0).unwrap(), aligned);
            for end in 0..data.len() {
                assert!(poc_lsb_aligned(&data[..end], 3, 0).is_err(), "{end}");
            }
            let mut bad = data.clone();
            bad[0] &= !0x10;
            assert_eq!(
                poc_lsb_aligned(&bad, 3, 0).unwrap_err().to_string(),
                "HEVC VPS extension alignment bit is not one"
            );
        }
    }
    #[test]
    fn bounded_extension_counts_and_format_validation() {
        let prefix = format!("00011111{}0000000", "0".repeat(18));
        // Prefix ends before profile count. Larger-than-normative bounded counts
        // are Invalid, without allocating based on the signalled size.
        for (tail, message) in [
            (ue(64), "HEVC VPS extension value out of range"),
            (ue(0) + &ue(256), "HEVC VPS extension value out of range"),
            (
                ue(0) + &ue(0) + &format!("{:016b}{:016b}", 0, 64),
                "zero HEVC VPS representation dimensions",
            ),
            (
                ue(0) + &ue(0) + &format!("{:016b}{:016b}0", 64, 64),
                "first HEVC VPS representation format lacks chroma/depth",
            ),
        ] {
            let data = packed(prefix.clone() + &tail + "1");
            assert_eq!(
                poc_lsb_aligned(&data, 3, 0).unwrap_err().to_string(),
                message
            );
        }
        let split = packed("000111111".to_owned() + &"0".repeat(16) + "1");
        assert_eq!(
            poc_lsb_aligned(&split, 3, 0).unwrap_err().to_string(),
            "HEVC VPS splitting has no scalability dimensions"
        );
    }
    #[test]
    fn representation_inference_crop_depth_dependency_and_future_tail() {
        let original = syntax(true);
        let splice = |at: usize, remove: usize, value: &str| {
            packed(original[..at].to_owned() + value + &original[at + remove..])
        };
        for (data, message) in [
            (
                splice(70, 4, "1001"),
                "HEVC VPS representation depth exceeds 16 bits",
            ),
            (
                splice(78, 1, &("1".to_owned() + &ue(32) + &ue(0).repeat(3))),
                "HEVC VPS representation crop removes picture",
            ),
            (
                splice(81, 1, &ue(31)),
                "HEVC VPS extension value out of range",
            ),
            (
                splice(83, 1, &ue(4097)),
                "HEVC VPS extension value out of range",
            ),
        ] {
            assert_eq!(
                poc_lsb_aligned(&data, 3, 0).unwrap_err().to_string(),
                message
            );
        }
        assert!(matches!(
            poc_lsb_aligned(&splice(84, 1, "1"), 3, 0),
            Err(crate::Error::Unsupported(_))
        ));
        assert!(poc_lsb_aligned(&splice(85, 1, "110110"), 3, 0).unwrap());
        assert!(poc_lsb_aligned(&splice(82, 1, "110"), 3, 0).unwrap());
        // Maximum 256 formats; later formats inherit chroma/depth, and the base
        // layer's representation index is inferred zero (no index bits).
        let mut many = original[..34].to_owned() + &ue(255) + &original[35..79];
        for _ in 1..256 {
            many += &format!("{:016b}{:016b}00", 64, 64);
        }
        many += "0"; // rep_format_idx_present_flag
        many += &original[79..];
        assert!(poc_lsb_aligned(&packed(many), 3, 0).unwrap());
    }
}
