//! Own HEVC parameter-set translation for direct NVDEC picture submission.
use fvid_codecs::codec::{hevc_cabac::SliceType, hevc_slice::SliceHeader};
use fvid_codecs::codec::{hevc_pps::Pps, hevc_scaling::Matrix, hevc_sps::Sps};
use fvid_cuda::nvdec_sdk::CUVIDHEVCPICPARAMS;
use fvid_cuda::{NvdecSession, nvdec_sdk::CUVIDPICPARAMS};

/// A live reference owned by the caller's DPB scheduler.
#[derive(Clone, Copy, Debug)]
pub struct HevcReference {
    pub slot: u32,
    pub poc: i32,
    pub long_term: bool,
}

/// Owns the Annex B slice bytes and offsets through synchronous submission.
pub struct HevcPicture {
    syntax: CUVIDPICPARAMS,
    bytes: Vec<u8>,
    offsets: Vec<u32>,
}
impl HevcPicture {
    pub fn prepare(
        sps: &Sps,
        pps: &Pps,
        slices: &[&[u8]],
        slot: u32,
        poc: i32,
        references: &[HevcReference],
        max_bytes: usize,
    ) -> Result<Self, String> {
        if slices.is_empty() || slices.len() > u32::MAX as usize || slot >= 32 {
            return Err("invalid NVDEC HEVC slice count/current slot".into());
        }
        let mut h = configuration(sps, pps)?;
        let first =
            SliceHeader::parse(slices[0], sps, pps, max_bytes).map_err(|e| e.to_string())?;
        if !first.first || first.dependent || references.len() > 16 {
            return Err(
                "NVDEC HEVC picture requires first independent slice and bounded DPB".into(),
            );
        }
        if first.nal.is_idr() && (poc != 0 || !references.is_empty()) {
            return Err("NVDEC HEVC IDR requires zero POC and empty reference state".into());
        }
        if !first.nal.is_idr() && poc.rem_euclid(1 << sps.poc_bits) as u32 != first.poc_lsb {
            return Err("NVDEC HEVC POC disagrees with slice".into());
        }
        h.IrapPicFlag = u8::from(first.nal.is_irap());
        h.IdrPicFlag = u8::from(first.nal.is_idr());
        h.CurrPicOrderCntVal = poc;
        h.NumBitsForShortTermRPSInSlice = i32::try_from(first.short_term_bit_length)
            .map_err(|_| "HEVC RPS bit length exceeds driver field")?;
        h.NumDeltaPocsOfRefRpsIdx = i32::try_from(first.short_term_predictor_delta_pocs)
            .map_err(|_| "HEVC RPS predictor exceeds driver field")?;
        for (index, reference) in references.iter().enumerate() {
            if reference.slot >= 32
                || reference.slot == slot
                || reference.poc == poc
                || references[..index]
                    .iter()
                    .any(|r| r.slot == reference.slot || r.poc == reference.poc)
            {
                return Err("NVDEC HEVC reference slots/POCs are inconsistent".into());
            }
            h.RefPicIdx[index] = reference.slot as i32;
            h.PicOrderCntVal[index] = reference.poc;
            h.IsLongTerm[index] = u8::from(reference.long_term);
        }
        for reference in &first.short_term {
            let target = poc
                .checked_add(reference.delta_poc)
                .ok_or("HEVC reference POC overflow")?;
            let index = references
                .iter()
                .position(|r| r.poc == target)
                .ok_or("NVDEC HEVC RPS reference has no live slot")?;
            if references[index].long_term {
                return Err("NVDEC HEVC short-term RPS has a long-term slot".into());
            }
            if reference.used {
                let (count, set) = if reference.delta_poc < 0 {
                    (&mut h.NumPocStCurrBefore, &mut h.RefPicSetStCurrBefore)
                } else {
                    (&mut h.NumPocStCurrAfter, &mut h.RefPicSetStCurrAfter)
                };
                let output = set
                    .get_mut(*count as usize)
                    .ok_or("NVDEC HEVC current reference set exceeds eight entries")?;
                *output = index as u8;
                *count += 1;
            }
        }
        let dpb_pocs: Vec<_> = references.iter().map(|r| r.poc).collect();
        let mut long_pocs = Vec::new();
        for entry in &first.long_term {
            let Some(target) = entry.resolve(poc, sps.poc_bits, &dpb_pocs)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            if long_pocs.contains(&target)
                || first.short_term.iter().any(|r| poc.checked_add(r.delta_poc) == Some(target))
            {
                return Err("NVDEC HEVC long-term RPS repeats a reference POC".into());
            }
            long_pocs.push(target);
            let Some(index) = references.iter().position(|r| r.poc == target) else {
                if entry.used {
                    return Err("NVDEC HEVC long-term RPS has no live slot".into());
                }
                continue;
            };
            if !references[index].long_term {
                return Err("NVDEC HEVC long-term RPS has a short-term slot".into());
            }
            if entry.used {
                let output = h.RefPicSetLtCurr.get_mut(h.NumPocLtCurr as usize)
                    .ok_or("NVDEC HEVC long-term current set exceeds eight entries")?;
                *output = index as u8;
                h.NumPocLtCurr += 1;
            }
        }
        h.NumPocTotalCurr = h.NumPocStCurrBefore + h.NumPocStCurrAfter + h.NumPocLtCurr;
        let mut bytes = Vec::new();
        let mut offsets = Vec::new();
        offsets
            .try_reserve(slices.len())
            .map_err(|e| e.to_string())?;
        let mut previous = first.clone();
        for (index, nal) in slices.iter().enumerate() {
            let header = if index == 0 {
                first.clone()
            } else {
                SliceHeader::parse_with_previous(nal, sps, pps, max_bytes, Some(&previous))
                    .map_err(|e| e.to_string())?
            };
            if index > 0 && (header.first || header.address <= previous.address)
                || header.nal != first.nal
                || header.poc_lsb != first.poc_lsb
                || header.short_term != first.short_term
                || header.long_term != first.long_term
                || header.picture_output != first.picture_output
                || header.no_output_of_prior_pictures != first.no_output_of_prior_pictures
            {
                return Err("NVDEC HEVC slices disagree on picture identity/order".into());
            }
            // Mixed intra/inter slice types require separate qualification.
            if header.slice_type != first.slice_type {
                return Err("NVDEC HEVC mixed slice types are not qualified".into());
            }
            let length = bytes
                .len()
                .checked_add(3)
                .and_then(|n| n.checked_add(nal.len()))
                .filter(|n| *n <= max_bytes && *n <= u32::MAX as usize)
                .ok_or("NVDEC HEVC picture exceeds bitstream limit")?;
            bytes
                .try_reserve(length - bytes.len())
                .map_err(|e| e.to_string())?;
            offsets.push(bytes.len() as u32);
            bytes.extend_from_slice(&[0, 0, 1]);
            bytes.extend_from_slice(nal);
            previous = header;
        }
        let mut syntax = CUVIDPICPARAMS::default();
        syntax.PicWidthInMbs = sps.dimensions[0].div_ceil(16) as i32;
        syntax.FrameHeightInMbs = sps.dimensions[1].div_ceil(16) as i32;
        syntax.CurrPicIdx = slot as i32;
        syntax.intra_pic_flag = i32::from(first.slice_type == SliceType::I);
        syntax.ref_pic_flag = i32::from(first.nal.is_irap() || first.nal.unit_type & 1 != 0);
        syntax.CodecSpecific.hevc = h;
        Ok(Self {
            syntax,
            bytes,
            offsets,
        })
    }
    fn parameters(&self) -> CUVIDPICPARAMS {
        let mut params = self.syntax;
        params.nBitstreamDataLen = self.bytes.len() as u32;
        params.pBitstreamData = self.bytes.as_ptr();
        params.nNumSlices = self.offsets.len() as u32;
        params.pSliceDataOffsets = self.offsets.as_ptr();
        params
    }
    /// # Safety
    /// Session must use HEVC with matching coded geometry/depth. All referenced
    /// slots must be live and reserved, including the output slot, until complete.
    pub unsafe fn submit(&self, session: &mut NvdecSession) -> Result<(), String> {
        unsafe { session.submit_picture(&mut self.parameters()) }
    }
}

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
        || sps.vui.as_ref().is_some_and(|vui| vui.field_sequence)
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
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA NVDEC with HEVC Main/Main10 support"]
    fn synthetic_owned_hevc_idr_submits_and_maps_on_nvidia() {
        use fvid_codecs::codec::{
            config::{HevcConfig, NalUnits},
            hevc_nal::NalHeader,
        };
        for bytes in [
            include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc.mp4").as_slice(),
            include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc-main10.mp4").as_slice(),
        ] {
            let (sps, pps) = sets(bytes);
            let mut reader =
                crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
                    .unwrap();
            let length = HevcConfig::parse(&reader.tracks()[0].configuration)
                .unwrap()
                .length_size;
            let mut packet = Vec::new();
            reader.read_packet(0, 0, &mut packet).unwrap();
            let slices: Vec<_> = NalUnits::new(&packet, length)
                .unwrap()
                .map(Result::unwrap)
                .filter(|nal| NalHeader::parse(nal).unwrap().is_vcl())
                .collect();
            assert!(NalHeader::parse(slices[0]).unwrap().is_idr());
            let picture = HevcPicture::prepare(&sps, &pps, &slices, 0, 0, &[], 1 << 20).unwrap();
            let [width, height] = sps.dimensions;
            let mut decoder = NvdecSession::open(
                fvid_cuda::CodecDevice::new(0).unwrap(),
                fvid_cuda::NvdecCodec::Hevc,
                sps.depth[0],
                width,
                height,
                20,
                2,
            )
            .unwrap();
            // SAFETY: Matching HEVC geometry/depth and an IDR without references.
            unsafe { picture.submit(&mut decoder) }.unwrap();
            // SAFETY: Submitted slot zero stays reserved until unmap.
            let surface = unsafe { decoder.map_progressive(0) }.unwrap();
            assert_eq!((surface.width, surface.height), (width, height));
            assert_ne!(surface.pointer, 0);
            assert!(surface.pitch >= width * if sps.depth[0] == 10 { 2 } else { 1 });
            decoder.unmap(surface.slot).unwrap();
            decoder.close().unwrap();
        }
    }
    #[test]
    fn mixed_reference_fixture_translates_both_driver_current_sets() {
        use fvid_codecs::codec::{config::NalUnits, hevc_decoder::HevcDecoder, hevc_nal::NalHeader};
        let data = include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-mixed-rext8.mp4");
        let mut reader = crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
        let decoder = HevcDecoder::from_configuration(&reader.tracks()[0].configuration, 16 << 20).unwrap();
        let (sps, pps) = decoder.parameters();
        let mut packet = Vec::new(); reader.read_packet(0, 2, &mut packet).unwrap();
        let slices: Vec<_> = NalUnits::new(&packet, 4).unwrap().map(Result::unwrap)
            .filter(|n| NalHeader::parse(n).unwrap().is_vcl()).collect();
        let refs = [HevcReference { slot: 4, poc: 0, long_term: false },
                    HevcReference { slot: 7, poc: 1, long_term: true }];
        let picture = HevcPicture::prepare(sps, pps, &slices, 31, 2, &refs, 1 << 20).unwrap();
        let h = unsafe { picture.parameters().CodecSpecific.hevc };
        assert_eq!(h.NumPocStCurrBefore, 1);
        assert_eq!(h.NumPocStCurrAfter, 0);
        assert_eq!(h.NumPocLtCurr, 1);
        assert_eq!(h.NumPocTotalCurr, 2);
        assert_eq!(h.RefPicSetStCurrBefore[0], 0);
        assert_eq!(h.RefPicSetLtCurr[0], 1);
        assert_eq!(&h.IsLongTerm[..2], &[0, 1]);
        assert_eq!(&h.RefPicIdx[..2], &[4, 7]);
    }
    #[test]
    fn synthetic_long_term_submission_sets_driver_classification_and_current_set() {
        use fvid_codecs::codec::{config::NalUnits, hevc_decoder::HevcDecoder, hevc_nal::NalHeader};
        for data in [include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-rext8.mp4").as_slice(),
                     include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-sps-rext8.mp4").as_slice()] {
        let mut reader = crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
        let software = HevcDecoder::from_configuration(&reader.tracks()[0].configuration, 16 << 20).unwrap();
        let (sps, pps) = software.parameters();
        let mut packet = Vec::new();
        for sample in 0..3 {
            reader.read_packet(0, sample, &mut packet).unwrap();
            let slices: Vec<_> = NalUnits::new(&packet, 4).unwrap().map(Result::unwrap)
                .filter(|nal| NalHeader::parse(nal).unwrap().is_vcl()).collect();
            let refs = if sample == 0 { Vec::new() } else {
                vec![HevcReference { slot: (sample - 1) as u32, poc: sample as i32 - 1, long_term: true }]
            };
            let picture = HevcPicture::prepare(sps, pps, &slices, 31, sample as i32, &refs, 1 << 20).unwrap();
            let h = unsafe { picture.parameters().CodecSpecific.hevc };
            assert_eq!(h.num_long_term_ref_pics_sps as usize, sps.long_term.len());
            assert_eq!(h.NumPocStCurrBefore, 0);
            assert_eq!(h.NumPocStCurrAfter, 0);
            assert_eq!(h.NumPocLtCurr, i32::from(sample != 0));
            assert_eq!(h.NumPocTotalCurr, h.NumPocLtCurr);
            if sample != 0 {
                assert_eq!(h.RefPicSetLtCurr[0], 0);
                assert_eq!(h.RefPicIdx[0], (sample - 1) as i32);
                assert_eq!(h.PicOrderCntVal[0], sample as i32 - 1);
                assert_eq!(h.IsLongTerm[0], 1);
                let mut invalid = refs.clone(); invalid[0].long_term = false;
                assert!(HevcPicture::prepare(sps, pps, &slices, 31, sample as i32, &invalid, 1 << 20)
                    .err().unwrap().contains("short-term slot"));
                assert!(HevcPicture::prepare(sps, pps, &slices, 31, sample as i32, &[], 1 << 20).is_err());
            }
        }
        }
    }
    #[test]
    fn synthetic_picture_submission_owns_bytes_and_resolves_live_references() {
        use fvid_codecs::codec::{
            config::{HevcConfig, NalUnits},
            hevc_nal::NalHeader,
        };
        for bytes in [
            include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
            include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
        ] {
            let (sps, pps) = sets(bytes);
            let mut reader =
                crate::owned_mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
                    .unwrap();
            let length = HevcConfig::parse(&reader.tracks()[0].configuration)
                .unwrap()
                .length_size;
            let mut packet = Vec::new();
            let mut saw_inter = false;
            for sample in 0..reader.tracks()[0].samples.len() {
                reader.read_packet(0, sample, &mut packet).unwrap();
                let slices: Vec<_> = NalUnits::new(&packet, length)
                    .unwrap()
                    .map(Result::unwrap)
                    .filter(|nal| NalHeader::parse(nal).unwrap().is_vcl())
                    .collect();
                let header = SliceHeader::parse(slices[0], &sps, &pps, 1 << 20).unwrap();
                let poc = header.poc_lsb as i32;
                let refs: Vec<_> = header
                    .short_term
                    .iter()
                    .enumerate()
                    .map(|(i, r)| HevcReference {
                        slot: i as u32,
                        poc: poc + r.delta_poc,
                        long_term: false,
                    })
                    .collect();
                let picture =
                    HevcPicture::prepare(&sps, &pps, &slices, 31, poc, &refs, 1 << 20).unwrap();
                let params = picture.parameters();
                assert_eq!(params.nNumSlices as usize, slices.len());
                assert_eq!(params.pBitstreamData, picture.bytes.as_ptr());
                assert_eq!(&picture.bytes[..3], &[0, 0, 1]);
                assert!(HevcPicture::prepare(&sps, &pps, &slices, 31, poc, &refs, 1).is_err());
                let h = unsafe { params.CodecSpecific.hevc };
                assert_eq!(h.CurrPicOrderCntVal, poc);
                assert_eq!(
                    h.NumPocTotalCurr as usize,
                    header.short_term.iter().filter(|r| r.used).count()
                );
                if !refs.is_empty() {
                    saw_inter = true;
                    assert!(
                        HevcPicture::prepare(&sps, &pps, &slices, 31, poc, &[], 1 << 20).is_err()
                    );
                    let mut aliased = refs.clone();
                    aliased[0].slot = 31;
                    assert!(
                        HevcPicture::prepare(&sps, &pps, &slices, 31, poc, &aliased, 1 << 20)
                            .is_err()
                    );
                    break;
                }
            }
            assert!(saw_inter);
        }
    }
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
