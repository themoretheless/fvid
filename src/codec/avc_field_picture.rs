//! Compact PCM and whole-field P-skip reconstruction with complementary weaving.
use super::{
    avc::{Pps, SliceGroups, Sps},
    avc_picture::IntraPicture,
    avc_slice::{SliceHeader, SliceType},
    bits::BitReader,
};
use crate::{Result, invalid, unsupported};

pub struct PcmField {
    pub picture: IntraPicture,
    pub bottom: bool,
    pub frame_num: u32,
    pub pps_id: u32,
}
fn samples(count: usize) -> Result<Vec<u16>> {
    let mut v = Vec::new();
    v.try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate AVC field samples"))?;
    v.resize(count, 0);
    Ok(v)
}
/// Decode compact field planes. The budget covers output samples and coverage;
/// caller-owned headers/RBSP and other pictures are excluded. Only PCM is admitted.
pub fn decode_pcm_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<PcmField> {
    let first = *headers
        .first()
        .ok_or_else(|| invalid("missing AVC field slices"))?;
    if !(8..=14).contains(&sps.bit_depth_luma)
        || sps.frame_mbs_only
        || !first.field_pic
        || first.first_mb != 0
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
    {
        return Err(invalid("invalid AVC PCM field configuration"));
    }
    if pps.cabac || !matches!(pps.slice_groups, SliceGroups::Single) {
        return Err(unsupported(
            "PCM field foundation requires single-group CAVLC",
        ));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize / 2);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("AVC field dimensions overflow"))?;
    let count = pixels / 256;
    if count == 0 || count > 65536 {
        return Err(invalid("invalid AVC PCM field size"));
    }
    if pixels
        .checked_mul(3)
        .and_then(|n| n.checked_add(count))
        .is_none_or(|n| n > budget)
    {
        return Err(invalid("AVC PCM field exceeds memory budget"));
    }
    let mut covered = crate::buffer(count)?;
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: [
            sps.crop[0] as usize,
            sps.crop[1] as usize,
            sps.crop[2] as usize / 2,
            sps.crop[3] as usize / 2,
        ],
        bit_depth: sps.bit_depth_luma,
        y: samples(pixels)?,
        cb: samples(pixels / 4)?,
        cr: samples(pixels / 4)?,
    };
    let mut seen = 0;
    for header in headers {
        if !header.field_pic
            || header.bottom_field != first.bottom_field
            || header.frame_num != first.frame_num
            || header.pps_id != pps.id
            || pps.sps_id != sps.id
            || header.slice_type != SliceType::I
            || header.idr != first.idr
            || header.idr_pic_id != first.idr_pic_id
            || header.poc_lsb != first.poc_lsb
            || header.delta_poc != first.delta_poc
            || header.redundant_pic_cnt != 0
            || (header.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
            || header.first_mb as usize != seen
        {
            return Err(invalid("AVC field slice identity or coverage mismatch"));
        }
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset)?;
        while bits.more_rbsp_data() {
            if seen >= count {
                return Err(invalid("AVC PCM field macroblock exceeds picture"));
            }
            if bits.unsigned_golomb()? != 25 {
                return Err(unsupported(
                    "non-PCM AVC field reconstruction is not connected",
                ));
            }
            while bits.position() % 8 != 0 {
                if bits.bit()? {
                    return Err(invalid("nonzero AVC PCM alignment bit"));
                }
            }
            for (plane, size, width) in [
                (&mut picture.y, 16, w),
                (&mut picture.cb, 8, w / 2),
                (&mut picture.cr, 8, w / 2),
            ] {
                let bx = seen % (w / 16) * size;
                let by = seen / (w / 16) * size;
                for y in 0..size {
                    for x in 0..size {
                        plane[(by + y) * width + bx + x] = bits.read(sps.bit_depth_luma)? as u16;
                    }
                }
            }
            covered[seen] = 1;
            seen += 1;
        }
        bits.finish_rbsp()?;
    }
    if covered.iter().any(|v| *v == 0) {
        return Err(invalid("incomplete AVC PCM field"));
    }
    Ok(PcmField {
        picture,
        bottom: first.bottom_field,
        frame_num: first.frame_num,
        pps_id: first.pps_id,
    })
}
/// Weave complementary compact fields. Budget covers the new frame only.
pub fn weave_pair(first: &PcmField, second: &PcmField, budget: usize) -> Result<IntraPicture> {
    let (top, bottom) = if first.bottom {
        (second, first)
    } else {
        (first, second)
    };
    let a = &top.picture;
    let b = &bottom.picture;
    if top.bottom
        || !bottom.bottom
        || top.frame_num != bottom.frame_num
        || top.pps_id != bottom.pps_id
        || a.coded_width != b.coded_width
        || a.coded_height != b.coded_height
        || a.bit_depth != b.bit_depth
        || a.crop != b.crop
    {
        return Err(invalid("AVC fields are not complementary"));
    }
    if a.coded_width == 0
        || a.coded_width % 16 != 0
        || a.coded_height == 0
        || a.coded_height % 16 != 0
    {
        return Err(invalid("invalid AVC compact field dimensions"));
    }
    let crop_top = a.crop[2]
        .checked_mul(2)
        .ok_or_else(|| invalid("AVC field crop overflow"))?;
    let crop_bottom = a.crop[3]
        .checked_mul(2)
        .ok_or_else(|| invalid("AVC field crop overflow"))?;
    let w = a.coded_width;
    let h = a
        .coded_height
        .checked_mul(2)
        .ok_or_else(|| invalid("AVC woven height overflow"))?;
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("AVC woven dimensions overflow"))?;
    if pixels.checked_mul(3).is_none_or(|n| n > budget) {
        return Err(invalid("AVC woven frame exceeds memory budget"));
    }
    let mut result = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: [a.crop[0], a.crop[1], crop_top, crop_bottom],
        bit_depth: a.bit_depth,
        y: samples(pixels)?,
        cb: samples(pixels / 4)?,
        cr: samples(pixels / 4)?,
    };
    for (dst, top, bottom, width, height) in [
        (&mut result.y, &a.y, &b.y, w, h / 2),
        (&mut result.cb, &a.cb, &b.cb, w / 2, h / 4),
        (&mut result.cr, &a.cr, &b.cr, w / 2, h / 4),
    ] {
        if top.len() != width * height || bottom.len() != width * height {
            return Err(invalid("invalid AVC compact field planes"));
        }
        for row in 0..height {
            dst[2 * row * width..(2 * row + 1) * width]
                .copy_from_slice(&top[row * width..(row + 1) * width]);
            dst[(2 * row + 1) * width..(2 * row + 2) * width]
                .copy_from_slice(&bottom[row * width..(row + 1) * width]);
        }
    }
    Ok(result)
}

/// Reconstruct a whole-field P skip run from its resolved same-parity reference.
/// Coded macroblocks and opposite-parity interpolation require the general field path.
pub fn decode_skip_field(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    reference: &PcmField,
    budget: usize,
) -> Result<PcmField> {
    let h = *headers
        .first()
        .ok_or_else(|| invalid("missing AVC P field"))?;
    if headers.len() != 1
        || pps.cabac
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || h.slice_type != SliceType::P
        || h.weights.is_some()
    {
        return Err(unsupported(
            "general AVC P field reconstruction is not connected",
        ));
    }
    let (w, height) = sps.coded_dimensions();
    let pixels = (w as usize)
        .checked_mul(height as usize / 2)
        .ok_or_else(|| invalid("AVC P field size overflow"))?;
    if !h.field_pic
        || h.first_mb != 0
        || h.redundant_pic_cnt != 0
        || h.pps_id != pps.id
        || pps.sps_id != sps.id
        || reference.picture.coded_width != w as usize
        || reference.picture.coded_height != height as usize / 2
        || reference.picture.bit_depth != sps.bit_depth_luma
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
    {
        return Err(invalid("invalid AVC P field configuration"));
    }
    if reference.bottom != h.bottom_field {
        return Err(unsupported(
            "opposite-parity AVC P field interpolation is not connected",
        ));
    }
    if pixels.checked_mul(3).is_none_or(|n| n > budget) {
        return Err(invalid("AVC P field exceeds memory budget"));
    }
    let mut bits = BitReader::new(&h.rbsp);
    bits.skip(h.entropy_bit_offset)?;
    let skipped = bits.unsigned_golomb()? as usize;
    if skipped != pixels / 256 {
        return Err(unsupported(
            "coded AVC P field macroblocks are not connected",
        ));
    }
    bits.finish_rbsp()?;
    let mut y = samples(pixels)?;
    let mut cb = samples(pixels / 4)?;
    let mut cr = samples(pixels / 4)?;
    if reference.picture.y.len() != y.len()
        || reference.picture.cb.len() != cb.len()
        || reference.picture.cr.len() != cr.len()
    {
        return Err(invalid("invalid AVC P field reference planes"));
    }
    y.copy_from_slice(&reference.picture.y);
    cb.copy_from_slice(&reference.picture.cb);
    cr.copy_from_slice(&reference.picture.cr);
    Ok(PcmField {
        picture: IntraPicture {
            coded_width: w as usize,
            coded_height: height as usize / 2,
            crop: reference.picture.crop,
            bit_depth: sps.bit_depth_luma,
            y,
            cb,
            cr,
        },
        bottom: h.bottom_field,
        frame_num: h.frame_num,
        pps_id: h.pps_id,
    })
}

/// CAVLC P fields: skip runs or partitioned L0 prediction and field-scanned residual.
pub fn decode_p_field(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    reference: &PcmField,
    budget: usize,
) -> Result<PcmField> {
    let references = vec![(reference, 0); headers.len()];
    decode_p_field_resolved(headers, sps, pps, &references, budget)
}
/// One resolved reference field and stable parity-aware identity per slice.
pub fn decode_p_field_resolved(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[(&PcmField, u64)],
    budget: usize,
) -> Result<PcmField> {
    let lists = references
        .iter()
        .map(std::slice::from_ref)
        .collect::<Vec<_>>();
    decode_p_field_lists(headers, sps, pps, &lists, budget)
}
/// Resolved L0 entries for each slice, selected per prediction partition.
pub fn decode_p_field_lists(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[&[(&PcmField, u64)]],
    budget: usize,
) -> Result<PcmField> {
    if references.len() != headers.len() {
        return Err(invalid("AVC field reference contexts differ from slices"));
    }
    let reference = references
        .first()
        .ok_or_else(|| invalid("missing AVC field reference"))?
        .first()
        .ok_or_else(|| invalid("empty AVC field reference list"))?
        .0;
    if headers
        .iter()
        .zip(references)
        .any(|(h, list)| list.len() != h.refs_l0 as usize || list.is_empty() || list.len() > 32)
    {
        return Err(invalid("invalid AVC field active reference list"));
    }
    let h = *headers
        .first()
        .ok_or_else(|| invalid("missing AVC P field"))?;
    let mut bits = BitReader::new(&h.rbsp);
    bits.skip(h.entropy_bit_offset)?;
    let (w, height) = sps.coded_dimensions();
    let count = w as usize / 16 * (height as usize / 32);
    if headers.len() == 1
        && h.weights.is_none()
        && reference.bottom == h.bottom_field
        && bits.unsigned_golomb()? as usize == count
    {
        return decode_skip_field(headers, sps, pps, reference, budget);
    }
    if !h.field_pic
        || h.slice_type != SliceType::P
        || h.redundant_pic_cnt != 0
        || pps.cabac
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || h.disable_deblocking_filter_idc > 2
    {
        return Err(unsupported(
            "general AVC P field reconstruction is not connected",
        ));
    }
    if count == 0 || count > 65536 || headers.len() > count {
        return Err(invalid("invalid AVC P field slice count"));
    }
    for current in headers {
        if !current.field_pic
            || current.slice_type != SliceType::P
            || current.frame_num != h.frame_num
            || current.bottom_field != h.bottom_field
            || current.pps_id != h.pps_id
            || current.idr != h.idr
            || current.idr_pic_id != h.idr_pic_id
            || current.poc_lsb != h.poc_lsb
            || current.delta_poc != h.delta_poc
            || (current.nal_ref_idc == 0) != (h.nal_ref_idc == 0)
            || current.memory_operations != h.memory_operations
            || current.redundant_pic_cnt != 0
            || current.disable_deblocking_filter_idc > 2
        {
            return Err(invalid("AVC P field slice identity mismatch"));
        }
        if let Some(weights) = &current.weights {
            if weights.l0.len() != current.refs_l0 as usize
                || weights.luma_denom > 7
                || weights.chroma_denom > 7
            {
                return Err(invalid("invalid AVC field prediction weights"));
            }
        }
    }
    let filtered = headers.iter().any(|h| h.disable_deblocking_filter_idc != 1);
    let pixels = reference
        .picture
        .coded_width
        .checked_mul(reference.picture.coded_height)
        .ok_or_else(|| invalid("AVC P field size overflow"))?;
    if reference.picture.coded_width != w as usize
        || reference.picture.coded_height != height as usize / 2
        || reference.picture.bit_depth != sps.bit_depth_luma
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
        || h.pps_id != pps.id
        || pps.sps_id != sps.id
    {
        return Err(unsupported(
            "AVC P field reference configuration is not connected",
        ));
    }
    for (candidate, _) in references.iter().flat_map(|list| list.iter()) {
        if candidate.picture.coded_width != reference.picture.coded_width
            || candidate.picture.coded_height != reference.picture.coded_height
            || candidate.picture.crop != reference.picture.crop
            || candidate.picture.bit_depth != reference.picture.bit_depth
        {
            return Err(unsupported("AVC field reference geometry differs"));
        }
    }
    let list_storage = references
        .iter()
        .try_fold(0usize, |n, list| n.checked_add((list.len() - 1) * 32))
        .ok_or_else(|| invalid("AVC field reference list storage overflow"))?;
    let required = pixels
        .checked_mul(3)
        .and_then(|n| {
            count
                .checked_mul(if !filtered { 1024 } else { 2048 })
                .and_then(|m| n.checked_add(m))
        })
        .and_then(|n| n.checked_add(16384))
        .and_then(|n| n.checked_add(list_storage))
        .ok_or_else(|| invalid("AVC P field storage overflow"))?;
    if required > budget {
        return Err(invalid("AVC P field exceeds memory budget"));
    }
    let mut output = PcmField {
        picture: IntraPicture {
            coded_width: w as usize,
            coded_height: height as usize / 2,
            crop: reference.picture.crop,
            bit_depth: sps.bit_depth_luma,
            y: samples(pixels)?,
            cb: samples(pixels / 4)?,
            cr: samples(pixels / 4)?,
        },
        bottom: h.bottom_field,
        frame_num: h.frame_num,
        pps_id: h.pps_id,
    };
    let mut motion =
        super::avc_motion_field::MotionField::new(w as usize, height as usize / 2, count * 960)?;
    let mut coefficients = super::avc_coefficient_field::CoefficientField::new(
        w as usize / 16,
        height as usize / 32,
        count * 64,
    )?;
    let scaling = super::avc_scaling::ScalingMatrices::new(sps, pps)?;

    let mut edges = Vec::new();
    if filtered {
        edges
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC field edges"))?;
    }

    let mut ordered = headers
        .iter()
        .enumerate()
        .map(|(index, h)| (index, *h))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(_, h)| h.first_mb);
    if ordered[0].1.first_mb != 0
        || ordered
            .windows(2)
            .any(|h| h[0].1.first_mb >= h[1].1.first_mb)
        || ordered
            .last()
            .is_some_and(|(_, h)| h.first_mb as usize >= count)
    {
        return Err(invalid("invalid AVC P field slice coverage"));
    }
    for (slice_index, (reference_index, h)) in ordered.iter().enumerate() {
        let list = references[*reference_index];
        let slice_id = slice_index as u32;
        let end = ordered
            .get(slice_index + 1)
            .map_or(count, |(_, h)| h.first_mb as usize);
        let mut qp = h.slice_qp;
        let mut bits = BitReader::new(&h.rbsp);
        bits.skip(h.entropy_bit_offset)?;
        let mut address = h.first_mb as usize;
        while address < end && bits.more_rbsp_data() {
            let skipped = bits.unsigned_golomb()? as usize;
            if skipped > end - address {
                return Err(invalid("AVC P field skip exceeds picture"));
            }
            for step in 0..=skipped {
                if address == end {
                    break;
                }
                let origin = [
                    address % (w as usize / 16) * 16,
                    address / (w as usize / 16) * 16,
                ];
                let mut residual = None;
                let predictions = if step < skipped {
                    coefficients.store(address, slice_id, [0; 16], [[0; 4]; 2])?;
                    vec![([0, 0], [16, 16], motion.decode_p_skip(origin, slice_id)?, 0)]
                } else {
                    let syntax = super::avc_inter::InterSyntax {
                        slice: SliceType::P,
                        active_references: [h.refs_l0, 0],
                        previous_qp: qp,
                        bit_depth: sps.bit_depth_luma,
                        chroma_array_type: 1,
                        transform8_enabled: pps.transform_8x8,
                        direct8_inference: sps.direct_8x8_inference,
                    };
                    let header =
                        super::avc_inter::read_inter_header_field(&mut bits, &syntax, true)?;
                    let c = super::avc_inter_coefficients::read_inter_coefficients_field(
                        &mut bits,
                        header.residual.pattern,
                        header.residual.transform8,
                        coefficients.neighbours(address, slice_id)?,
                        true,
                    )?;
                    coefficients.store(address, slice_id, c.luma_counts, c.chroma_counts)?;
                    qp = header.residual.qp;
                    if header.residual.pattern != 0 {
                        residual = Some((c, header.residual.transform8));
                    }
                    let parts = header.partitions;
                    let neighbour = motion.decode_macroblock(origin, slice_id, &parts)?;
                    parts
                        .iter()
                        .zip(neighbour)
                        .map(|(part, vectors)| match vectors[0] {
                            super::avc_mv::Neighbour::Inter { vector, reference } => Ok((
                                part.origin.map(usize::from),
                                part.size.map(usize::from),
                                vector,
                                reference,
                            )),
                            _ => Err(invalid("missing AVC P field motion")),
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                let mut blocks = [super::avc_boundary::BlockEdge {
                    intra: false,
                    switching_slice: false,
                    nonzero_luma: false,
                    motion: [None; 2],
                }; 16];
                for (offset, dimensions, vector, reference_index) in predictions {
                    let (reference, reference_id) = *list
                        .get(reference_index as usize)
                        .ok_or_else(|| invalid("AVC field partition reference exceeds list"))?;
                    let reference_bottom = reference.bottom;
                    let planes = [
                        (&reference.picture.y, w as usize, height as usize / 2),
                        (&reference.picture.cb, w as usize / 2, height as usize / 4),
                        (&reference.picture.cr, w as usize / 2, height as usize / 4),
                    ];
                    if filtered {
                        for by in offset[1] / 4..(offset[1] + dimensions[1]) / 4 {
                            for bx in offset[0] / 4..(offset[0] + dimensions[0]) / 4 {
                                blocks[by * 4 + bx].motion[0] =
                                    Some(super::avc_boundary::MotionReference {
                                        picture: reference_id,
                                        vector,
                                    });
                            }
                        }
                    }

                    for (component, (plane, pw, ph)) in planes.iter().enumerate() {
                        let divisor = if component == 0 { 1 } else { 2 };
                        let [bw, bh] = dimensions.map(|v| v / divisor);
                        let xy = [
                            (origin[0] + offset[0]) / divisor,
                            (origin[1] + offset[1]) / divisor,
                        ];
                        let mut block = [0u16; 256];
                        let reference = super::avc_motion::ReferencePlane::from_decoded(
                            plane,
                            *pw,
                            *ph,
                            *pw,
                            sps.bit_depth_luma,
                        )?;
                        let mut mv = vector.map(i32::from);
                        if component != 0 {
                            mv[1] += match (reference_bottom, h.bottom_field) {
                                (false, true) => 2,
                                (true, false) => -2,
                                _ => 0,
                            };
                        }
                        if component == 0 {
                            reference.luma(
                                xy.map(|v| v as i32),
                                mv,
                                bw,
                                bh,
                                &mut block[..bw * bh],
                            )?;
                        } else {
                            reference.chroma(
                                xy.map(|v| v as i32),
                                mv,
                                bw,
                                bh,
                                &mut block[..bw * bh],
                            )?;
                        }
                        if let Some(weights) = &h.weights {
                            let entry = weights
                                .l0
                                .get(reference_index as usize)
                                .ok_or_else(|| invalid("missing AVC field partition weight"))?;
                            let ((weight, offset), denominator) = if component == 0 {
                                (entry.luma, weights.luma_denom)
                            } else {
                                (entry.chroma[component - 1], weights.chroma_denom)
                            };
                            super::avc_motion::weight_block(
                                &mut block[..bw * bh],
                                weight,
                                offset,
                                denominator,
                                sps.bit_depth_luma,
                            )?;
                        }
                        let dst = match component {
                            0 => &mut output.picture.y,
                            1 => &mut output.picture.cb,
                            _ => &mut output.picture.cr,
                        };
                        for row in 0..bh {
                            let start = (xy[1] + row) * pw + xy[0];
                            dst[start..start + bw]
                                .copy_from_slice(&block[row * bw..(row + 1) * bw]);
                        }
                    }
                }
                let eight = residual.as_ref().is_some_and(|(_, eight)| *eight);
                if filtered {
                    if let Some((c, _)) = &residual {
                        for i in 0..16 {
                            blocks[i].nonzero_luma = if eight {
                                let bx = (i % 4) / 2 * 2;
                                let by = (i / 4) / 2 * 2;
                                [
                                    by * 4 + bx,
                                    by * 4 + bx + 1,
                                    (by + 1) * 4 + bx,
                                    (by + 1) * 4 + bx + 1,
                                ]
                                .iter()
                                .any(|&j| c.luma_counts[j] != 0)
                            } else {
                                c.luma_counts[i] != 0
                            };
                        }
                    }
                    let bd = 6 * (i32::from(sps.bit_depth_chroma) - 8);
                    edges.push(super::avc_boundary::DecodedBlockEdges {
                        blocks,
                        qp: [
                            qp,
                            i32::from(super::avc_picture::chroma_qp(
                                qp,
                                pps.chroma_qp_offset,
                                sps.bit_depth_chroma,
                            )) - bd,
                            i32::from(super::avc_picture::chroma_qp(
                                qp,
                                pps.second_chroma_qp_offset,
                                sps.bit_depth_chroma,
                            )) - bd,
                        ],
                        offsets: [h.alpha_offset, h.beta_offset],
                        transform8: eight,
                        slice_id,
                        disable_filter: h.disable_deblocking_filter_idc as u8,
                    });
                }
                if let Some((c, eight)) = residual {
                    let mut prediction =
                        super::avc_compensation::Prediction420::empty(sps.bit_depth_luma);
                    for (dst, src, width, size, xy) in [
                        (
                            &mut prediction.y[..],
                            &output.picture.y[..],
                            w as usize,
                            16,
                            origin,
                        ),
                        (
                            &mut prediction.cb[..],
                            &output.picture.cb[..],
                            w as usize / 2,
                            8,
                            origin.map(|v| v / 2),
                        ),
                        (
                            &mut prediction.cr[..],
                            &output.picture.cr[..],
                            w as usize / 2,
                            8,
                            origin.map(|v| v / 2),
                        ),
                    ] {
                        for row in 0..size {
                            let start = (xy[1] + row) * width + xy[0];
                            dst[row * size..(row + 1) * size]
                                .copy_from_slice(&src[start..start + size]);
                        }
                    }
                    let luma = if eight {
                        super::avc_compensation::InterLumaResidual::Blocks8(&c.luma8)
                    } else {
                        super::avc_compensation::InterLumaResidual::Blocks4(&c.luma4)
                    };
                    let qps = [
                        (qp + 6 * (i32::from(sps.bit_depth_luma) - 8)) as u8,
                        super::avc_picture::chroma_qp(
                            qp,
                            pps.chroma_qp_offset,
                            sps.bit_depth_chroma,
                        ),
                        super::avc_picture::chroma_qp(
                            qp,
                            pps.second_chroma_qp_offset,
                            sps.bit_depth_chroma,
                        ),
                    ];
                    let reconstructed = if sps.transform_bypass && qps[0] == 0 {
                        prediction.reconstruct_inter_bypass(luma, &c.chroma_dc, &c.chroma_ac)?
                    } else {
                        prediction.reconstruct_inter(
                            luma,
                            &c.chroma_dc,
                            &c.chroma_ac,
                            qps,
                            &[scaling.four[3], scaling.four[4], scaling.four[5]],
                            &scaling.eight[1],
                        )?
                    };
                    for (dst, src, width, size, xy) in [
                        (
                            &mut output.picture.y[..],
                            &reconstructed.y[..],
                            w as usize,
                            16,
                            origin,
                        ),
                        (
                            &mut output.picture.cb[..],
                            &reconstructed.cb[..],
                            w as usize / 2,
                            8,
                            origin.map(|v| v / 2),
                        ),
                        (
                            &mut output.picture.cr[..],
                            &reconstructed.cr[..],
                            w as usize / 2,
                            8,
                            origin.map(|v| v / 2),
                        ),
                    ] {
                        for row in 0..size {
                            let start = (xy[1] + row) * width + xy[0];
                            dst[start..start + size]
                                .copy_from_slice(&src[row * size..(row + 1) * size]);
                        }
                    }
                }
                address += 1;
            }
        }
        if address != end {
            return Err(invalid("incomplete AVC P field slice"));
        }
        bits.finish_rbsp()?;
    }
    if filtered {
        let mut grids: [Vec<super::avc_deblock::MacroblockEdges>; 3] =
            std::array::from_fn(|_| Vec::with_capacity(count));
        let width_mbs = w as usize / 16;
        for row in 0..height as usize / 32 {
            let current = &edges[row * width_mbs..(row + 1) * width_mbs];
            let previous = if row == 0 {
                None
            } else {
                Some(&edges[(row - 1) * width_mbs..row * width_mbs])
            };
            for (grid, mut next) in grids
                .iter_mut()
                .zip(super::avc_boundary::row_edges_field(previous, current)?)
            {
                grid.append(&mut next);
            }
        }
        for (i, (plane, grid)) in [
            &mut output.picture.y,
            &mut output.picture.cb,
            &mut output.picture.cr,
        ]
        .into_iter()
        .zip(grids.iter())
        .enumerate()
        {
            let divisor = if i == 0 { 1 } else { 2 };
            super::avc_deblock::inter_plane(
                plane,
                w as usize / divisor,
                height as usize / 2 / divisor,
                sps.bit_depth_luma,
                i != 0,
                grid,
            )?;
        }
    }
    Ok(output)
}
