//! Base HEVC picture parameter sets, including tile geometry and filtering syntax.
use super::{bits::BitReader, hevc_nal::NalRbsp, hevc_scaling::ScalingLists, hevc_sps::Sps};
use crate::{Result, invalid};
fn ue(b: &mut BitReader<'_>, max: u32) -> Result<u32> {
    let n = b.unsigned_golomb()?;
    if n > max {
        return Err(invalid("HEVC PPS unsigned value exceeds range"));
    }
    Ok(n)
}
fn se(b: &mut BitReader<'_>, min: i32, max: i32) -> Result<i32> {
    let n = b.signed_golomb()?;
    if !(min..=max).contains(&n) {
        return Err(invalid("HEVC PPS signed value exceeds range"));
    }
    Ok(n)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles {
    pub column_widths: Vec<u32>,
    pub row_heights: Vec<u32>,
    pub loop_filter_across: bool,
}
fn tile_axis(b: &mut BitReader<'_>, extent: u32, count: u32, uniform: bool) -> Result<Vec<u32>> {
    let mut result = Vec::with_capacity(count as usize);
    let mut consumed = 0u32;
    for i in 0..count - 1 {
        let size = if uniform {
            ((u64::from(i + 1) * u64::from(extent)) / u64::from(count)
                - (u64::from(i) * u64::from(extent)) / u64::from(count)) as u32
        } else {
            ue(b, extent - 1)? + 1
        };
        consumed = consumed
            .checked_add(size)
            .ok_or_else(|| invalid("HEVC tile extent overflow"))?;
        if size == 0 || consumed >= extent {
            return Err(invalid("HEVC tiles exceed picture extent"));
        }
        result.push(size);
    }
    result.push(extent - consumed);
    Ok(result)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Deblocking {
    pub override_enabled: bool,
    pub disabled: bool,
    pub offsets_div2: [i8; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pps {
    pub id: u8,
    pub sps_id: u8,
    pub dependent_slices: bool,
    pub output_flag_present: bool,
    pub extra_slice_header_bits: u8,
    pub sign_data_hiding: bool,
    pub cabac_init_present: bool,
    pub default_references: [u8; 2],
    pub initial_qp: i32,
    pub constrained_intra: bool,
    pub transform_skip: bool,
    pub cu_qp_delta_depth: Option<u8>,
    pub chroma_qp_offsets: [i8; 2],
    pub slice_chroma_qp_offsets: bool,
    pub weighted_prediction: bool,
    pub weighted_biprediction: bool,
    pub transquant_bypass: bool,
    pub entropy_sync: bool,
    pub tiles: Option<Tiles>,
    pub loop_filter_across_slices: bool,
    pub deblocking: Deblocking,
    pub scaling_lists: Option<ScalingLists>,
    pub lists_modification: bool,
    pub parallel_merge_log2: u8,
    pub slice_header_extension: bool,
}
impl Pps {
    pub fn parse(nal: &[u8], sps: &Sps, budget: usize) -> Result<Self> {
        let rbsp = NalRbsp::parse(nal, budget)?;
        rbsp.header.require_base_layer()?;
        if rbsp.header.unit_type != 34 {
            return Err(invalid("expected HEVC PPS NAL"));
        }
        let b = &mut BitReader::new(&rbsp.bytes);
        let id = ue(b, 63)? as u8;
        let sps_id = ue(b, 15)? as u8;
        if sps_id != sps.id {
            return Err(invalid("HEVC PPS references another SPS"));
        }
        let dependent_slices = b.bit()?;
        let output_flag_present = b.bit()?;
        let extra_slice_header_bits = b.read(3)? as u8;
        let sign_data_hiding = b.bit()?;
        let cabac_init_present = b.bit()?;
        let default_references = [ue(b, 14)? as u8 + 1, ue(b, 14)? as u8 + 1];
        let initial_qp = se(b, -26 - 6 * (i32::from(sps.depth[0]) - 8), 25)? + 26;
        let constrained_intra = b.bit()?;
        let transform_skip = b.bit()?;
        let cu_qp_delta_depth = if b.bit()? {
            Some(ue(
                b,
                u32::from(sps.coding_block_log2[1] - sps.coding_block_log2[0]),
            )? as u8)
        } else {
            None
        };
        let chroma_qp_offsets = [se(b, -12, 12)? as i8, se(b, -12, 12)? as i8];
        let slice_chroma_qp_offsets = b.bit()?;
        let weighted_prediction = b.bit()?;
        let weighted_biprediction = b.bit()?;
        let transquant_bypass = b.bit()?;
        let tiles_enabled = b.bit()?;
        let entropy_sync = b.bit()?;
        let tiles = if tiles_enabled {
            let side = 1u32 << sps.coding_block_log2[1];
            let width = sps.dimensions[0].div_ceil(side);
            let height = sps.dimensions[1].div_ceil(side);
            let columns = ue(b, width - 1)? + 1;
            let rows = ue(b, height - 1)? + 1;
            if columns > 20 || rows > 22 {
                return Err(invalid("HEVC tile count exceeds supported level limits"));
            }
            let uniform = b.bit()?;
            Some(Tiles {
                column_widths: tile_axis(b, width, columns, uniform)?,
                row_heights: tile_axis(b, height, rows, uniform)?,
                loop_filter_across: b.bit()?,
            })
        } else {
            None
        };
        let loop_filter_across_slices = b.bit()?;
        let deblocking = if b.bit()? {
            let override_enabled = b.bit()?;
            let disabled = b.bit()?;
            let offsets_div2 = if disabled {
                [0, 0]
            } else {
                [se(b, -6, 6)? as i8, se(b, -6, 6)? as i8]
            };
            Deblocking {
                override_enabled,
                disabled,
                offsets_div2,
            }
        } else {
            Deblocking::default()
        };
        let scaling_lists = if b.bit()? {
            if !sps.scaling_lists_enabled {
                return Err(invalid("PPS scaling lists require SPS enablement"));
            }
            Some(ScalingLists::read(b)?)
        } else {
            None
        };
        let lists_modification = b.bit()?;
        let parallel_merge_log2 = ue(b, u32::from(sps.coding_block_log2[1] - 2))? as u8 + 2;
        let slice_header_extension = b.bit()?;
        if b.bit()? {
            return Err(invalid("HEVC PPS extensions are not implemented"));
        }
        b.finish_rbsp()?;
        Ok(Self {
            id,
            sps_id,
            dependent_slices,
            output_flag_present,
            extra_slice_header_bits,
            sign_data_hiding,
            cabac_init_present,
            default_references,
            initial_qp,
            constrained_intra,
            transform_skip,
            cu_qp_delta_depth,
            chroma_qp_offsets,
            slice_chroma_qp_offsets,
            weighted_prediction,
            weighted_biprediction,
            transquant_bypass,
            entropy_sync,
            tiles,
            loop_filter_across_slices,
            deblocking,
            scaling_lists,
            lists_modification,
            parallel_merge_log2,
            slice_header_extension,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_pps_and_truncation() {
        let hex =
            "42010101600000030090000003000003001ea020810596566924caf0168080000003008000000c84";
        let nal: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let sps = Sps::parse(&nal, 1024).unwrap();
        let data = [0x44, 1, 0xc1, 0x72, 0xb4, 0x22, 0x40];
        let pps = Pps::parse(&data, &sps, 1024).unwrap();
        assert_eq!(pps.id, 0);
        assert_eq!(pps.sps_id, 0);
        assert_eq!(pps.initial_qp, 26);
        assert_eq!(pps.default_references, [1, 1]);
        assert_eq!(pps.tiles, None);
        assert_eq!(pps.parallel_merge_log2, 2);
        for end in 0..data.len() {
            assert!(Pps::parse(&data[..end], &sps, 1024).is_err());
        }
    }
    #[test]
    fn uniform_and_explicit_tiles_partition_ctus() {
        assert_eq!(
            tile_axis(&mut BitReader::new(&[]), 7, 3, true).unwrap(),
            vec![2, 2, 3]
        );
        // column_width_minus1 values 0,2 -> sizes 1,3,3.
        assert_eq!(
            tile_axis(&mut BitReader::new(&[0xb0]), 7, 3, false).unwrap(),
            vec![1, 3, 3]
        );
        assert!(tile_axis(&mut BitReader::new(&[0x38]), 7, 2, false).is_err());
    }
}
