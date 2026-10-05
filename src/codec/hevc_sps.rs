//! H.265 sequence parameters for base-layer pictures.
use super::{
    bits::BitReader,
    hevc_nal::NalRbsp,
    hevc_profile::ProfileTierLevel,
    hevc_vps::{Ordering, read_ordering},
    hevc_vui::Vui,
};
use crate::{Result, invalid};
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let value = b.unsigned_golomb()?;
    if value > max {
        return Err(invalid("HEVC SPS value exceeds range"));
    }
    Ok(value)
}
pub use super::hevc_rps::ShortTermReference;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub depth: [u8; 2],
    pub block_log2: [u8; 2],
    pub loop_filter_disabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sps {
    pub vps_id: u8,
    pub id: u8,
    pub temporal_id_nesting: bool,
    pub profile: ProfileTierLevel,
    pub chroma_format: u8,
    pub separate_colour_plane: bool,
    pub dimensions: [u32; 2],
    /// Cropping in luma samples, left/right/top/bottom.
    pub crop: [u32; 4],
    pub depth: [u8; 2],
    pub poc_bits: u8,
    pub ordering: Vec<Ordering>,
    #[cfg(test)]
    pub(crate) ordering_bit_range: std::ops::Range<usize>,
    pub coding_block_log2: [u8; 2],
    pub transform_block_log2: [u8; 2],
    pub transform_hierarchy_depth: [u8; 2],
    pub scaling_lists_enabled: bool,
    pub scaling_lists: super::hevc_scaling::ScalingLists,
    pub amp: bool,
    pub sao: bool,
    pub pcm: Option<Pcm>,
    pub short_term: Vec<Vec<ShortTermReference>>,
    #[cfg(test)]
    pub(crate) long_term_flag_bit: usize,
    pub long_term_present: bool,
    pub long_term: Vec<(u16, bool)>,
    pub temporal_mvp: bool,
    pub strong_intra_smoothing: bool,
    /// Range-extension switch for both weak and strong reference filtering.
    pub intra_smoothing_disabled: bool,
    pub transform_skip_rotation: bool,
    pub transform_skip_context: bool,
    pub implicit_rdpcm: bool,
    pub explicit_rdpcm: bool,
    pub high_precision_offsets: bool,
    pub persistent_rice: bool,
    pub vui: Option<Vui>,
}
impl Sps {
    pub fn display_dimensions(&self) -> [u32; 2] {
        [
            self.dimensions[0] - self.crop[0] - self.crop[1],
            self.dimensions[1] - self.crop[2] - self.crop[3],
        ]
    }
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        let rbsp = NalRbsp::parse(nal, budget)?;
        rbsp.header.require_base_layer()?;
        if rbsp.header.unit_type != 33 || rbsp.header.temporal_id != 0 {
            return Err(invalid("expected HEVC SPS at temporal ID zero"));
        }
        let b = &mut BitReader::new(&rbsp.bytes);
        let vps_id = b.read(4)? as u8;
        let max_sub = b.read(3)? as u8;
        let temporal_id_nesting = b.bit()?;
        if max_sub == 0 && !temporal_id_nesting {
            return Err(invalid("SPS temporal nesting must be set"));
        }
        let profile = ProfileTierLevel::read(b, true, max_sub)?;
        let id = ue(b, 15)? as u8;
        let chroma_format = ue(b, 3)? as u8;
        let separate_colour_plane = chroma_format == 3 && b.bit()?;
        let dimensions = [b.unsigned_golomb()?, b.unsigned_golomb()?];
        if dimensions.contains(&0) {
            return Err(invalid("zero HEVC picture dimension"));
        }
        let mut crop = [0; 4];
        if b.bit()? {
            let scale = if separate_colour_plane {
                [1, 1]
            } else {
                match chroma_format {
                    1 => [2, 2],
                    2 => [2, 1],
                    _ => [1, 1],
                }
            };
            for i in 0..4 {
                crop[i] = b
                    .unsigned_golomb()?
                    .checked_mul(scale[i / 2])
                    .ok_or_else(|| invalid("HEVC conformance crop overflow"))?;
            }
            for axis in 0..2 {
                if u64::from(crop[axis * 2]) + u64::from(crop[axis * 2 + 1])
                    >= u64::from(dimensions[axis])
                {
                    return Err(invalid("HEVC conformance crop removes picture"));
                }
            }
        }
        let depth = [ue(b, 8)? as u8 + 8, ue(b, 8)? as u8 + 8];
        let poc_bits = ue(b, 12)? as u8 + 4;
        #[cfg(test)]
        let ordering_start = b.position();
        let ordering = read_ordering(b, max_sub)?;
        #[cfg(test)]
        let ordering_bit_range = ordering_start..b.position();
        let min_cb = ue(b, 3)? as u8 + 3;
        let max_cb = min_cb + ue(b, 3)? as u8;
        let min_tb = ue(b, 3)? as u8 + 2;
        let max_tb = min_tb + ue(b, 3)? as u8;
        if !(4..=6).contains(&max_cb)
            || min_tb > min_cb
            || max_tb > 5
            || max_tb > max_cb
            || dimensions.iter().any(|v| v % (1u32 << min_cb) != 0)
        {
            return Err(invalid("invalid HEVC coding/transform block geometry"));
        }
        let transform_hierarchy_depth = [
            ue(b, u32::from(max_cb - min_tb))? as u8,
            ue(b, u32::from(max_cb - min_tb))? as u8,
        ];
        let scaling_lists_enabled = b.bit()?;
        let scaling_lists = if !scaling_lists_enabled {
            super::hevc_scaling::ScalingLists::flat()
        } else if b.bit()? {
            super::hevc_scaling::ScalingLists::read(b)?
        } else {
            super::hevc_scaling::ScalingLists::default()
        };
        let amp = b.bit()?;
        let sao = b.bit()?;
        let pcm = if b.bit()? {
            let pcm_depth = [b.read(4)? as u8 + 1, b.read(4)? as u8 + 1];
            let min = ue(b, 2)? as u8 + 3;
            let max = min + ue(b, 2)? as u8;
            if max > max_cb || max > 5 || pcm_depth[0] > depth[0] || pcm_depth[1] > depth[1] {
                return Err(invalid("invalid HEVC PCM parameters"));
            }
            Some(Pcm {
                depth: pcm_depth,
                block_log2: [min, max],
                loop_filter_disabled: b.bit()?,
            })
        } else {
            None
        };
        let count = ue(b, 64)? as usize;
        let mut short_term = Vec::with_capacity(count);
        for _ in 0..count {
            short_term.push(super::hevc_rps::read_short_term(
                b,
                &short_term,
                false,
                ordering.last().unwrap().max_decoded_pictures - 1,
            )?);
        }
        #[cfg(test)]
        let long_term_flag_bit = b.position();
        let long_term_present = b.bit()?;
        let mut long_term = Vec::new();
        if long_term_present {
            for _ in 0..ue(b, 32)? {
                long_term.push((b.read(poc_bits)? as u16, b.bit()?));
            }
        }
        let temporal_mvp = b.bit()?;
        let strong_intra_smoothing = b.bit()?;
        let vui = if b.bit()? {
            Some(Vui::read(b, max_sub)?)
        } else {
            None
        };
        let mut intra_smoothing_disabled = false;
        let mut transform_skip_rotation = false;
        let mut transform_skip_context = false;
        let mut implicit_rdpcm = false;
        let mut explicit_rdpcm = false;
        let mut high_precision_offsets = false;
        let mut persistent_rice = false;
        if b.bit()? {
            let range = b.bit()?;
            if b.read(7)? != 0 {
                return Err(crate::unsupported(
                    "HEVC multilayer/3D/SCC/unknown SPS extensions are not implemented",
                ));
            }
            if range {
                let flags = b.read(9)?;
                // 7.3.2.2.2: the sixth of nine flags disables reference filtering.
                if flags & !((1 << 8) | (1 << 7) | (1 << 6) | (1 << 5) | (1 << 3) | (1 << 2) | (1 << 1)) != 0 {
                    return Err(crate::unsupported(
                        "remaining HEVC SPS range-extension tools are not implemented",
                    ));
                }
                intra_smoothing_disabled = flags & (1 << 3) != 0;
                transform_skip_rotation = flags & (1 << 8) != 0;
                transform_skip_context = flags & (1 << 7) != 0;
                implicit_rdpcm = flags & (1 << 6) != 0;
                explicit_rdpcm = flags & (1 << 5) != 0;
                high_precision_offsets = flags & (1 << 2) != 0;
                persistent_rice = flags & (1 << 1) != 0;
            }
        }
        b.finish_rbsp()?;
        Ok(Self {
            vps_id,
            id,
            temporal_id_nesting,
            profile,
            chroma_format,
            separate_colour_plane,
            dimensions,
            crop,
            depth,
            poc_bits,
            ordering,
            #[cfg(test)]
            ordering_bit_range,
            coding_block_log2: [min_cb, max_cb],
            transform_block_log2: [min_tb, max_tb],
            transform_hierarchy_depth,
            scaling_lists_enabled,
            scaling_lists,
            amp,
            sao,
            pcm,
            short_term,
            #[cfg(test)]
            long_term_flag_bit,
            long_term_present,
            long_term,
            temporal_mvp,
            strong_intra_smoothing,
            intra_smoothing_disabled,
            transform_skip_rotation,
            transform_skip_context,
            implicit_rdpcm,
            explicit_rdpcm,
            high_precision_offsets,
            persistent_rice,
            vui,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_main10_conformance_window_uses_chroma_units() {
        let hex = "42010102200000030090000003000003001ea024839c92365959ae4caf016808000003000800000300f040";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let s = Sps::parse(&nal, 1024).unwrap();
        assert_eq!(s.profile.profile.unwrap().idc, 2);
        assert_eq!(s.dimensions, [72, 56]);
        assert_eq!(s.crop, [0, 6, 0, 6]);
        assert_eq!(s.display_dimensions(), [66, 50]);
        assert_eq!(s.depth, [10, 10]);
        assert_eq!(s.vui.unwrap().timing.unwrap().time_scale, 30);
        assert!(Sps::parse(&nal, 1).is_err());
    }
    #[test]
    fn real_sps_geometry_tools_timing_and_truncation() {
        let hex =
            "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let s = Sps::parse(&nal, 1024).unwrap();
        assert_eq!(s.dimensions, [64, 64]);
        assert_eq!(s.display_dimensions(), [64, 64]);
        assert_eq!(s.depth, [8, 8]);
        assert_eq!(s.chroma_format, 1);
        assert_eq!(s.coding_block_log2, [3, 6]);
        assert_eq!(s.transform_block_log2, [2, 5]);
        assert!(s.sao && s.temporal_mvp && s.strong_intra_smoothing);
        assert_eq!(s.vui.unwrap().timing.unwrap().time_scale, 25);
        for end in 0..nal.len() {
            assert!(Sps::parse(&nal[..end], 1024).is_err());
        }
    }
}
