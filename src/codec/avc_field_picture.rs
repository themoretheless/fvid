//! Compact CAVLC fields with intra/inter prediction and complementary weaving.
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
/// CAVLC I fields, including FMO, reconstructed in compact coordinates.
pub fn decode_intra_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<PcmField> {
    let first = *headers
        .first()
        .ok_or_else(|| invalid("missing AVC I field"))?;
    if sps.frame_mbs_only
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
        || !(8..=14).contains(&sps.bit_depth_luma)
    {
        return Err(unsupported("AVC I field configuration is not connected"));
    }
    for h in headers {
        if !h.field_pic
            || !matches!(h.slice_type, SliceType::I | SliceType::Si)
            || h.bottom_field != first.bottom_field
            || h.frame_num != first.frame_num
            || h.pps_id != pps.id
            || pps.sps_id != sps.id
            || h.idr != first.idr
            || h.idr_pic_id != first.idr_pic_id
            || h.poc_lsb != first.poc_lsb
            || h.delta_poc != first.delta_poc
            || h.memory_operations != first.memory_operations
            || h.redundant_pic_cnt != 0
            || (h.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
        {
            return Err(invalid("AVC I field slice identity mismatch"));
        }
    }
    let mut ordered = headers.to_vec();
    ordered.sort_by_key(|h| h.first_mb);
    if ordered.iter().all(|h| h.slice_type == SliceType::I && h.disable_deblocking_filter_idc == 1) {
        match decode_pcm_slices(&ordered, sps, pps, budget) {
            Ok(field) => return Ok(field),
            Err(crate::Error::Unsupported(_)) => {}
            Err(error) => return Err(error),
        }
    }
    let picture = super::avc_picture::decode_intra_slices(&ordered, sps, pps, budget)?;
    Ok(PcmField {
        picture,
        bottom: first.bottom_field,
        frame_num: first.frame_num,
        pps_id: first.pps_id,
    })
}

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
/// Compact one parity of a validated full-frame reference. Samples remain in
/// field coordinates; chroma is separated by its own plane row parity.
pub(super) fn split_frame(
    picture: &IntraPicture,
    bottom: bool,
    frame_num: u32,
    pps_id: u32,
    budget: usize,
) -> Result<PcmField> {
    let w = picture.coded_width;
    let h = picture.coded_height;
    if w == 0
        || w % 16 != 0
        || h == 0
        || h % 32 != 0
        || picture.crop[2] % 2 != 0
        || picture.crop[3] % 2 != 0
    {
        return Err(invalid("invalid AVC frame-to-field geometry"));
    }
    let bytes = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(|| invalid("AVC frame split storage overflow"))?
        / 2;
    if bytes > budget {
        return Err(invalid("AVC frame split exceeds budget"));
    }
    let split = |plane: &[u16], width: usize, height: usize| -> Result<Vec<u16>> {
        if plane.len()
            != width
                .checked_mul(height)
                .ok_or_else(|| invalid("AVC frame plane overflow"))?
        {
            return Err(invalid("invalid AVC frame plane length"));
        }
        let mut out = samples(width * (height / 2))?;
        for row in 0..height / 2 {
            let source = (2 * row + usize::from(bottom)) * width;
            out[row * width..(row + 1) * width].copy_from_slice(&plane[source..source + width]);
        }
        Ok(out)
    };
    Ok(PcmField {
        picture: IntraPicture {
            coded_width: w,
            coded_height: h / 2,
            crop: [
                picture.crop[0],
                picture.crop[1],
                picture.crop[2] / 2,
                picture.crop[3] / 2,
            ],
            bit_depth: picture.bit_depth,
            y: split(&picture.y, w, h)?,
            cb: split(&picture.cb, w / 2, h / 2)?,
            cr: split(&picture.cr, w / 2, h / 2)?,
        },
        bottom,
        frame_num,
        pps_id,
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
    if headers.iter().any(|h| h.slice_type != SliceType::P) {
        return Err(invalid("P field entrypoint requires P slices"));
    }
    let lists = references
        .iter()
        .map(|list| [*list, &[][..]])
        .collect::<Vec<_>>();
    decode_inter_field_lists(headers, sps, pps, &lists, budget)
}
/// Reference orders for implicit weighting. POC belongs to the selected field,
/// never the minimum POC of the containing complementary pair.
pub struct ImplicitFieldWeights<'a> {
    pub poc: i32,
    pub references: &'a [[&'a [(i32, bool)]; 2]],
}
/// Explicit P/B prediction without reference POC metadata.
/// Implicit weighting uses the context-aware entrypoint below.
pub fn decode_inter_field_lists(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&PcmField, u64)]; 2]],
    budget: usize,
) -> Result<PcmField> {
    decode_inter_field_lists_with_order(headers, sps, pps, references, None, budget)
}
/// P/B field reconstruction with selected-field POC/long-term metadata.
/// Direct prediction still requires retained reference motion.
pub fn decode_inter_field_lists_with_order(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&PcmField, u64)]; 2]],
    implicit: Option<&ImplicitFieldWeights<'_>>,
    budget: usize,
) -> Result<PcmField> {
    decode_inter_field_impl(headers, sps, pps, references, implicit, None, budget, false)
        .map(|(picture, _)| picture)
}
/// Return complete field motion for a caller that retains reference metadata.
/// Memory for the persistent snapshot is accounted by that caller separately.
pub fn decode_inter_field_lists_with_motion(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&PcmField, u64)]; 2]],
    implicit: Option<&ImplicitFieldWeights<'_>>,
    budget: usize,
) -> Result<(PcmField, super::avc_motion_field::MotionField)> {
    let (picture, motion) =
        decode_inter_field_impl(headers, sps, pps, references, implicit, None, budget, true)?;
    Ok((
        picture,
        motion.ok_or_else(|| invalid("missing reconstructed field motion"))?,
    ))
}
pub(super) fn decode_inter_field_impl(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&PcmField, u64)]; 2]],
    implicit: Option<&ImplicitFieldWeights<'_>>,
    direct: Option<&[super::avc_direct::FieldDirectPrediction<'_>]>,
    budget: usize,
    retain_motion: bool,
) -> Result<(PcmField, Option<super::avc_motion_field::MotionField>)> {
    if references.len() != headers.len() {
        return Err(invalid("AVC field reference contexts differ from slices"));
    }
    if direct.is_some_and(|contexts| contexts.len() != headers.len()) {
        return Err(invalid("AVC field direct contexts differ from slices"));
    }
    // An I slice has no active references, even when it arrives before an
    // inter slice. Geometry comes from an actual inter reference in the unit.
    let reference = references
        .iter()
        .find_map(|lists| lists[0].first())
        .ok_or_else(|| invalid("empty AVC field reference list"))?
        .0;
    if headers.iter().zip(references).any(|(h, lists)| {
        if matches!(h.slice_type, SliceType::I | SliceType::Si) {
            return !lists[0].is_empty() || !lists[1].is_empty();
        }
        lists[0].len() != h.refs_l0 as usize
            || lists[0].is_empty()
            || lists[0].len() > 32
            || h.slice_type == SliceType::B
                && (lists[1].len() != h.refs_l1 as usize
                    || lists[1].is_empty()
                    || lists[1].len() > 32)
            || matches!(h.slice_type, SliceType::P | SliceType::Sp) && !lists[1].is_empty()
    }) {
        return Err(invalid("invalid AVC field active reference list"));
    }
    let h = *headers
        .first()
        .ok_or_else(|| invalid("missing AVC P field"))?;
    let mut bits = BitReader::new(&h.rbsp);
    bits.skip(h.entropy_bit_offset)?;
    let (w, height) = sps.coded_dimensions();
    let count = w as usize / 16 * (height as usize / 32);
    if !pps.cabac
        && h.slice_type == SliceType::P
        && headers.len() == 1
        && h.weights.is_none()
        && reference.bottom == h.bottom_field
        && bits.unsigned_golomb()? as usize == count
    {
        if !retain_motion {
            return decode_skip_field(headers, sps, pps, reference, budget)
                .map(|picture| (picture, None));
        }
        // Every macroblock is skipped in one slice, hence all predictors are
        // zero with L0[0]. Keep the copy path and retain its real reference
        // identity through the caller's slice mapping.
        let working = count
            .checked_mul(960)
            .ok_or_else(|| invalid("AVC skip field motion storage overflow"))?;
        let pixels_budget = budget
            .checked_sub(working)
            .ok_or_else(|| invalid("AVC skip field motion exceeds budget"))?;
        let picture = decode_skip_field(headers, sps, pps, reference, pixels_budget)?;
        let mut motion = super::avc_motion_field::MotionField::new_field(
            w as usize,
            height as usize / 2,
            working,
        )?;
        for address in 0..count {
            motion.store(
                [
                    address % (w as usize / 16) * 16,
                    address / (w as usize / 16) * 16,
                ],
                [16, 16],
                0,
                [
                    super::avc_mv::Neighbour::Inter {
                        reference: 0,
                        vector: [0, 0],
                    },
                    super::avc_mv::Neighbour::NoPrediction,
                ],
            )?;
        }
        return Ok((picture, Some(motion)));
    }
    if !h.field_pic
        || !matches!(h.slice_type, SliceType::I | SliceType::Si | SliceType::P | SliceType::Sp | SliceType::B)
        || matches!(h.slice_type, SliceType::I | SliceType::Si) && pps.cabac
        || h.redundant_pic_cnt != 0
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
            // CAVLC I/SI/P/SP share this dispatcher. Intra syntax has no skip
            // run or references; switching reconstruction stays slice-local.
            || current.slice_type != h.slice_type
                && !(!pps.cabac
                    && ((matches!(current.slice_type, SliceType::I | SliceType::Si | SliceType::P | SliceType::Sp)
                        && matches!(h.slice_type, SliceType::I | SliceType::Si | SliceType::P | SliceType::Sp))
                        || (matches!(current.slice_type, SliceType::I | SliceType::B)
                            && matches!(h.slice_type, SliceType::I | SliceType::B))))
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
                || current.slice_type == SliceType::B
                    && weights.l1.len() != current.refs_l1 as usize
                || weights.luma_denom > 7
                || weights.chroma_denom > 7
            {
                return Err(invalid("invalid AVC field prediction weights"));
            }
        }
    }
    let implicit = if headers.iter().any(|h| h.slice_type == SliceType::B) && pps.weighted_bipred == 2 {
        let context = implicit.ok_or_else(|| {
            unsupported("implicit weighted B fields require reference POC context")
        })?;
        if context.references.len() != references.len()
            || context
                .references
                .iter()
                .zip(references)
                .any(|(orders, lists)| {
                    orders
                        .iter()
                        .zip(lists)
                        .any(|(order, list)| order.len() != list.len())
                })
        {
            return Err(invalid("implicit field reference orders differ from lists"));
        }
        Some(context)
    } else {
        None
    };
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
    for (candidate, _) in references
        .iter()
        .flat_map(|lists| lists.iter())
        .flat_map(|list| list.iter())
    {
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
        .try_fold(0usize, |n, lists| {
            n.checked_add((lists[0].len().saturating_sub(1) + lists[1].len()) * 32)
        })
        .ok_or_else(|| invalid("AVC field reference list storage overflow"))?;
    let required = pixels
        .checked_mul(3)
        .and_then(|n| {
            count
                .checked_mul(if pps.cabac {
                    8192
                } else if !filtered {
                    1024
                } else {
                    2048
                })
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
    let mut motion = super::avc_motion_field::MotionField::new_field(
        w as usize,
        height as usize / 2,
        count * 960,
    )?;
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
        let lists = references[*reference_index];
        let direct_context = direct.map(|contexts| &contexts[*reference_index]);
        let slice_id = slice_index as u32;
        let end = ordered
            .get(slice_index + 1)
            .map_or(count, |(_, h)| h.first_mb as usize);
        let mut qp = h.slice_qp;
        let mut intra = if pps.cabac {
            None
        } else {
            Some(super::avc_macroblock::IntraCavlcReader::new_context(
                h, sps, pps, count,
            )?)
        };
        let mut cabac = if pps.cabac {
            Some(super::avc_cabac_slice::InterCabacSlice::new(
                h,
                sps,
                pps,
                count * 4096,
            )?)
        } else {
            None
        };
        let mut ready = crate::buffer(pixels / 16)?;
        let mut bits = BitReader::new(&h.rbsp);
        bits.skip(h.entropy_bit_offset)?;
        let mut address = h.first_mb as usize;
        while address < end && (pps.cabac || bits.more_rbsp_data()) {
            let mut cabac_block = if let Some(reader) = &mut cabac {
                match reader.read_macroblock()? {
                    Some(block) => Some(block),
                    None => break,
                }
            } else {
                None
            };
            let skipped = if let Some(super::avc_inter_slice::InterMacroblock::Skip {
                address: at,
                qp: current,
            }) = &cabac_block
            {
                if *at != address {
                    return Err(invalid("CABAC field address mismatch"));
                }
                qp = *current;
                1
            } else if pps.cabac || matches!(h.slice_type, SliceType::I | SliceType::Si) {
                0
            } else {
                bits.unsigned_golomb()? as usize
            };
            if skipped > end - address {
                return Err(invalid("AVC P field skip exceeds picture"));
            }
            for step in 0..if pps.cabac { 1 } else { skipped + 1 } {
                if address == end {
                    break;
                }
                let origin = [
                    address % (w as usize / 16) * 16,
                    address / (w as usize / 16) * 16,
                ];
                let intra_block = if matches!(
                    cabac_block,
                    Some(super::avc_inter_slice::InterMacroblock::Intra(_))
                ) {
                    match cabac_block.take().unwrap() {
                        super::avc_inter_slice::InterMacroblock::Intra(block) => Some(*block),
                        _ => unreachable!(),
                    }
                } else if !pps.cabac && step == skipped {
                    let mut probe = bits.clone();
                    let code = probe.unsigned_golomb()?;
                    let intra_offset = match h.slice_type {
                        SliceType::I | SliceType::Si => 0,
                        SliceType::B => 23,
                        _ => 5,
                    };
                    if code >= intra_offset {
                        bits = probe;
                        Some(intra.as_mut().unwrap().read_embedded(
                            &mut bits,
                            address as u32,
                            qp,
                            code - intra_offset,
                        )?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(block) = intra_block {
                    if block.address as usize != address {
                        return Err(invalid("CABAC intra field address mismatch"));
                    }
                    if let Some(reader) = &intra {
                        let (luma, chroma) = reader.counts(address)?;
                        coefficients.store(address, slice_id, luma, chroma)?;
                    }
                    motion.store(
                        origin,
                        [16, 16],
                        slice_id,
                        [super::avc_mv::Neighbour::NoPrediction; 2],
                    )?;
                    super::avc_picture::reconstruct_macroblock(
                        &mut output.picture,
                        &block,
                        sps,
                        pps,
                        &scaling,
                        &mut ready,
                    )?;
                    let pcm = matches!(block.luma, super::avc_macroblock::IntraLuma::Pcm { .. });
                    if !pcm {
                        qp = block.qp;
                    }
                    if filtered {
                        let bd = 6 * (i32::from(sps.bit_depth_chroma) - 8);
                        edges.push(super::avc_boundary::DecodedBlockEdges {
                            blocks: [super::avc_boundary::BlockEdge {
                                intra: true,
                                switching_slice: matches!(h.slice_type, SliceType::Sp | SliceType::Si),
                                nonzero_luma: false,
                                motion: [None; 2],
                            }; 16],
                            qp: if pcm {
                                [0; 3]
                            } else {
                                [
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
                                ]
                            },
                            offsets: [h.alpha_offset, h.beta_offset],
                            transform8: matches!(
                                block.luma,
                                super::avc_macroblock::IntraLuma::Blocks8 { .. }
                            ),
                            slice_id,
                            disable_filter: h.disable_deblocking_filter_idc as u8,
                        });
                    }
                    address += 1;
                    continue;
                }
                let mut residual = None;
                let predictions = if step < skipped {
                    coefficients.store(address, slice_id, [0; 16], [[0; 4]; 2])?;
                    if let Some(reader) = &mut intra {
                        reader.record_inter(address, [0; 16], [[0; 4]; 2])?;
                    }
                    if h.slice_type == SliceType::B {
                        if direct_context.is_none() {
                            return Err(unsupported(
                                "B field direct prediction requires reference motion context",
                            ));
                        }
                        let super::avc_inter::MacroblockType::Inter {
                            partitions: parts, ..
                        } = super::avc_inter::macroblock_type(SliceType::B, 0)?
                        else {
                            return Err(invalid("invalid B skip partition layout"));
                        };
                        let vectors = motion.decode_field_macroblock_with_direct(
                            origin,
                            slice_id,
                            &parts,
                            direct_context,
                        )?;
                        parts
                            .iter()
                            .zip(vectors)
                            .map(|(p, v)| (p.origin.map(usize::from), p.size.map(usize::from), v))
                            .collect::<Vec<_>>()
                    } else {
                        let vector = motion.decode_p_skip(origin, slice_id)?;
                        vec![(
                            [0, 0],
                            [16, 16],
                            [
                                super::avc_mv::Neighbour::Inter {
                                    vector,
                                    reference: 0,
                                },
                                super::avc_mv::Neighbour::NoPrediction,
                            ],
                        )]
                    }
                } else {
                    let (header, c) = if pps.cabac {
                        match cabac_block
                            .take()
                            .ok_or_else(|| invalid("missing CABAC field block"))?
                        {
                            super::avc_inter_slice::InterMacroblock::Coded {
                                address: at,
                                header,
                                coefficients,
                            } => {
                                if at != address {
                                    return Err(invalid("CABAC coded field address mismatch"));
                                }
                                (header, *coefficients)
                            }
                            _ => return Err(invalid("unexpected CABAC field block")),
                        }
                    } else {
                        let syntax = super::avc_inter::InterSyntax {
                            slice: h.slice_type,
                            active_references: [
                                h.refs_l0,
                                if h.slice_type == SliceType::B {
                                    h.refs_l1
                                } else {
                                    0
                                },
                            ],
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
                        (header, c)
                    };
                    coefficients.store(address, slice_id, c.luma_counts, c.chroma_counts)?;
                    if let Some(reader) = &mut intra {
                        reader.record_inter(address, c.luma_counts, c.chroma_counts)?;
                    }
                    qp = header.residual.qp;
                    if header.residual.pattern != 0 {
                        residual = Some((c, header.residual.transform8));
                    }
                    let parts = header.partitions;
                    if parts
                        .iter()
                        .any(|p| p.prediction == super::avc_inter::Prediction::Direct)
                        && direct_context.is_none()
                    {
                        return Err(unsupported(
                            "B field direct prediction requires reference motion context",
                        ));
                    }
                    let neighbour = motion.decode_field_macroblock_with_direct(
                        origin,
                        slice_id,
                        &parts,
                        direct_context,
                    )?;
                    parts
                        .iter()
                        .zip(neighbour)
                        .map(|(part, vectors)| {
                            (
                                part.origin.map(usize::from),
                                part.size.map(usize::from),
                                vectors,
                            )
                        })
                        .collect::<Vec<_>>()
                };
                let mut blocks = [super::avc_boundary::BlockEdge {
                    intra: false,
                    switching_slice: matches!(h.slice_type, SliceType::Sp | SliceType::Si),
                    nonzero_luma: false,
                    motion: [None; 2],
                }; 16];
                for (offset, dimensions, vectors) in predictions {
                    let mut selected = [None; 2];
                    for list in 0..2 {
                        match vectors[list] {
                            super::avc_mv::Neighbour::Inter { reference, vector } => {
                                let (picture, id) =
                                    *lists[list].get(reference as usize).ok_or_else(|| {
                                        invalid("AVC field partition reference exceeds list")
                                    })?;
                                selected[list] = Some((picture, reference, vector));
                                if filtered {
                                    for by in offset[1] / 4..(offset[1] + dimensions[1]) / 4 {
                                        for bx in offset[0] / 4..(offset[0] + dimensions[0]) / 4 {
                                            blocks[by * 4 + bx].motion[list] =
                                                Some(super::avc_boundary::MotionReference {
                                                    picture: id,
                                                    vector,
                                                });
                                        }
                                    }
                                }
                            }
                            super::avc_mv::Neighbour::NoPrediction => {}
                            _ => return Err(invalid("missing AVC field motion")),
                        }
                    }
                    if selected.iter().all(Option::is_none) {
                        return Err(invalid("empty AVC field prediction"));
                    }
                    let implicit_weights = if selected.iter().all(Option::is_some) {
                        implicit.map(|context| {
                            let references = context.references[*reference_index];
                            let a = references[0][selected[0].unwrap().1 as usize];
                            let b = references[1][selected[1].unwrap().1 as usize];
                            super::avc_mv::implicit_weights(
                                i64::from(context.poc),
                                i64::from(a.0),
                                i64::from(b.0),
                                a.1 || b.1,
                            )
                        })
                    } else {
                        None
                    };
                    for component in 0..3 {
                        let divisor = if component == 0 { 1 } else { 2 };
                        let [bw, bh] = dimensions.map(|v| v / divisor);
                        let xy = [
                            (origin[0] + offset[0]) / divisor,
                            (origin[1] + offset[1]) / divisor,
                        ];
                        let pw = w as usize / divisor;
                        let ph = height as usize / 2 / divisor;
                        let mut prediction = [[0u16; 256]; 2];
                        let mut weights = [(1i16, 0i16); 2];
                        let denominator = h.weights.as_ref().map_or(0, |weights| {
                            if component == 0 {
                                weights.luma_denom
                            } else {
                                weights.chroma_denom
                            }
                        });
                        for list in 0..2 {
                            if let Some((picture, index, vector)) = selected[list] {
                                let plane = match component {
                                    0 => &picture.picture.y,
                                    1 => &picture.picture.cb,
                                    _ => &picture.picture.cr,
                                };
                                let reference = super::avc_motion::ReferencePlane::from_decoded(
                                    plane,
                                    pw,
                                    ph,
                                    pw,
                                    sps.bit_depth_luma,
                                )?;
                                let mut mv = vector.map(i32::from);
                                if component != 0 {
                                    mv[1] += match (picture.bottom, h.bottom_field) {
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
                                        &mut prediction[list][..bw * bh],
                                    )?;
                                } else {
                                    reference.chroma(
                                        xy.map(|v| v as i32),
                                        mv,
                                        bw,
                                        bh,
                                        &mut prediction[list][..bw * bh],
                                    )?;
                                }
                                if let Some(table) = &h.weights {
                                    let entries = if list == 0 { &table.l0 } else { &table.l1 };
                                    let entry = entries.get(index as usize).ok_or_else(|| {
                                        invalid("missing AVC field partition weight")
                                    })?;
                                    weights[list] = if component == 0 {
                                        entry.luma
                                    } else {
                                        entry.chroma[component - 1]
                                    };
                                }
                            }
                        }
                        let denominator = if let Some(implicit) = implicit_weights {
                            weights = implicit.map(|weight| (weight, 0));
                            5
                        } else {
                            denominator
                        };
                        let used = if selected[0].is_some() { 0 } else { 1 };
                        if selected.iter().all(Option::is_some) {
                            let (a, b) = prediction.split_at_mut(1);
                            super::avc_motion::bipred_block(
                                &mut a[0][..bw * bh],
                                &b[0][..bw * bh],
                                weights.map(|w| w.0),
                                weights.map(|w| w.1),
                                denominator,
                                sps.bit_depth_luma,
                            )?;
                        } else if h.weights.is_some() {
                            super::avc_motion::weight_block(
                                &mut prediction[used][..bw * bh],
                                weights[used].0,
                                weights[used].1,
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
                                .copy_from_slice(&prediction[used][row * bw..(row + 1) * bw]);
                        }
                    }
                }
                let eight = residual.as_ref().is_some_and(|(_, eight)| *eight);
                if filtered {
                    if let Some((c, _)) = &residual {
                        for i in 0..16 {
                            blocks[i].nonzero_luma = if eight {
                                c.luma8[(i / 4) / 2 * 2 + (i % 4) / 2]
                                    .iter()
                                    .any(|&v| v != 0)
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
                // SP skips and coded zero-CBP blocks still quantize prediction.
                if h.slice_type == SliceType::Sp && residual.is_none() {
                    residual = Some((super::avc_inter_coefficients::InterCoefficients {
                        luma4: [[0;16];16], luma8: [[0;64];4],
                        chroma_dc: [[0;4];2], chroma_ac: [[[0;16];4];2],
                        luma_counts: [0;16], chroma_counts: [[0;4];2],
                    }, false));
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
                    let reconstructed = if h.slice_type == SliceType::Sp {
                        let qs=h.slice_qs.ok_or_else(||invalid("missing SP field QS"))?;
                        prediction.reconstruct_sp(&c.luma4, &c.chroma_dc, &c.chroma_ac, qps,
                            [qs as u8,super::avc_picture::chroma_qp(qs,pps.chroma_qp_offset,8),super::avc_picture::chroma_qp(qs,pps.second_chroma_qp_offset,8)],h.sp_for_switch)?
                    } else if sps.transform_bypass && qps[0] == 0 {
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
                if !pps.constrained_intra_pred {
                    for by in 0..4 {
                        for bx in 0..4 {
                            ready[(origin[1] / 4 + by) * (w as usize / 4) + origin[0] / 4 + bx] = 1;
                        }
                    }
                }
                address += 1;
            }
        }
        if address != end {
            return Err(invalid("incomplete AVC P field slice"));
        }
        if !pps.cabac {
            bits.finish_rbsp()?;
        } else if cabac.as_mut().unwrap().read_macroblock()?.is_some() {
            return Err(invalid("CABAC field slice exceeds coverage"));
        }
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
    Ok((output, if retain_motion { Some(motion) } else { None }))
}
