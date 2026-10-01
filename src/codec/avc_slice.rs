//! AVC slice-header syntax. Decoded coefficients and picture reconstruction are separate.
use super::{
    avc::{PictureOrder, Pps, SliceGroups, Sps},
    bits::{BitReader, unescape_rbsp},
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceType {
    P,
    B,
    I,
    Sp,
    Si,
}
#[derive(Debug, PartialEq, Eq)]
pub enum RefModification {
    Subtract(u32),
    Add(u32),
    LongTerm(u32),
}
#[derive(Debug, PartialEq, Eq)]
pub enum MemoryOperation {
    ForgetShort(u32),
    ForgetLong(u32),
    ShortToLong { difference: u32, index: u32 },
    LimitLong(u32),
    Reset,
    CurrentLong(u32),
}
#[derive(Debug)]
pub struct Weight {
    pub luma: (i16, i16),
    pub chroma: [(i16, i16); 2],
}
#[derive(Debug)]
pub struct Weights {
    pub luma_denom: u8,
    pub chroma_denom: u8,
    pub l0: Vec<Weight>,
    pub l1: Vec<Weight>,
}
#[derive(Debug)]
pub struct SliceHeader {
    pub nal_ref_idc: u8,
    pub idr: bool,
    pub first_mb: u32,
    pub slice_type: SliceType,
    pub all_same_type: bool,
    pub pps_id: u32,
    pub colour_plane_id: u8,
    pub frame_num: u32,
    pub field_pic: bool,
    pub bottom_field: bool,
    pub idr_pic_id: Option<u32>,
    pub poc_lsb: Option<u32>,
    pub delta_poc_bottom: i32,
    pub delta_poc: [i32; 2],
    pub redundant_pic_cnt: u32,
    pub direct_spatial_mv_pred: bool,
    pub refs_l0: u32,
    pub refs_l1: u32,
    pub modifications_l0: Vec<RefModification>,
    pub modifications_l1: Vec<RefModification>,
    pub weights: Option<Weights>,
    pub no_output_of_prior_pics: bool,
    pub long_term_reference: bool,
    pub adaptive_reference_marking: bool,
    pub memory_operations: Vec<MemoryOperation>,
    pub cabac_init_idc: u32,
    pub slice_qp: i32,
    pub sp_for_switch: bool,
    pub slice_qs: Option<i32>,
    pub disable_deblocking_filter_idc: u32,
    pub alpha_offset: i32,
    pub beta_offset: i32,
    pub slice_group_change_cycle: Option<u32>,
    /// RBSP excludes the one-byte NAL header; entropy bit offset is relative to this buffer.
    pub rbsp: Vec<u8>,
    pub header_bits: usize,
    pub entropy_bit_offset: usize,
}
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let n = b.unsigned_golomb()?;
    if n > max {
        return Err(invalid("AVC slice value out of range"));
    }
    Ok(n)
}
fn modifications(b: &mut BitReader<'_>) -> Result<Vec<RefModification>> {
    let mut out = Vec::new();
    if !b.bit()? {
        return Ok(out);
    }
    loop {
        let command = ue(b, 3)?;
        if command == 3 {
            return Ok(out);
        }
        if out.len() >= 64 {
            return Err(invalid("too many AVC reference modifications"));
        }
        let value = b.unsigned_golomb()?;
        out.push(match command {
            0 => RefModification::Subtract(value),
            1 => RefModification::Add(value),
            _ => RefModification::LongTerm(value),
        });
    }
}
fn weight_value(b: &mut BitReader<'_>) -> Result<i16> {
    let n = b.signed_golomb()?;
    if !(-128..=127).contains(&n) {
        return Err(invalid("AVC weight out of range"));
    }
    Ok(n as i16)
}
fn weight_list(
    b: &mut BitReader<'_>,
    count: u32,
    luma: u8,
    chroma: u8,
    has_chroma: bool,
) -> Result<Vec<Weight>> {
    let mut result = Vec::new();
    for _ in 0..count {
        let luma_weight = if b.bit()? {
            (weight_value(b)?, weight_value(b)?)
        } else {
            (1i16 << luma, 0)
        };
        let mut chroma_weight = [(1i16 << chroma, 0); 2];
        if has_chroma && b.bit()? {
            for w in &mut chroma_weight {
                *w = (weight_value(b)?, weight_value(b)?);
            }
        }
        result.push(Weight {
            luma: luma_weight,
            chroma: chroma_weight,
        });
    }
    Ok(result)
}
impl SliceHeader {
    /// Retrieve the PPS ID before choosing the referenced PPS/SPS pair.
    pub fn parameter_set_id(nal: &[u8]) -> Result<u32> {
        if nal
            .first()
            .is_none_or(|h| h & 0x80 != 0 || !matches!(h & 31, 1 | 5))
        {
            return Err(invalid("expected AVC coded slice NAL"));
        }
        if nal.len() > 32 << 20 {
            return Err(invalid("AVC slice exceeds 32 MiB"));
        }
        let rbsp = unescape_rbsp(&nal[1..])?;
        let mut b = BitReader::new(&rbsp);
        b.unsigned_golomb()?;
        ue(&mut b, 9)?;
        ue(&mut b, 255)
    }
    pub fn parse(nal: &[u8], sps: &Sps, pps: &Pps) -> Result<Self> {
        if nal
            .first()
            .is_none_or(|h| h & 0x80 != 0 || !matches!(h & 31, 1 | 5))
        {
            return Err(invalid("expected AVC coded slice NAL"));
        }
        if nal.len() > 32 << 20 {
            return Err(invalid("AVC slice exceeds 32 MiB"));
        }
        if pps.sps_id != sps.id {
            return Err(invalid("AVC PPS/SPS mismatch"));
        }
        let idr = nal[0] & 31 == 5;
        let nal_ref_idc = (nal[0] >> 5) & 3;
        if idr && nal_ref_idc == 0 {
            return Err(invalid("IDR NAL cannot be non-reference"));
        }
        let rbsp = unescape_rbsp(&nal[1..])?;
        let mut b = BitReader::new(&rbsp);
        let first_mb = b.unsigned_golomb()?;
        let code = ue(&mut b, 9)?;
        let slice_type = match code % 5 {
            0 => SliceType::P,
            1 => SliceType::B,
            2 => SliceType::I,
            3 => SliceType::Sp,
            _ => SliceType::Si,
        };
        if idr && !matches!(slice_type, SliceType::I | SliceType::Si) {
            return Err(invalid("IDR contains non-intra slice"));
        }
        let pps_id = ue(&mut b, 255)?;
        if pps_id != pps.id {
            return Err(invalid("slice references another PPS"));
        }
        let colour_plane_id = if sps.separate_colour_plane {
            let id = b.read(2)? as u8;
            if id > 2 {
                return Err(invalid("reserved colour plane ID"));
            }
            id
        } else {
            0
        };
        let frame_num = b.read(sps.frame_num_bits)?;
        if idr && frame_num != 0 {
            return Err(invalid("IDR frame_num must be zero"));
        }
        let field_pic = if sps.frame_mbs_only { false } else { b.bit()? };
        let bottom_field = field_pic && b.bit()?;
        let picture_size = u64::from(sps.width_mbs)
            * u64::from(sps.height_map_units)
            * if sps.frame_mbs_only || field_pic {
                1
            } else {
                2
            };
        let mbaff = sps.mb_adaptive_frame_field && !field_pic;
        if u64::from(first_mb) * if mbaff { 2 } else { 1 } >= picture_size {
            return Err(invalid("slice begins outside picture"));
        }
        let idr_pic_id = if idr { Some(ue(&mut b, 65535)?) } else { None };
        let mut poc_lsb = None;
        let mut delta_poc_bottom = 0;
        let mut delta_poc = [0; 2];
        match &sps.picture_order {
            PictureOrder::Lsb { bits } => {
                poc_lsb = Some(b.read(*bits)?);
                if pps.bottom_field_pic_order_present && !field_pic {
                    delta_poc_bottom = b.signed_golomb()?;
                }
            }
            PictureOrder::Cycle {
                always_zero: false, ..
            } => {
                delta_poc[0] = b.signed_golomb()?;
                if pps.bottom_field_pic_order_present && !field_pic {
                    delta_poc[1] = b.signed_golomb()?;
                }
            }
            _ => {}
        }
        let redundant_pic_cnt = if pps.redundant_pic_cnt_present {
            ue(&mut b, 127)?
        } else {
            0
        };
        let direct_spatial_mv_pred = slice_type == SliceType::B && b.bit()?;
        let mut refs_l0 = pps.default_refs_l0;
        let mut refs_l1 = pps.default_refs_l1;
        if matches!(slice_type, SliceType::P | SliceType::B | SliceType::Sp) && b.bit()? {
            refs_l0 = 1 + ue(&mut b, 31)?;
            if slice_type == SliceType::B {
                refs_l1 = 1 + ue(&mut b, 31)?;
            }
        }
        if !(1..=32).contains(&refs_l0) || !(1..=32).contains(&refs_l1) {
            return Err(invalid("invalid active reference count"));
        }
        let modifications_l0 = if !matches!(slice_type, SliceType::I | SliceType::Si) {
            modifications(&mut b)?
        } else {
            Vec::new()
        };
        let modifications_l1 = if slice_type == SliceType::B {
            modifications(&mut b)?
        } else {
            Vec::new()
        };
        let weighted = (pps.weighted_pred && matches!(slice_type, SliceType::P | SliceType::Sp))
            || (pps.weighted_bipred == 1 && slice_type == SliceType::B);
        let weights = if weighted {
            let has_chroma = sps.chroma_format != 0 && !sps.separate_colour_plane;
            let luma_denom = ue(&mut b, 7)? as u8;
            let chroma_denom = if has_chroma { ue(&mut b, 7)? as u8 } else { 0 };
            let l0 = weight_list(&mut b, refs_l0, luma_denom, chroma_denom, has_chroma)?;
            let l1 = if slice_type == SliceType::B {
                weight_list(&mut b, refs_l1, luma_denom, chroma_denom, has_chroma)?
            } else {
                Vec::new()
            };
            Some(Weights {
                luma_denom,
                chroma_denom,
                l0,
                l1,
            })
        } else {
            None
        };
        let (mut no_output_of_prior_pics, mut long_term_reference, mut adaptive_reference_marking) =
            (false, false, false);
        let mut memory_operations = Vec::new();
        if nal_ref_idc != 0 {
            if idr {
                no_output_of_prior_pics = b.bit()?;
                long_term_reference = b.bit()?;
            } else {
                adaptive_reference_marking = b.bit()?;
                if adaptive_reference_marking {
                    loop {
                        let op = ue(&mut b, 6)?;
                        if op == 0 {
                            break;
                        }
                        if memory_operations.len() >= 64 {
                            return Err(invalid("too many AVC memory operations"));
                        }
                        memory_operations.push(match op {
                            1 => MemoryOperation::ForgetShort(b.unsigned_golomb()?),
                            2 => MemoryOperation::ForgetLong(b.unsigned_golomb()?),
                            3 => MemoryOperation::ShortToLong {
                                difference: b.unsigned_golomb()?,
                                index: b.unsigned_golomb()?,
                            },
                            4 => MemoryOperation::LimitLong(b.unsigned_golomb()?),
                            5 => MemoryOperation::Reset,
                            _ => MemoryOperation::CurrentLong(b.unsigned_golomb()?),
                        });
                    }
                }
            }
        }
        let cabac_init_idc = if pps.cabac && !matches!(slice_type, SliceType::I | SliceType::Si) {
            ue(&mut b, 2)?
        } else {
            0
        };
        let slice_qp = pps
            .initial_qp
            .checked_add(b.signed_golomb()?)
            .ok_or_else(|| invalid("slice QP overflow"))?;
        if !(-6 * (i32::from(sps.bit_depth_luma) - 8)..=51).contains(&slice_qp) {
            return Err(invalid("slice QP out of range"));
        }
        let sp_for_switch = slice_type == SliceType::Sp && b.bit()?;
        let slice_qs = if matches!(slice_type, SliceType::Sp | SliceType::Si) {
            let qs = pps
                .initial_qs
                .checked_add(b.signed_golomb()?)
                .ok_or_else(|| invalid("slice QS overflow"))?;
            if !(0..=51).contains(&qs) {
                return Err(invalid("slice QS out of range"));
            }
            Some(qs)
        } else {
            None
        };
        let (mut disable_deblocking_filter_idc, mut alpha_offset, mut beta_offset) = (0, 0, 0);
        if pps.deblocking_filter_control_present {
            disable_deblocking_filter_idc = ue(&mut b, 2)?;
            if disable_deblocking_filter_idc != 1 {
                let a = b.signed_golomb()?;
                let c = b.signed_golomb()?;
                if !(-6..=6).contains(&a) || !(-6..=6).contains(&c) {
                    return Err(invalid("deblocking offset out of range"));
                }
                alpha_offset = 2 * a;
                beta_offset = 2 * c;
            }
        }
        let slice_group_change_cycle = if let SliceGroups::Changing { rate, .. } = &pps.slice_groups
        {
            if *rate == 0 {
                return Err(invalid("zero slice group change rate"));
            }
            let map_units = u64::from(sps.width_mbs) * u64::from(sps.height_map_units);
            let max_cycle = map_units.div_ceil(u64::from(*rate));
            let bits = (64 - max_cycle.leading_zeros()) as u8;
            let cycle = b.read(bits)?;
            if u64::from(cycle) > max_cycle {
                return Err(invalid("slice group cycle out of range"));
            }
            Some(cycle)
        } else {
            None
        };
        let header_bits = b.position();
        if pps.cabac {
            while !b.position().is_multiple_of(8) {
                if !b.bit()? {
                    return Err(invalid("CABAC alignment bit must be one"));
                }
            }
        }
        let entropy_bit_offset = b.position();
        if b.remaining() == 0 {
            return Err(invalid("slice has no entropy payload"));
        }
        Ok(Self {
            nal_ref_idc,
            idr,
            first_mb,
            slice_type,
            all_same_type: code >= 5,
            pps_id,
            colour_plane_id,
            frame_num,
            field_pic,
            bottom_field,
            idr_pic_id,
            poc_lsb,
            delta_poc_bottom,
            delta_poc,
            redundant_pic_cnt,
            direct_spatial_mv_pred,
            refs_l0,
            refs_l1,
            modifications_l0,
            modifications_l1,
            weights,
            no_output_of_prior_pics,
            long_term_reference,
            adaptive_reference_marking,
            memory_operations,
            cabac_init_idc,
            slice_qp,
            sp_for_switch,
            slice_qs,
            disable_deblocking_filter_idc,
            alpha_offset,
            beta_offset,
            slice_group_change_cycle,
            header_bits,
            entropy_bit_offset,
            rbsp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn params() -> (Sps, Pps) {
        let sps = Sps::parse(&hex("6742c01fda03c045fbc044000003000400000300f03c60ca80")).unwrap();
        let pps = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &sps).unwrap();
        (sps, pps)
    }
    fn slice() -> Vec<u8> {
        hex("6588843a0c6001e80489846b55e09f80113808531376a2d02325a04c001e2f0c")
    }
    #[test]
    fn baseline_idr_header_matches_reference_trace() {
        let (sps, pps) = params();
        let nal = slice();
        assert_eq!(SliceHeader::parameter_set_id(&nal).unwrap(), 0);
        let h = SliceHeader::parse(&nal, &sps, &pps).unwrap();
        assert!(h.idr);
        assert_eq!(h.nal_ref_idc, 3);
        assert_eq!(h.slice_type, SliceType::I);
        assert_eq!((h.first_mb, h.frame_num, h.pps_id), (0, 0, 0));
        assert_eq!(h.idr_pic_id, Some(0));
        assert_eq!(h.slice_qp, 25);
        assert_eq!(h.disable_deblocking_filter_idc, 1);
        assert_eq!((h.header_bits, h.entropy_bit_offset), (24, 24));
    }
    #[test]
    fn short_headers_and_wrong_parameter_sets_are_rejected() {
        let (sps, mut pps) = params();
        let nal = slice();
        for n in 0..=4 {
            assert!(SliceHeader::parse(&nal[..n], &sps, &pps).is_err());
        }
        pps.id = 1;
        assert!(SliceHeader::parse(&nal, &sps, &pps).is_err());
        pps.id = 0;
        pps.sps_id = 1;
        assert!(SliceHeader::parse(&nal, &sps, &pps).is_err());
    }
    #[test]
    fn bounded_mutations_do_not_panic() {
        let (sps, pps) = params();
        let original = slice();
        for i in 0..original.len() {
            for value in 0..=255 {
                let mut nal = original.clone();
                nal[i] = value;
                let _ = SliceHeader::parameter_set_id(&nal);
                if let Ok(h) = SliceHeader::parse(&nal, &sps, &pps) {
                    assert!(h.entropy_bit_offset < h.rbsp.len() * 8);
                    assert!((0..=51).contains(&h.slice_qp));
                }
            }
        }
    }
}
