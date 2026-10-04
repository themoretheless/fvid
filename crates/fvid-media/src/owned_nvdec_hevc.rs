//! Own HEVC parameter-set translation for direct NVDEC picture submission.
use fvid_codecs::codec::{hevc_pps::Pps, hevc_scaling::Matrix, hevc_sps::Sps};
use fvid_cuda::nvdec_sdk::CUVIDHEVCPICPARAMS;

/// Populate codec configuration, not picture/RPS state. A submission adapter
/// must additionally supply slice bytes, POC and reserved live reference slots.
pub fn configuration(sps: &Sps, pps: &Pps) -> Result<CUVIDHEVCPICPARAMS, String> {
    if sps.chroma_format != 1
        || sps.separate_colour_plane
        || !matches!(sps.depth, [8, 8] | [10, 10])
        || sps.intra_smoothing_disabled
        || sps.transform_skip_rotation
        || sps.transform_skip_context
        || sps.implicit_rdpcm
        || sps.explicit_rdpcm
        || sps.persistent_rice
    {
        return Err("NVDEC HEVC configuration currently requires Main/Main10 4:2:0 tools".into());
    }
    if pps.sps_id != sps.id
        || sps
            .dimensions
            .iter()
            .any(|size| *size == 0 || *size > i32::MAX as u32)
        || !(3..=6).contains(&sps.coding_block_log2[0])
        || !(sps.coding_block_log2[0]..=6).contains(&sps.coding_block_log2[1])
        || !(2..=5).contains(&sps.transform_block_log2[0])
        || !(sps.transform_block_log2[0]..=5).contains(&sps.transform_block_log2[1])
        || !(4..=16).contains(&sps.poc_bits)
        || sps.short_term.len() > 64
        || sps.long_term.len() > 32
        || pps
            .default_references
            .iter()
            .any(|count| !(1..=15).contains(count))
        || !(2..=5).contains(&pps.transform_skip_max_log2)
        || !(2..=6).contains(&pps.parallel_merge_log2)
    {
        return Err("NVDEC HEVC parameter sets are inconsistent/out of range".into());
    }
    let mut h = CUVIDHEVCPICPARAMS::default();
    h.RefPicIdx = [-1; 16];
    h.RefPicSetStCurrBefore = [255; 8];
    h.RefPicSetStCurrAfter = [255; 8];
    h.RefPicSetLtCurr = [255; 8];
    h.pic_width_in_luma_samples = sps.dimensions[0] as i32;
    h.pic_height_in_luma_samples = sps.dimensions[1] as i32;
    h.log2_min_luma_coding_block_size_minus3 = sps.coding_block_log2[0] - 3;
    h.log2_diff_max_min_luma_coding_block_size =
        sps.coding_block_log2[1] - sps.coding_block_log2[0];
    h.log2_min_transform_block_size_minus2 = sps.transform_block_log2[0] - 2;
    h.log2_diff_max_min_transform_block_size =
        sps.transform_block_log2[1] - sps.transform_block_log2[0];
    h.max_transform_hierarchy_depth_inter = sps.transform_hierarchy_depth[0];
    h.max_transform_hierarchy_depth_intra = sps.transform_hierarchy_depth[1];
    h.log2_max_pic_order_cnt_lsb_minus4 = sps.poc_bits - 4;
    h.num_short_term_ref_pic_sets = sps.short_term.len() as u8;
    h.num_long_term_ref_pics_sps = sps.long_term.len() as u8;
    h.bit_depth_luma_minus8 = sps.depth[0] - 8;
    h.bit_depth_chroma_minus8 = sps.depth[1] - 8;
    h.log2_max_transform_skip_block_size_minus2 = pps.transform_skip_max_log2 - 2;
    h.log2_sao_offset_scale_luma = pps.sao_offset_scale[0];
    h.log2_sao_offset_scale_chroma = pps.sao_offset_scale[1];
    macro_rules! flag {
        ($field:ident, $value:expr) => {
            h.$field = u8::from($value);
        };
    }
    flag!(
        strong_intra_smoothing_enabled_flag,
        sps.strong_intra_smoothing
    );
    flag!(amp_enabled_flag, sps.amp);
    flag!(separate_colour_plane_flag, sps.separate_colour_plane);
    flag!(long_term_ref_pics_present_flag, sps.long_term_present);
    flag!(sps_temporal_mvp_enabled_flag, sps.temporal_mvp);
    flag!(sample_adaptive_offset_enabled_flag, sps.sao);
    flag!(scaling_list_enable_flag, sps.scaling_lists_enabled);
    flag!(
        high_precision_offsets_enabled_flag,
        sps.high_precision_offsets
    );
    flag!(dependent_slice_segments_enabled_flag, pps.dependent_slices);
    flag!(
        slice_segment_header_extension_present_flag,
        pps.slice_header_extension
    );
    flag!(sign_data_hiding_enabled_flag, pps.sign_data_hiding);
    flag!(cu_qp_delta_enabled_flag, pps.cu_qp_delta_depth.is_some());
    h.diff_cu_qp_delta_depth = pps.cu_qp_delta_depth.unwrap_or(0);
    h.init_qp_minus26 = i8::try_from(pps.initial_qp.checked_sub(26).ok_or("HEVC QP overflow")?)
        .map_err(|_| "HEVC QP exceeds NVDEC field")?;
    h.pps_cb_qp_offset = pps.chroma_qp_offsets[0];
    h.pps_cr_qp_offset = pps.chroma_qp_offsets[1];
    flag!(constrained_intra_pred_flag, pps.constrained_intra);
    flag!(weighted_pred_flag, pps.weighted_prediction);
    flag!(weighted_bipred_flag, pps.weighted_biprediction);
    flag!(transform_skip_enabled_flag, pps.transform_skip);
    flag!(transquant_bypass_enabled_flag, pps.transquant_bypass);
    flag!(entropy_coding_sync_enabled_flag, pps.entropy_sync);
    h.log2_parallel_merge_level_minus2 = pps.parallel_merge_log2 - 2;
    h.num_extra_slice_header_bits = pps.extra_slice_header_bits;
    flag!(
        loop_filter_across_slices_enabled_flag,
        pps.loop_filter_across_slices
    );
    flag!(output_flag_present_flag, pps.output_flag_present);
    h.num_ref_idx_l0_default_active_minus1 = pps.default_references[0] - 1;
    h.num_ref_idx_l1_default_active_minus1 = pps.default_references[1] - 1;
    flag!(lists_modification_present_flag, pps.lists_modification);
    flag!(cabac_init_present_flag, pps.cabac_init_present);
    flag!(
        pps_slice_chroma_qp_offsets_present_flag,
        pps.slice_chroma_qp_offsets
    );
    flag!(
        deblocking_filter_override_enabled_flag,
        pps.deblocking.override_enabled
    );
    flag!(pps_deblocking_filter_disabled_flag, pps.deblocking.disabled);
    h.pps_beta_offset_div2 = pps.deblocking.offsets_div2[0];
    h.pps_tc_offset_div2 = pps.deblocking.offsets_div2[1];
    if let Some(pcm) = &sps.pcm {
        if pcm.depth.contains(&0) || pcm.block_log2[0] < 3 || pcm.block_log2[1] < pcm.block_log2[0]
        {
            return Err("invalid HEVC PCM configuration".into());
        }
        h.pcm_enabled_flag = 1;
        h.log2_min_pcm_luma_coding_block_size_minus3 = pcm.block_log2[0] - 3;
        h.log2_diff_max_min_pcm_luma_coding_block_size = pcm.block_log2[1] - pcm.block_log2[0];
        h.pcm_sample_bit_depth_luma_minus1 = pcm.depth[0] - 1;
        h.pcm_sample_bit_depth_chroma_minus1 = pcm.depth[1] - 1;
        flag!(pcm_loop_filter_disabled_flag, pcm.loop_filter_disabled);
    }
    if let Some(tiles) = &pps.tiles {
        let side = 1u32 << sps.coding_block_log2[1];
        axis(
            &tiles.column_widths,
            sps.dimensions[0].div_ceil(side),
            &mut h.column_width_minus1,
        )?;
        axis(
            &tiles.row_heights,
            sps.dimensions[1].div_ceil(side),
            &mut h.row_height_minus1,
        )?;
        h.tiles_enabled_flag = 1;
        h.num_tile_columns_minus1 = (tiles.column_widths.len() - 1) as u8;
        h.num_tile_rows_minus1 = (tiles.row_heights.len() - 1) as u8;
        flag!(
            loop_filter_across_tiles_enabled_flag,
            tiles.loop_filter_across
        );
        // Own parser retains derived extents, so explicit spacing represents
        // both original uniform and nonuniform tile grids without guessing.
        h.uniform_spacing_flag = 0;
    }
    let lists = pps.scaling_lists.as_ref().unwrap_or(&sps.scaling_lists);
    for id in 0..6 {
        h.ScalingList4x4[id] = raster::<16>(lists.matrix(0, id).map_err(|e| e.to_string())?, 4);
        h.ScalingList8x8[id] = raster::<64>(lists.matrix(1, id).map_err(|e| e.to_string())?, 8);
        let matrix = lists.matrix(2, id).map_err(|e| e.to_string())?;
        h.ScalingList16x16[id] = raster::<64>(matrix, 8);
        h.ScalingListDCCoeff16x16[id] = matrix.dc;
    }
    for (index, id) in [0, 3].into_iter().enumerate() {
        let matrix = lists.matrix(3, id).map_err(|e| e.to_string())?;
        h.ScalingList32x32[index] = raster::<64>(matrix, 8);
        h.ScalingListDCCoeff32x32[index] = matrix.dc;
    }
    Ok(h)
}
fn axis(widths: &[u32], total: u32, output: &mut [u16; 21]) -> Result<(), String> {
    if widths.is_empty()
        || widths.len() > output.len() + 1
        || widths.contains(&0)
        || widths
            .iter()
            .try_fold(0u32, |sum, width| sum.checked_add(*width))
            != Some(total)
    {
        return Err("invalid HEVC NVDEC tile grid".into());
    }
    for (target, width) in output.iter_mut().zip(widths.iter().take(widths.len() - 1)) {
        *target = u16::try_from(*width - 1).map_err(|_| "HEVC tile extent exceeds NVDEC field")?;
    }
    Ok(())
}
fn raster<const N: usize>(matrix: &Matrix, side: usize) -> [u8; N] {
    let mut output = [0; N];
    let mut index = 0;
    for diagonal in 0..2 * side - 1 {
        for x in 0..side {
            if diagonal >= x && diagonal - x < side {
                let y = diagonal - x;
                output[y * side + x] = matrix.coefficients[index];
                index += 1;
            }
        }
    }
    output
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sets(bytes: &[u8]) -> (Sps, Pps) {
        let reader =
            crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
                .unwrap();
        let config =
            fvid_codecs::codec::config::HevcConfig::parse(&reader.tracks()[0].configuration)
                .unwrap();
        let nal = |kind| {
            config
                .arrays
                .iter()
                .find(|array| array.nal_type == kind)
                .unwrap()
                .units[0]
        };
        let sps = Sps::parse(nal(33), 1 << 20).unwrap();
        let pps = Pps::parse(nal(34), &sps, 1 << 20).unwrap();
        (sps, pps)
    }
    #[test]
    fn synthetic_main_and_main10_configurations_translate_without_libav() {
        for (bytes, depth) in [
            (
                include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
                8,
            ),
            (
                include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
                10,
            ),
        ] {
            let (sps, pps) = sets(bytes);
            let h = configuration(&sps, &pps).unwrap();
            assert_eq!(
                (h.pic_width_in_luma_samples, h.pic_height_in_luma_samples),
                (sps.dimensions[0] as i32, sps.dimensions[1] as i32)
            );
            assert_eq!(
                (h.bit_depth_luma_minus8, h.bit_depth_chroma_minus8),
                (depth - 8, depth - 8)
            );
            assert_eq!(
                h.entropy_coding_sync_enabled_flag,
                u8::from(pps.entropy_sync)
            );
            assert_eq!(
                h.max_transform_hierarchy_depth_inter,
                sps.transform_hierarchy_depth[0]
            );
            assert!(h.RefPicIdx.iter().all(|slot| *slot == -1));
        }
    }
    #[test]
    fn diagonal_scaling_and_tile_extents_are_exact_and_bounded() {
        let matrix = Matrix {
            coefficients: std::array::from_fn(|index| index as u8 + 1),
            dc: 97,
        };
        assert_eq!(
            raster::<16>(&matrix, 4),
            [1, 3, 6, 10, 2, 5, 9, 13, 4, 8, 12, 15, 7, 11, 14, 16]
        );
        let mut output = [0; 21];
        axis(&[2, 3, 1], 6, &mut output).unwrap();
        assert_eq!(&output[..2], &[1, 2]);
        assert!(axis(&[], 0, &mut output).is_err());
        assert!(axis(&[0, 6], 6, &mut output).is_err());
        assert!(axis(&[1; 23], 23, &mut output).is_err());
        assert!(axis(&[2, 3], 6, &mut output).is_err());
    }
}
