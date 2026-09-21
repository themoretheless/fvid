//! H.264 sequence parameter syntax, implemented independently of codec libraries.
use super::bits::{BitReader, unescape_rbsp};
use crate::{Result, invalid};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PictureOrder {
    Lsb {
        bits: u8,
    },
    Cycle {
        always_zero: bool,
        non_ref_offset: i32,
        top_bottom_offset: i32,
        offsets: Vec<i32>,
    },
    DecodeOrder,
}
#[derive(Debug, Clone)]
pub enum ScalingList {
    Default,
    Explicit(Vec<u8>),
}
#[derive(Debug, Clone)]
pub struct Hrd {
    pub bit_rate_scale: u8,
    pub cpb_size_scale: u8,
    pub cpbs: Vec<(u32, u32, bool)>,
    pub initial_delay_bits: u8,
    pub removal_delay_bits: u8,
    pub output_delay_bits: u8,
    pub time_offset_bits: u8,
}
#[derive(Debug, Clone, Default)]
pub struct Vui {
    pub aspect_ratio: Option<(u16, u16)>,
    pub overscan_appropriate: Option<bool>,
    pub video_signal: Option<(u8, bool, Option<[u8; 3]>)>,
    pub chroma_location: Option<(u32, u32)>,
    /// num_units_in_tick, time_scale, fixed_frame_rate_flag.
    pub timing: Option<(u32, u32, bool)>,
    pub nal_hrd: Option<Hrd>,
    pub vcl_hrd: Option<Hrd>,
    pub low_delay_hrd: bool,
    pub pic_struct_present: bool,
    pub restriction: Option<BitstreamRestriction>,
}
#[derive(Debug, Clone)]
pub struct BitstreamRestriction {
    pub motion_vectors_over_boundaries: bool,
    pub max_bytes_per_pic_denom: u32,
    pub max_bits_per_mb_denom: u32,
    pub log2_max_mv_length_horizontal: u32,
    pub log2_max_mv_length_vertical: u32,
    pub max_num_reorder_frames: u32,
    pub max_dec_frame_buffering: u32,
}
#[derive(Debug, Clone)]
pub struct Sps {
    pub profile: u8,
    pub constraints: u8,
    pub level: u8,
    pub id: u32,
    pub chroma_format: u32,
    pub separate_colour_plane: bool,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub transform_bypass: bool,
    /// Absent entries require SPS scaling-list fallback rule A.
    pub scaling_lists: Option<Vec<Option<ScalingList>>>,
    pub frame_num_bits: u8,
    pub picture_order: PictureOrder,
    pub max_num_ref_frames: u32,
    pub gaps_allowed: bool,
    pub width_mbs: u32,
    pub height_map_units: u32,
    pub frame_mbs_only: bool,
    pub mb_adaptive_frame_field: bool,
    pub direct_8x8_inference: bool,
    /// Left, right, top, bottom in luma pixels.
    pub crop: [u32; 4],
    pub vui: Option<Vui>,
}
fn ue(b: &mut BitReader<'_>, maximum: u32, field: &str) -> Result<u32> {
    let v = b.unsigned_golomb()?;
    if v > maximum {
        return Err(invalid(&format!("AVC {field} out of range")));
    }
    Ok(v)
}
fn scaling(b: &mut BitReader<'_>, count: usize) -> Result<ScalingList> {
    let mut last = 8i32;
    let mut next = 8i32;
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        if next != 0 {
            let delta = b.signed_golomb()?;
            if !(-128..=127).contains(&delta) {
                return Err(invalid("AVC scaling-list delta out of range"));
            }
            next = (last + delta + 256) % 256;
            if i == 0 && next == 0 {
                return Ok(ScalingList::Default);
            }
        }
        let value = if next == 0 { last } else { next };
        values.push(value as u8);
        last = value;
    }
    Ok(ScalingList::Explicit(values))
}
fn hrd(b: &mut BitReader<'_>) -> Result<Hrd> {
    let count = ue(b, 31, "cpb_cnt_minus1")? + 1;
    let bit_rate_scale = b.read(4)? as u8;
    let cpb_size_scale = b.read(4)? as u8;
    let mut cpbs = Vec::new();
    for _ in 0..count {
        cpbs.push((b.unsigned_golomb()?, b.unsigned_golomb()?, b.bit()?));
    }
    Ok(Hrd {
        bit_rate_scale,
        cpb_size_scale,
        cpbs,
        initial_delay_bits: b.read(5)? as u8 + 1,
        removal_delay_bits: b.read(5)? as u8 + 1,
        output_delay_bits: b.read(5)? as u8 + 1,
        time_offset_bits: b.read(5)? as u8,
    })
}
fn vui(b: &mut BitReader<'_>) -> Result<Vui> {
    let mut v = Vui::default();
    if b.bit()? {
        let id = b.read(8)? as usize;
        const SAR: [(u16, u16); 17] = [
            (0, 0),
            (1, 1),
            (12, 11),
            (10, 11),
            (16, 11),
            (40, 33),
            (24, 11),
            (20, 11),
            (32, 11),
            (80, 33),
            (18, 11),
            (15, 11),
            (64, 33),
            (160, 99),
            (4, 3),
            (3, 2),
            (2, 1),
        ];
        v.aspect_ratio = if id == 255 {
            Some((b.read(16)? as u16, b.read(16)? as u16))
        } else if id == 0 {
            None
        } else {
            Some(
                *SAR.get(id)
                    .ok_or_else(|| invalid("reserved AVC aspect_ratio_idc"))?,
            )
        };
        if v.aspect_ratio.is_some_and(|(w, h)| w == 0 || h == 0) {
            return Err(invalid("zero AVC sample aspect ratio"));
        }
    }
    if b.bit()? {
        v.overscan_appropriate = Some(b.bit()?);
    }
    if b.bit()? {
        let format = b.read(3)? as u8;
        let full = b.bit()?;
        if format > 5 {
            return Err(invalid("reserved AVC video format"));
        }
        let colour = if b.bit()? {
            Some([b.read(8)? as u8, b.read(8)? as u8, b.read(8)? as u8])
        } else {
            None
        };
        v.video_signal = Some((format, full, colour));
    }
    if b.bit()? {
        v.chroma_location = Some((ue(b, 5, "chroma location")?, ue(b, 5, "chroma location")?));
    }
    if b.bit()? {
        let units = b.read(32)?;
        let scale = b.read(32)?;
        let fixed = b.bit()?;
        if units == 0 || scale == 0 {
            return Err(invalid("zero AVC timing value"));
        }
        v.timing = Some((units, scale, fixed));
    }
    if b.bit()? {
        v.nal_hrd = Some(hrd(b)?);
    }
    if b.bit()? {
        v.vcl_hrd = Some(hrd(b)?);
    }
    if v.nal_hrd.is_some() || v.vcl_hrd.is_some() {
        v.low_delay_hrd = b.bit()?;
    }
    v.pic_struct_present = b.bit()?;
    if b.bit()? {
        let r = BitstreamRestriction {
            motion_vectors_over_boundaries: b.bit()?,
            max_bytes_per_pic_denom: ue(b, 16, "max_bytes_per_pic_denom")?,
            max_bits_per_mb_denom: ue(b, 16, "max_bits_per_mb_denom")?,
            log2_max_mv_length_horizontal: ue(b, 16, "horizontal MV limit")?,
            log2_max_mv_length_vertical: ue(b, 16, "vertical MV limit")?,
            max_num_reorder_frames: ue(b, 16, "reorder frame limit")?,
            max_dec_frame_buffering: ue(b, 16, "decoded frame buffer limit")?,
        };
        if r.max_num_reorder_frames > r.max_dec_frame_buffering {
            return Err(invalid("AVC reorder depth exceeds frame buffer"));
        }
        v.restriction = Some(r);
    }
    Ok(v)
}
impl Sps {
    pub fn coded_dimensions(&self) -> (u32, u32) {
        (
            self.width_mbs * 16,
            self.height_map_units * 16 * if self.frame_mbs_only { 1 } else { 2 },
        )
    }
    pub fn display_dimensions(&self) -> (u32, u32) {
        let (w, h) = self.coded_dimensions();
        (
            w - self.crop[0] - self.crop[1],
            h - self.crop[2] - self.crop[3],
        )
    }
    pub fn parse(nal: &[u8]) -> Result<Self> {
        if nal
            .first()
            .is_none_or(|h| h & 0x80 != 0 || h & 31 != 7 || h & 0x60 == 0)
        {
            return Err(invalid("expected AVC SPS NAL"));
        }
        if nal.len() > 65535 {
            return Err(invalid("AVC SPS exceeds size limit"));
        }
        let rbsp = unescape_rbsp(&nal[1..])?;
        let mut b = BitReader::new(&rbsp);
        let profile = b.read(8)? as u8;
        let constraints = b.read(8)? as u8;
        let level = b.read(8)? as u8;
        if constraints & 3 != 0 {
            return Err(invalid("AVC SPS reserved bits are nonzero"));
        }
        let id = ue(&mut b, 31, "SPS id")?;
        let mut chroma_format = 1;
        let mut separate_colour_plane = false;
        let mut bit_depth_luma = 8;
        let mut bit_depth_chroma = 8;
        let mut transform_bypass = false;
        let mut scaling_lists = None;
        if matches!(
            profile,
            100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
        ) {
            chroma_format = ue(&mut b, 3, "chroma_format_idc")?;
            if chroma_format == 3 {
                separate_colour_plane = b.bit()?;
            }
            bit_depth_luma = 8 + ue(&mut b, 6, "luma bit depth")? as u8;
            bit_depth_chroma = 8 + ue(&mut b, 6, "chroma bit depth")? as u8;
            transform_bypass = b.bit()?;
            if b.bit()? {
                let count = if chroma_format == 3 { 12 } else { 8 };
                let mut lists = Vec::new();
                for i in 0..count {
                    lists.push(if b.bit()? {
                        Some(scaling(&mut b, if i < 6 { 16 } else { 64 })?)
                    } else {
                        None
                    });
                }
                scaling_lists = Some(lists);
            }
        } else if !matches!(profile, 66 | 77 | 88) {
            return Err(invalid("unsupported AVC SPS profile"));
        }
        let frame_num_bits = 4 + ue(&mut b, 12, "frame_num width")? as u8;
        let picture_order = match ue(&mut b, 2, "pic_order_cnt_type")? {
            0 => PictureOrder::Lsb {
                bits: 4 + ue(&mut b, 12, "POC LSB width")? as u8,
            },
            1 => {
                let always_zero = b.bit()?;
                let non_ref_offset = b.signed_golomb()?;
                let top_bottom_offset = b.signed_golomb()?;
                let count = ue(&mut b, 255, "POC cycle size")?;
                let mut offsets = Vec::new();
                for _ in 0..count {
                    offsets.push(b.signed_golomb()?);
                }
                PictureOrder::Cycle {
                    always_zero,
                    non_ref_offset,
                    top_bottom_offset,
                    offsets,
                }
            }
            _ => PictureOrder::DecodeOrder,
        };
        let max_num_ref_frames = ue(&mut b, 16, "reference frame limit")?;
        let gaps_allowed = b.bit()?;
        let width_mbs = 1 + ue(&mut b, 65535, "macroblock width")?;
        let height_map_units = 1 + ue(&mut b, 65535, "map-unit height")?;
        let frame_mbs_only = b.bit()?;
        let mb_adaptive_frame_field = if frame_mbs_only { false } else { b.bit()? };
        let direct_8x8_inference = b.bit()?;
        let mut crop = [0; 4];
        let chroma_array_type = if separate_colour_plane {
            0
        } else {
            chroma_format
        };
        let (sub_x, sub_y) = match chroma_array_type {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        let crop_y = sub_y * if frame_mbs_only { 1 } else { 2 };
        if b.bit()? {
            for (i, value) in crop.iter_mut().enumerate() {
                *value = b
                    .unsigned_golomb()?
                    .checked_mul(if i < 2 { sub_x } else { crop_y })
                    .ok_or_else(|| invalid("AVC crop overflow"))?;
            }
        }
        let vui = if b.bit()? { Some(vui(&mut b)?) } else { None };
        b.finish_rbsp()?;
        let s = Self {
            profile,
            constraints,
            level,
            id,
            chroma_format,
            separate_colour_plane,
            bit_depth_luma,
            bit_depth_chroma,
            transform_bypass,
            scaling_lists,
            frame_num_bits,
            picture_order,
            max_num_ref_frames,
            gaps_allowed,
            width_mbs,
            height_map_units,
            frame_mbs_only,
            mb_adaptive_frame_field,
            direct_8x8_inference,
            crop,
            vui,
        };
        let (w, h) = s.coded_dimensions();
        if crop[0].checked_add(crop[1]).is_none_or(|n| n >= w)
            || crop[2].checked_add(crop[3]).is_none_or(|n| n >= h)
        {
            return Err(invalid("AVC crop removes entire picture"));
        }
        if s.vui
            .as_ref()
            .and_then(|v| v.restriction.as_ref())
            .is_some_and(|r| r.max_dec_frame_buffering < max_num_ref_frames)
        {
            return Err(invalid("AVC frame buffer smaller than reference count"));
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn baseline() -> Vec<u8> {
        // x264 Constrained Baseline, 960x540, 30 fps. Stored SPS only, not a codec implementation.
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }
    #[test]
    fn reads_real_baseline_geometry_and_timing() {
        let s = Sps::parse(&baseline()).unwrap();
        assert_eq!(
            (s.profile, s.id, s.bit_depth_luma, s.chroma_format),
            (66, 0, 8, 1)
        );
        assert_eq!(s.coded_dimensions(), (960, 544));
        assert_eq!(s.display_dimensions(), (960, 540));
        assert_eq!(s.crop, [0, 0, 0, 4]);
        let (units, scale, _) = s.vui.as_ref().unwrap().timing.unwrap();
        assert_eq!(scale / units, 60);
        assert!(s.frame_mbs_only);
    }
    #[test]
    fn rejects_truncated_and_extra_rbsp_data() {
        let nal = baseline();
        for n in 0..nal.len() {
            assert!(Sps::parse(&nal[..n]).is_err(), "length {n}");
        }
        let mut extra = nal.clone();
        extra.push(0);
        assert!(Sps::parse(&extra).is_err());
        let mut forbidden = nal.clone();
        forbidden[0] |= 128;
        assert!(Sps::parse(&forbidden).is_err());
    }
    #[test]
    fn baseline_pps_and_truncated_or_wrong_reference() {
        let mut sps = Sps::parse(&baseline()).unwrap();
        let nal = [0x68, 0xce, 0x09, 0xc8];
        let pps = Pps::parse(&nal, &sps).unwrap();
        assert_eq!((pps.id, pps.sps_id), (0, 0));
        assert!(!pps.cabac);
        assert!(matches!(pps.slice_groups, SliceGroups::Single));
        for n in 0..nal.len() {
            assert!(Pps::parse(&nal[..n], &sps).is_err());
        }
        for i in 0..nal.len() {
            for value in 0..=255 {
                let mut changed = nal;
                changed[i] = value;
                let _ = Pps::parse(&changed, &sps);
            }
        }
        sps.id = 1;
        assert!(Pps::parse(&nal, &sps).is_err());
    }
    #[test]
    fn mutations_never_panic_and_successful_geometry_is_positive() {
        let original = baseline();
        for i in 0..original.len() {
            for value in 0..=255 {
                let mut nal = original.clone();
                nal[i] = value;
                if let Ok(s) = Sps::parse(&nal) {
                    let (w, h) = s.display_dimensions();
                    assert!(w > 0 && h > 0);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum SliceGroups {
    Single,
    Interleaved(Vec<u32>),
    Dispersed(u32),
    Foreground(Vec<(u32, u32)>),
    Changing {
        map_type: u32,
        direction: bool,
        rate: u32,
    },
    Explicit {
        groups: u32,
        map: Vec<u8>,
    },
}
#[derive(Debug, Clone)]
pub struct Pps {
    pub id: u32,
    pub sps_id: u32,
    pub cabac: bool,
    pub bottom_field_pic_order_present: bool,
    pub slice_groups: SliceGroups,
    pub default_refs_l0: u32,
    pub default_refs_l1: u32,
    pub weighted_pred: bool,
    pub weighted_bipred: u8,
    pub initial_qp: i32,
    pub initial_qs: i32,
    pub chroma_qp_offset: i32,
    pub deblocking_filter_control_present: bool,
    pub constrained_intra_pred: bool,
    pub redundant_pic_cnt_present: bool,
    pub transform_8x8: bool,
    /// Missing entries require PPS fallback rule B, using the referenced SPS.
    pub scaling_lists: Option<Vec<Option<ScalingList>>>,
    pub second_chroma_qp_offset: i32,
}
fn se(b: &mut BitReader<'_>, min: i32, max: i32, field: &str) -> Result<i32> {
    let n = b.signed_golomb()?;
    if !(min..=max).contains(&n) {
        return Err(invalid(&format!("AVC {field} out of range")));
    }
    Ok(n)
}
impl Pps {
    pub fn parse(nal: &[u8], sps: &Sps) -> Result<Self> {
        if nal
            .first()
            .is_none_or(|h| h & 0x80 != 0 || h & 31 != 8 || h & 0x60 == 0)
        {
            return Err(invalid("expected AVC PPS NAL"));
        }
        if nal.len() > 65535 {
            return Err(invalid("AVC PPS exceeds size limit"));
        }
        let rbsp = unescape_rbsp(&nal[1..])?;
        let mut b = BitReader::new(&rbsp);
        let id = ue(&mut b, 255, "PPS id")?;
        let sps_id = ue(&mut b, 31, "PPS SPS id")?;
        if sps_id != sps.id {
            return Err(invalid("PPS references a different SPS"));
        }
        let cabac = b.bit()?;
        let bottom_field_pic_order_present = b.bit()?;
        let groups = 1 + ue(&mut b, 7, "slice group count")?;
        let picture_size = sps
            .width_mbs
            .checked_mul(sps.height_map_units)
            .ok_or_else(|| invalid("slice group picture overflow"))?;
        let max_index = picture_size
            .checked_sub(1)
            .ok_or_else(|| invalid("empty slice group picture"))?;
        let slice_groups = if groups == 1 {
            SliceGroups::Single
        } else {
            match ue(&mut b, 6, "slice group map type")? {
                0 => {
                    let mut lengths = Vec::new();
                    for _ in 0..groups {
                        lengths.push(1 + ue(&mut b, max_index, "slice group run")?);
                    }
                    SliceGroups::Interleaved(lengths)
                }
                1 => SliceGroups::Dispersed(groups),
                2 => {
                    let mut rects = Vec::new();
                    for _ in 0..groups - 1 {
                        let top = ue(&mut b, max_index, "slice group top left")?;
                        let bottom = ue(&mut b, max_index, "slice group bottom right")?;
                        if top > bottom || top % sps.width_mbs > bottom % sps.width_mbs {
                            return Err(invalid("invalid slice group rectangle"));
                        }
                        rects.push((top, bottom));
                    }
                    SliceGroups::Foreground(rects)
                }
                map_type @ 3..=5 => {
                    if groups != 2 {
                        return Err(invalid("changing slice maps require two groups"));
                    }
                    SliceGroups::Changing {
                        map_type,
                        direction: b.bit()?,
                        rate: 1 + ue(&mut b, max_index, "slice group change rate")?,
                    }
                }
                _ => {
                    let size = 1 + ue(
                        &mut b,
                        max_index.min(1_048_575),
                        "explicit slice group map size",
                    )?;
                    let bits = (32 - (groups - 1).leading_zeros()) as u8;
                    let mut map = Vec::with_capacity(size as usize);
                    for _ in 0..size {
                        let group = b.read(bits)?;
                        if group >= groups {
                            return Err(invalid("slice group ID out of range"));
                        }
                        map.push(group as u8);
                    }
                    SliceGroups::Explicit { groups, map }
                }
            }
        };
        let default_refs_l0 = 1 + ue(&mut b, 31, "L0 reference count")?;
        let default_refs_l1 = 1 + ue(&mut b, 31, "L1 reference count")?;
        let weighted_pred = b.bit()?;
        let weighted_bipred = b.read(2)? as u8;
        if weighted_bipred > 2 {
            return Err(invalid("reserved weighted_bipred_idc"));
        }
        let initial_qp = 26
            + se(
                &mut b,
                -26 - 6 * (i32::from(sps.bit_depth_luma) - 8),
                25,
                "initial QP",
            )?;
        let initial_qs = 26 + se(&mut b, -26, 25, "initial QS")?;
        let chroma_qp_offset = se(&mut b, -12, 12, "chroma QP offset")?;
        let deblocking_filter_control_present = b.bit()?;
        let constrained_intra_pred = b.bit()?;
        let redundant_pic_cnt_present = b.bit()?;
        let mut transform_8x8 = false;
        let mut scaling_lists = None;
        let mut second_chroma_qp_offset = chroma_qp_offset;
        if b.more_rbsp_data() {
            transform_8x8 = b.bit()?;
            if b.bit()? {
                let extra = if sps.chroma_format == 3 { 6 } else { 2 };
                let count = 6 + if transform_8x8 { extra } else { 0 };
                let mut lists = Vec::new();
                for i in 0..count {
                    lists.push(if b.bit()? {
                        Some(scaling(&mut b, if i < 6 { 16 } else { 64 })?)
                    } else {
                        None
                    });
                }
                scaling_lists = Some(lists);
            }
            second_chroma_qp_offset = se(&mut b, -12, 12, "second chroma QP offset")?;
        }
        b.finish_rbsp()?;
        Ok(Self {
            id,
            sps_id,
            cabac,
            bottom_field_pic_order_present,
            slice_groups,
            default_refs_l0,
            default_refs_l1,
            weighted_pred,
            weighted_bipred,
            initial_qp,
            initial_qs,
            chroma_qp_offset,
            deblocking_filter_control_present,
            constrained_intra_pred,
            redundant_pic_cnt_present,
            transform_8x8,
            scaling_lists,
            second_chroma_qp_offset,
        })
    }
}
