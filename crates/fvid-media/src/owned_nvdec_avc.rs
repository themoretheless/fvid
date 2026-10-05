//! Translate FVid-owned AVC syntax into NVDEC picture parameters.
use fvid_codecs::codec::{
    avc::{PictureOrder, Pps, SliceGroups, Sps},
    avc_access_unit::Slice,
    avc_scaling::ScalingMatrices,
    avc_slice::SliceType,
};
use fvid_cuda::{
    NvdecSession,
    nvdec_sdk::{CUVIDH264PICPARAMS, CUVIDPICPARAMS},
};

/// Reference state before marking the current picture.
#[derive(Clone, Copy)]
pub struct AvcReference {
    pub slot: u32,
    pub frame_index: i32,
    pub long_term: bool,
    pub non_existing: bool,
    pub field_order: [i32; 2],
}
/// Owns the slice bytes and offsets used during synchronous driver submission.
pub struct AvcPicture {
    syntax: CUVIDPICPARAMS,
    bytes: Vec<u8>,
    offsets: Vec<u32>,
}
impl AvcPicture {
    pub fn prepare(
        sps: &Sps,
        pps: &Pps,
        slices: &[Slice<'_>],
        slot: u32,
        field_order: [i32; 2],
        references: &[AvcReference],
        max_bytes: usize,
    ) -> Result<Self, String> {
        let first = slices.first().ok_or("NVDEC AVC picture has no slices")?;
        if !sps.frame_mbs_only
            || sps.chroma_format != 1
            || sps.separate_colour_plane
            || sps.bit_depth_luma != 8
            || sps.bit_depth_chroma != 8
            || !matches!(pps.slice_groups, SliceGroups::Single)
        {
            return Err(
                "NVDEC AVC adapter currently requires progressive eight-bit 4:2:0 without FMO"
                    .into(),
            );
        }
        if pps.sps_id != sps.id
            || first.header.pps_id != pps.id
            || !(4..=16).contains(&sps.frame_num_bits)
            || sps.max_num_ref_frames > 16
            || references.len() > sps.max_num_ref_frames as usize
            || references.len() > 16
            || !(1..=32).contains(&pps.default_refs_l0)
            || !(1..=32).contains(&pps.default_refs_l1)
            || !(0..=51).contains(&pps.initial_qp)
            || !(0..=51).contains(&pps.initial_qs)
            || pps.weighted_bipred > 2
            || first.header.frame_num >= (1 << sps.frame_num_bits)
            || slot > i32::MAX as u32
        {
            return Err("NVDEC AVC parameter sets/reference slots are inconsistent".into());
        }
        let mut h = CUVIDH264PICPARAMS::default();
        h.log2_max_frame_num_minus4 = i32::from(sps.frame_num_bits - 4);
        match &sps.picture_order {
            PictureOrder::Lsb { bits } => {
                if !(4..=16).contains(bits) {
                    return Err("invalid AVC POC width".into());
                }
                h.log2_max_pic_order_cnt_lsb_minus4 = i32::from(*bits - 4);
            }
            PictureOrder::Cycle { always_zero, .. } => {
                h.pic_order_cnt_type = 1;
                h.delta_pic_order_always_zero_flag = i32::from(*always_zero);
            }
            PictureOrder::DecodeOrder => h.pic_order_cnt_type = 2,
        }
        h.frame_mbs_only_flag = 1;
        h.direct_8x8_inference_flag = i32::from(sps.direct_8x8_inference);
        h.num_ref_frames = sps.max_num_ref_frames as i32;
        h.qpprime_y_zero_transform_bypass_flag = u8::from(sps.transform_bypass);
        h.entropy_coding_mode_flag = i32::from(pps.cabac);
        h.pic_order_present_flag = i32::from(pps.bottom_field_pic_order_present);
        h.num_ref_idx_l0_active_minus1 = i32::try_from(
            pps.default_refs_l0
                .checked_sub(1)
                .ok_or("invalid AVC reference count")?,
        )
        .map_err(|_| "AVC reference overflow")?;
        h.num_ref_idx_l1_active_minus1 = i32::try_from(
            pps.default_refs_l1
                .checked_sub(1)
                .ok_or("invalid AVC reference count")?,
        )
        .map_err(|_| "AVC reference overflow")?;
        h.weighted_pred_flag = i32::from(pps.weighted_pred);
        h.weighted_bipred_idc = i32::from(pps.weighted_bipred);
        h.pic_init_qp_minus26 = pps.initial_qp - 26;
        h.pic_init_qs_minus26 = i8::try_from(pps.initial_qs - 26).map_err(|_| "AVC QS overflow")?;
        h.deblocking_filter_control_present_flag = i32::from(pps.deblocking_filter_control_present);
        h.redundant_pic_cnt_present_flag = i32::from(pps.redundant_pic_cnt_present);
        h.transform_8x8_mode_flag = i32::from(pps.transform_8x8);
        h.constrained_intra_pred_flag = i32::from(pps.constrained_intra_pred);
        h.chroma_qp_index_offset = pps.chroma_qp_offset;
        h.second_chroma_qp_index_offset = pps.second_chroma_qp_offset;
        h.ref_pic_flag = i32::from(first.header.nal_ref_idc != 0);
        h.frame_num =
            i32::try_from(first.header.frame_num).map_err(|_| "AVC frame number overflow")?;
        h.CurrFieldOrderCnt = field_order;
        for entry in &mut h.dpb {
            entry.PicIdx = -1;
        }
        for (index, r) in references.iter().enumerate() {
            if !r.non_existing
                && (r.slot > i32::MAX as u32
                    || r.slot == slot
                    || references[..index]
                        .iter()
                        .any(|other| !other.non_existing && other.slot == r.slot))
            {
                return Err("NVDEC AVC duplicate or invalid reference slot".into());
            }
            h.dpb[index].PicIdx = if r.non_existing { -1 } else { r.slot as i32 };
            h.dpb[index].FrameIdx = r.frame_index;
            h.dpb[index].is_long_term = i32::from(r.long_term);
            h.dpb[index].not_existing = i32::from(r.non_existing);
            h.dpb[index].used_for_reference = 3;
            h.dpb[index].FieldOrderCnt = r.field_order;
        }
        let matrices = ScalingMatrices::new(sps, pps).map_err(|e| e.to_string())?;
        h.WeightScale4x4 = matrices.four;
        h.WeightScale8x8 = matrices.eight;
        let mut bytes = Vec::new();
        let mut offsets = Vec::new();
        let mut intra = true;
        for slice in slices {
            if slice.header.field_pic
                || slice.header.frame_num != first.header.frame_num
                || slice.header.pps_id != pps.id
                || slice.header.idr != first.header.idr
                || slice.header.nal_ref_idc != first.header.nal_ref_idc
            {
                return Err("NVDEC AVC slices do not describe one progressive picture".into());
            }
            intra &= matches!(slice.header.slice_type, SliceType::I | SliceType::Si);
            let size = bytes
                .len()
                .checked_add(3)
                .and_then(|n| n.checked_add(slice.nal.len()))
                .ok_or("AVC picture byte overflow")?;
            if size > max_bytes || size > u32::MAX as usize {
                return Err("NVDEC AVC picture exceeds byte limit".into());
            }
            offsets.try_reserve(1).map_err(|e| e.to_string())?;
            bytes
                .try_reserve(size - bytes.len())
                .map_err(|e| e.to_string())?;
            offsets.push(bytes.len() as u32);
            bytes.extend_from_slice(&[0, 0, 1]);
            bytes.extend_from_slice(slice.nal);
        }
        let mut syntax = CUVIDPICPARAMS::default();
        syntax.PicWidthInMbs = i32::try_from(sps.width_mbs).map_err(|_| "AVC width overflow")?;
        syntax.FrameHeightInMbs =
            i32::try_from(sps.height_map_units).map_err(|_| "AVC height overflow")?;
        syntax.CurrPicIdx = slot as i32;
        syntax.ref_pic_flag = h.ref_pic_flag;
        syntax.intra_pic_flag = i32::from(intra);
        syntax.CodecSpecific.h264 = h;
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
    /// Session must be H.264 with matching coded geometry. All reference slots
    /// must describe its live pictures and remain reserved through completion.
    pub unsafe fn submit(&self, session: &mut NvdecSession) -> Result<(), String> {
        // SAFETY: Owned arrays remain live through the call; caller guarantees
        // session codec, geometry and reference ownership.
        unsafe { session.submit_picture(&mut self.parameters()) }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn synthetic_owned_avc_picture_decodes_and_maps_on_nvidia() {
        use fvid_codecs::codec::{avc_access_unit, avc_poc::PocDecoder, config::AvcConfig};
        let mut reader = crate::owned_mp4::Mp4Reader::open(
            std::io::Cursor::new(include_bytes!(
                "../../../tests/fixtures/playback-errors/cuda-h264.mp4"
            )),
            crate::owned_mp4::Limits::default(),
        )
        .unwrap();
        let config = AvcConfig::parse(&reader.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let length = config.length_size;
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let slices = avc_access_unit::prepare(&packet, length, &sps, &pps, 1 << 20).unwrap();
        assert!(slices[0].header.idr);
        let order = PocDecoder::new()
            .decode(&sps, &slices[0].header)
            .unwrap()
            .before_marking;
        let picture = AvcPicture::prepare(
            &sps,
            &pps,
            &slices,
            0,
            [order.top.unwrap(), order.bottom.unwrap()],
            &[],
            1 << 20,
        )
        .unwrap();
        let (width, height) = sps.coded_dimensions();
        let mut decoder = NvdecSession::open(
            fvid_cuda::CodecDevice::new(0).unwrap(),
            fvid_cuda::NvdecCodec::H264,
            8,
            width,
            height,
            20,
            2,
        )
        .unwrap();
        // SAFETY: First IDR has no references; decoder codec/geometry match SPS.
        unsafe { picture.submit(&mut decoder) }.unwrap();
        // SAFETY: Successfully submitted picture zero remains reserved.
        let surface = unsafe { decoder.map_progressive(0) }.unwrap();
        assert_eq!((surface.width, surface.height), (width, height));
        assert_ne!(surface.pointer, 0);
        assert!(surface.pitch >= width);
        decoder.unmap(surface.slot).unwrap();
        decoder.close().unwrap();
    }
    #[test]
    fn synthetic_mp4_owned_parser_prepares_nvdec_picture_without_driver() {
        use fvid_codecs::codec::{avc_access_unit, config::AvcConfig};
        let mut reader = crate::owned_mp4::Mp4Reader::open(
            std::io::Cursor::new(include_bytes!(
                "../../../tests/fixtures/playback-errors/control.mp4"
            )),
            crate::owned_mp4::Limits::default(),
        )
        .unwrap();
        let config = AvcConfig::parse(&reader.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let length = config.length_size;
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let slices = avc_access_unit::prepare(&packet, length, &sps, &pps, 1 << 20).unwrap();
        let picture = AvcPicture::prepare(&sps, &pps, &slices, 0, [0, 0], &[], 1 << 20).unwrap();
        let params = picture.parameters();
        assert_eq!(params.CurrPicIdx, 0);
        assert_eq!(params.intra_pic_flag, 1);
        assert_eq!(picture.offsets[0], 0);
        assert!(picture.bytes.starts_with(&[0, 0, 1]));
        // SAFETY: prepare selected and initialized the H.264 union member.
        let h = unsafe { params.CodecSpecific.h264 };
        assert!(h.dpb.iter().all(|entry| entry.PicIdx == -1));
        assert_eq!(
            h.WeightScale4x4,
            ScalingMatrices::new(&sps, &pps).unwrap().four
        );
        assert!(AvcPicture::prepare(&sps, &pps, &slices, 0, [0, 0], &[], 1).is_err());
        let r = AvcReference {
            slot: 0,
            frame_index: 0,
            long_term: false,
            non_existing: false,
            field_order: [0, 0],
        };
        assert!(AvcPicture::prepare(&sps, &pps, &slices, 0, [0, 0], &[r], 1 << 20).is_err());
    }
}
