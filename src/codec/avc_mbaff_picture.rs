//! MBAFF intra pictures and address-local inter reconstruction/publication.
use super::{
    avc::{Pps, Sps},
    avc_cabac_macroblock::IntraCabacReader,
    avc_deblock::{MbaffIntraBlock, mbaff_intra_plane},
    avc_macroblock::{IntraCavlcReader, IntraLuma},
    avc_mbaff::{Readiness420, layout, write_samples},
    avc_picture::{IntraPicture, reconstruct_macroblock},
    avc_scaling::ScalingMatrices,
    avc_slice::SliceHeader,
};
use crate::{Result, invalid};
fn plane(size: usize) -> Result<Vec<u16>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(size)
        .map_err(|_| invalid("cannot allocate MBAFF reconstruction plane"))?;
    values.resize(size, 0);
    Ok(values)
}
/// Reconstruct and publish an inter macroblock in frame or alternating field
/// rows. Prediction, scaling and component QPs are already resolved by caller.
/// Validation and residual reconstruction finish before any picture plane changes.
pub fn reconstruct_inter_macroblock(
    picture: &mut IntraPicture,
    address: usize,
    field: bool,
    prediction: super::avc_compensation::Prediction420,
    coefficients: Option<&super::avc_inter_coefficients::InterCoefficients>,
    transform8: bool,
    qps: [u8; 3],
    bypass: bool,
    scaling: &ScalingMatrices,
) -> Result<()> {
    let (w, h) = (picture.coded_width, picture.coded_height);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF picture size overflow"))?;
    if !(8..=14).contains(&picture.bit_depth)
        || w % 16 != 0
        || h % 32 != 0
        || prediction.dimensions() != (16, 16)
        || prediction.bit_depth() != picture.bit_depth
        || picture.y.len() != pixels
        || picture.cb.len() != pixels / 4
        || picture.cr.len() != pixels / 4
    {
        return Err(invalid("invalid MBAFF inter reconstruction picture"));
    }
    let maximum = (1u16 << picture.bit_depth) - 1;
    if prediction
        .y
        .iter()
        .chain(&prediction.cb)
        .chain(&prediction.cr)
        .any(|&v| v > maximum)
    {
        return Err(invalid("MBAFF inter prediction exceeds bit depth"));
    }
    let targets = [
        layout(address, w / 16, h / 16, true, field, [1, 1])?,
        layout(address, w / 16, h / 16, true, field, [2, 2])?,
    ];
    let reconstructed = if let Some(c) = coefficients {
        let luma = if transform8 {
            super::avc_compensation::InterLumaResidual::Blocks8(&c.luma8)
        } else {
            super::avc_compensation::InterLumaResidual::Blocks4(&c.luma4)
        };
        if bypass {
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
        }
    } else {
        prediction
    };
    write_samples(&mut picture.y, w, targets[0], &reconstructed.y)?;
    write_samples(&mut picture.cb, w / 2, targets[1], &reconstructed.cb)?;
    write_samples(&mut picture.cr, w / 2, targets[1], &reconstructed.cr)?;
    Ok(())
}
/// Join inter sample publication with complete slice-local readiness. No ready
/// component or sample is changed if validation/residual reconstruction fails.
pub fn reconstruct_inter_macroblock_ready(
    picture: &mut IntraPicture,
    address: usize,
    field: bool,
    prediction: super::avc_compensation::Prediction420,
    coefficients: Option<&super::avc_inter_coefficients::InterCoefficients>,
    transform8: bool,
    qps: [u8; 3],
    bypass: bool,
    scaling: &ScalingMatrices,
    readiness: &mut Readiness420,
) -> Result<()> {
    let geometry = [picture.coded_width / 16, picture.coded_height / 16];
    readiness.check_mbaff_complete(address, geometry, field)?;
    reconstruct_inter_macroblock(
        picture,
        address,
        field,
        prediction,
        coefficients,
        transform8,
        qps,
        bypass,
        scaling,
    )?;
    readiness.publish_mbaff_complete(address, geometry, field)
}
enum MbaffSliceReader<'a> {
    Cavlc(super::avc_inter_slice::InterCavlcSlice<'a>),
    Cabac(super::avc_cabac_slice::InterCabacSlice<'a>),
    IntraCavlc(IntraCavlcReader<'a>),
    IntraCabac(IntraCabacReader<'a>),
}
impl<'a> MbaffSliceReader<'a> {
    fn new(header: &'a SliceHeader, sps: &'a Sps, pps: &'a Pps, count: usize) -> Result<Self> {
        Ok(if header.slice_type == super::avc_slice::SliceType::I {
            if pps.cabac {
                Self::IntraCabac(IntraCabacReader::new_mbaff(header, sps, pps, count)?)
            } else {
                Self::IntraCavlc(IntraCavlcReader::new_mbaff(header, sps, pps, count)?)
            }
        } else if pps.cabac {
            Self::Cabac(super::avc_cabac_slice::InterCabacSlice::new_mbaff(
                header,
                sps,
                pps,
                count * 1024,
            )?)
        } else {
            Self::Cavlc(super::avc_inter_slice::InterCavlcSlice::new_mbaff(
                header,
                sps,
                pps,
                count * 80,
            )?)
        })
    }
    fn read_macroblock(&mut self) -> Result<Option<super::avc_inter_slice::InterMacroblock>> {
        match self {
            Self::Cavlc(r) => r.read_macroblock(),
            Self::Cabac(r) => r.read_macroblock(),
            Self::IntraCavlc(r) => r.read_macroblock().map(|block| {
                block.map(|mb| super::avc_inter_slice::InterMacroblock::Intra(Box::new(mb)))
            }),
            Self::IntraCabac(r) => r.read_macroblock().map(|block| {
                block.map(|mb| super::avc_inter_slice::InterMacroblock::Intra(Box::new(mb)))
            }),
        }
    }
    fn field_decoding(&self) -> bool {
        match self {
            Self::Cavlc(r) => r.field_decoding(),
            Self::Cabac(r) => r.field_decoding(),
            Self::IntraCavlc(r) => r.field_decoding(),
            Self::IntraCabac(r) => r.field_decoding(),
        }
    }
    fn pair_field(&self, pair: usize) -> Option<bool> {
        match self {
            Self::Cavlc(r) => r.pair_field(pair),
            Self::Cabac(r) => r.pair_field(pair),
            Self::IntraCavlc(r) => r.pair_field(pair),
            Self::IntraCabac(r) => r.pair_field(pair),
        }
    }
}
/// Assemble ordered MBAFF P slices before the deblocking stage.
/// References are independently resolved frame lists for each slice. This is
/// an intermediate reconstruction API, not a complete playback admission path.
pub fn decode_p_slices_unfiltered(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[&IntraPicture]; 2]],
    budget: usize,
) -> Result<(IntraPicture, super::avc_motion_field::MotionField)> {
    decode_p_slices_impl(headers, sps, pps, references, budget, false, None)
}
/// Reconstruct and deblock ordered MBAFF P slices with resolved frame lists.
pub fn decode_p_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[&IntraPicture]; 2]],
    budget: usize,
) -> Result<(IntraPicture, super::avc_motion_field::MotionField)> {
    decode_p_slices_impl(headers, sps, pps, references, budget, true, None)
}
/// Assemble ordered I/P/B MBAFF slices with per-slice direct/POC contexts.
pub fn decode_inter_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[&IntraPicture]; 2]],
    direct: &[Option<&super::avc_direct::MbaffDirectPrediction<'_>>],
    budget: usize,
) -> Result<(IntraPicture, super::avc_motion_field::MotionField)> {
    decode_p_slices_impl(headers, sps, pps, references, budget, true, Some(direct))
}
fn decode_p_slices_impl(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[&IntraPicture]; 2]],
    budget: usize,
    filtered: bool,
    direct_by_slice: Option<&[Option<&super::avc_direct::MbaffDirectPrediction<'_>>]>,
) -> Result<(IntraPicture, super::avc_motion_field::MotionField)> {
    use super::{
        avc_boundary::{BlockEdge, DecodedBlockEdges, MotionReference},
        avc_compensation::Reference420,
        avc_deblock::{MbaffBlockEdges, mbaff_inter_plane},
        avc_inter::{Partition, Prediction},
        avc_inter_prediction::predict_macroblock_mbaff,
        avc_inter_slice::InterMacroblock,
        avc_motion_field::MotionField,
        avc_mv::Neighbour,
        avc_slice::SliceType,
    };
    if headers.is_empty()
        || headers.len() != references.len()
        || direct_by_slice.is_some_and(|v| v.len() != headers.len())
        || headers[0].first_mb != 0
        || sps.frame_mbs_only
        || !sps.mb_adaptive_frame_field
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
        || headers.iter().any(|h| {
            !matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B)
                || h.field_pic
                || h.redundant_pic_cnt != 0
                || h.disable_deblocking_filter_idc > 2
        })
    {
        return Err(invalid("invalid MBAFF reconstruction configuration"));
    }
    let first = headers[0];
    if headers.iter().any(|h| {
        h.pps_id != first.pps_id
            || h.frame_num != first.frame_num
            || (h.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
            || h.idr != first.idr
            || h.idr_pic_id != first.idr_pic_id
            || h.poc_lsb != first.poc_lsb
            || h.delta_poc_bottom != first.delta_poc_bottom
            || h.delta_poc != first.delta_poc
            || h.colour_plane_id != first.colour_plane_id
    }) {
        return Err(invalid("MBAFF slices belong to different pictures"));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF picture size overflow"))?;
    let count = pixels / 256;
    let reserve = pixels
        .checked_mul(6)
        .and_then(|n| n.checked_add(pixels / 16))
        .and_then(|n| {
            count
                .checked_mul(
                    (if pps.cabac { 1024 } else { 80 }) + std::mem::size_of::<MbaffBlockEdges>(),
                )
                .and_then(|c| n.checked_add(c))
        })
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| invalid("MBAFF P budget overflow"))?;
    let mut motion = MotionField::new(
        w,
        h,
        budget
            .checked_sub(reserve)
            .ok_or_else(|| invalid("MBAFF P reconstruction exceeds memory budget"))?,
    )?;
    let scaling = ScalingMatrices::new(sps, pps)?;
    let mut ready = Readiness420::new(w / 16, h / 16, true, count * 7)?;
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|n| n as usize),
        bit_depth: sps.bit_depth_luma,
        y: plane(pixels)?,
        cb: plane(pixels / 4)?,
        cr: plane(pixels / 4)?,
    };
    let mut deblocking = Vec::new();
    deblocking
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate MBAFF P deblocking metadata"))?;
    let edge_state = |blocks, qp, field, transform8, slice, header: &SliceHeader| MbaffBlockEdges {
        field,
        edges: DecodedBlockEdges {
            blocks,
            qp,
            transform8,
            slice_id: slice,
            disable_filter: header.disable_deblocking_filter_idc as u8,
            offsets: [header.alpha_offset, header.beta_offset],
        },
    };
    let mut seen = 0;
    for (slice, (header, lists)) in headers.iter().zip(references).enumerate() {
        let direct = direct_by_slice.and_then(|v| v[slice]);
        let is_b = header.slice_type == SliceType::B;
        if is_b && direct.is_none() {
            return Err(invalid("MBAFF B slice lacks direct context"));
        }
        if header.first_mb as usize * 2 != seen {
            return Err(invalid("MBAFF P slice coverage gap or overlap"));
        }
        let end = headers
            .get(slice + 1)
            .map_or(count, |h| h.first_mb as usize * 2);
        if end <= seen || end > count {
            return Err(invalid("invalid MBAFF P slice range"));
        }
        let mut views: [Vec<Reference420<'_>>; 2] = [Vec::new(), Vec::new()];
        for list in 0..2 {
            if lists[list].len() > 32 {
                return Err(invalid("MBAFF P reference list exceeds 32 frames"));
            }
            views[list]
                .try_reserve_exact(lists[list].len())
                .map_err(|_| invalid("cannot allocate MBAFF references"))?;
            for reference in lists[list] {
                if reference.coded_width != w
                    || reference.coded_height != h
                    || reference.bit_depth != picture.bit_depth
                {
                    return Err(invalid("MBAFF P reference geometry/depth mismatch"));
                }
                views[list].push(Reference420::new(
                    [&reference.y, &reference.cb, &reference.cr],
                    w,
                    h,
                    [w, w / 2, w / 2],
                    picture.bit_depth,
                )?);
            }
        }
        let refs: [Vec<&Reference420<'_>>; 2] =
            [views[0].iter().collect(), views[1].iter().collect()];
        ready.reset_slice();
        let mut reader = MbaffSliceReader::new(header, sps, pps, count)?;
        while let Some(block) = reader.read_macroblock()? {
            let field = reader.field_decoding();
            if let InterMacroblock::Intra(mb) = block {
                let address = mb.address as usize;
                if address != seen || seen >= end {
                    return Err(invalid("MBAFF P intra coverage mismatch"));
                }
                motion.store_mbaff(
                    address,
                    [0, 0],
                    [16, 16],
                    slice as u32,
                    field,
                    [Neighbour::NoPrediction; 2],
                )?;
                let pcm = matches!(mb.luma, IntraLuma::Pcm { .. });
                let bd = 6 * (i32::from(sps.bit_depth_luma) - 8);
                let qps = if pcm {
                    [0; 3]
                } else {
                    [
                        mb.qp,
                        i32::from(super::avc_picture::chroma_qp(
                            mb.qp,
                            pps.chroma_qp_offset,
                            sps.bit_depth_chroma,
                        )) - bd,
                        i32::from(super::avc_picture::chroma_qp(
                            mb.qp,
                            pps.second_chroma_qp_offset,
                            sps.bit_depth_chroma,
                        )) - bd,
                    ]
                };
                deblocking.push(edge_state(
                    [BlockEdge {
                        intra: true,
                        switching_slice: false,
                        nonzero_luma: false,
                        motion: [None; 2],
                    }; 16],
                    qps,
                    field,
                    matches!(mb.luma, IntraLuma::Blocks8 { .. }),
                    slice as u32,
                    header,
                ));
                reconstruct_intra_macroblock(
                    &mut picture,
                    *mb,
                    field,
                    sps,
                    pps,
                    &scaling,
                    &mut ready,
                )?;
                seen += 1;
                continue;
            }
            let (address, qp, parts, coefficients, eight) = match block {
                InterMacroblock::Skip { address, qp } => (
                    address,
                    qp,
                    if is_b {
                        let super::avc_inter::MacroblockType::Inter { partitions, .. } =
                            super::avc_inter::macroblock_type(SliceType::B, 0)?
                        else {
                            return Err(invalid("invalid B skip partitions"));
                        };
                        partitions
                    } else {
                        vec![Partition {
                            origin: [0, 0],
                            size: [16, 16],
                            prediction: Prediction::L0,
                            group: 0,
                            references: [Some(0), None],
                            differences: [[0; 2]; 2],
                        }]
                    },
                    None,
                    false,
                ),
                InterMacroblock::Coded {
                    address,
                    header,
                    coefficients,
                } => (
                    address,
                    header.residual.qp,
                    header.partitions,
                    Some(coefficients),
                    header.residual.transform8,
                ),
                InterMacroblock::Intra(_) => unreachable!(),
            };
            if address != seen || seen >= end {
                return Err(invalid("MBAFF P inter coverage mismatch"));
            }
            let vectors = if coefficients.is_none() && !is_b {
                vec![[
                    Neighbour::Inter {
                        reference: 0,
                        vector: motion
                            .decode_p_skip_mbaff(address, slice as u32, |p| reader.pair_field(p))?,
                    },
                    Neighbour::NoPrediction,
                ]]
            } else {
                motion.decode_macroblock_mbaff_with_direct(
                    address,
                    slice as u32,
                    &parts,
                    |p| reader.pair_field(p),
                    direct,
                )?
            };
            let weights = super::avc_inter_prediction::mbaff_weights(
                address, field, header, pps, &vectors, direct,
            )?;
            let prediction = predict_macroblock_mbaff(
                address,
                [w / 16, h / 16],
                field,
                picture.bit_depth,
                &parts,
                &vectors,
                [&refs[0], &refs[1]],
                weights.as_deref(),
            )?;
            let qps = [
                (super::avc_residual_syntax::update_qp(qp, 0, sps.bit_depth_luma)?
                    + 6 * (i32::from(sps.bit_depth_luma) - 8)) as u8,
                super::avc_picture::chroma_qp(qp, pps.chroma_qp_offset, sps.bit_depth_chroma),
                super::avc_picture::chroma_qp(
                    qp,
                    pps.second_chroma_qp_offset,
                    sps.bit_depth_chroma,
                ),
            ];
            let mut blocks = [BlockEdge {
                intra: false,
                switching_slice: false,
                nonzero_luma: false,
                motion: [None; 2],
            }; 16];
            for (part, vectors) in parts.iter().zip(&vectors) {
                let mut identities = [None; 2];
                for list in 0..2 {
                    if let Neighbour::Inter { reference, vector } = vectors[list] {
                        let frame = lists[list]
                            .get(usize::from(reference) / if field { 2 } else { 1 })
                            .ok_or_else(|| invalid("missing MBAFF deblocking reference"))?;
                        let identity = references
                            .iter()
                            .flat_map(|l| l.iter().flat_map(|r| r.iter()))
                            .position(|r| std::ptr::eq(*r, *frame))
                            .ok_or_else(|| invalid("unknown MBAFF reference identity"))?;
                        identities[list] = Some(MotionReference {
                            picture: identity as u64 * 3
                                + if field {
                                    1 + ((address % 2) ^ (usize::from(reference) % 2)) as u64
                                } else {
                                    0
                                },
                            vector,
                        });
                    }
                }
                for y in
                    usize::from(part.origin[1]) / 4..usize::from(part.origin[1] + part.size[1]) / 4
                {
                    for x in usize::from(part.origin[0]) / 4
                        ..usize::from(part.origin[0] + part.size[0]) / 4
                    {
                        blocks[y * 4 + x].motion = identities;
                    }
                }
            }
            if let Some(c) = &coefficients {
                for (i, block) in blocks.iter_mut().enumerate() {
                    block.nonzero_luma = if eight {
                        c.luma8[i / 4 / 2 * 2 + i % 4 / 2].iter().any(|&v| v != 0)
                    } else {
                        c.luma_counts[i] != 0
                    };
                }
            }
            let bd = 6 * (i32::from(sps.bit_depth_luma) - 8);
            deblocking.push(edge_state(
                blocks,
                qps.map(|q| i32::from(q) - bd),
                field,
                eight,
                slice as u32,
                header,
            ));
            reconstruct_inter_macroblock_ready(
                &mut picture,
                address,
                field,
                prediction,
                coefficients.as_deref(),
                eight,
                qps,
                sps.transform_bypass && qps[0] == 0,
                &scaling,
                &mut ready,
            )?;
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete MBAFF P slice"));
        }
    }
    if filtered {
        mbaff_inter_plane(&mut picture.y, w, h, picture.bit_depth, 0, &deblocking)?;
        mbaff_inter_plane(
            &mut picture.cb,
            w / 2,
            h / 2,
            picture.bit_depth,
            1,
            &deblocking,
        )?;
        mbaff_inter_plane(
            &mut picture.cr,
            w / 2,
            h / 2,
            picture.bit_depth,
            2,
            &deblocking,
        )?;
    }
    Ok((picture, motion))
}

/// Shared address-local intra reconstruction for intra and mixed MBAFF slices.
/// The picture assembler owns the matching slice-local readiness geometry.
pub(crate) fn reconstruct_intra_macroblock(
    picture: &mut IntraPicture,
    mut mb: super::avc_macroblock::IntraMacroblock,
    field: bool,
    sps: &Sps,
    pps: &Pps,
    scaling: &ScalingMatrices,
    readiness: &mut Readiness420,
) -> Result<()> {
    let (w, h) = (picture.coded_width, picture.coded_height);
    let address = mb.address as usize;
    readiness.check_mbaff_complete(address, [w / 16, h / 16], field)?;
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF picture size overflow"))?;
    if sps.coded_dimensions() != (w as u32, h as u32)
        || picture.bit_depth != sps.bit_depth_luma
        || picture.y.len() != pixels
        || picture.cb.len() != pixels / 4
        || picture.cr.len() != pixels / 4
    {
        return Err(invalid("invalid MBAFF intra reconstruction picture"));
    }
    layout(address, w / 16, h / 16, true, field, [1, 1])?;
    let parity = if field { address % 2 } else { 0 };
    let step = if field { 2 } else { 1 };
    let view_h = h / step;
    let mut view = IntraPicture {
        coded_width: w,
        coded_height: view_h,
        crop: [0; 4],
        bit_depth: picture.bit_depth,
        y: plane(w * view_h)?,
        cb: plane(w * view_h / 4)?,
        cr: plane(w * view_h / 4)?,
    };
    for (source, dest, stride) in [
        (&picture.y, &mut view.y, w),
        (&picture.cb, &mut view.cb, w / 2),
        (&picture.cr, &mut view.cr, w / 2),
    ] {
        for (row, line) in dest.chunks_exact_mut(stride).enumerate() {
            let start = (row * step + parity) * stride;
            line.copy_from_slice(&source[start..start + stride]);
        }
    }
    let mut ready = crate::buffer(w * view_h / 16)?;
    for by in 0..view_h / 4 {
        for bx in 0..w / 4 {
            let mut available = true;
            for dy in 0..4 {
                for dx in 0..4 {
                    available &=
                        readiness.available(0, [bx * 4 + dx, (by * 4 + dy) * step + parity])?;
                }
            }
            ready[by * (w / 4) + bx] = u8::from(available);
        }
    }
    let geometry = layout(address, w / 16, h / 16, true, field, [1, 1])?;
    let logical_y = (geometry.origin[1] - parity) / step;
    mb.address = (logical_y / 16 * (w / 16) + geometry.origin[0] / 16) as u32;
    reconstruct_macroblock(&mut view, &mb, sps, pps, scaling, &mut ready)?;
    for (component, source, dest, stride, side) in [
        (0, &view.y, &mut picture.y, w, 16),
        (1, &view.cb, &mut picture.cb, w / 2, 8),
        (2, &view.cr, &mut picture.cr, w / 2, 8),
    ] {
        let sub = if component == 0 { [1, 1] } else { [2, 2] };
        let target = layout(address, w / 16, h / 16, true, field, sub)?;
        let x = target.origin[0];
        let y = (target.origin[1] - parity) / step;
        let mut samples = [0u16; 256];
        for row in 0..side {
            samples[row * side..row * side + side]
                .copy_from_slice(&source[(y + row) * stride + x..(y + row) * stride + x + side]);
        }
        write_samples(dest, stride, target, &samples[..side * side])?;
    }
    readiness.publish_mbaff_complete(address, [w / 16, h / 16], field)
}

/// Decode one complete intra MBAFF CAVLC or CABAC slice.
/// Budget includes frame planes, a temporary prediction view and entropy/readiness
/// contexts. Input RBSP and caller-held output pictures are outside this budget.
pub fn decode_intra_picture(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    decode_intra_slices(&[header], sps, pps, budget)
}
/// Reconstruct ordered intra slices with independent entropy and prediction state.
pub fn decode_intra_slices(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    let header = *headers
        .first()
        .ok_or_else(|| invalid("missing MBAFF intra slices"))?;
    if header.first_mb != 0 {
        return Err(invalid(
            "MBAFF intra picture must start at macroblock pair zero",
        ));
    }
    if sps.bit_depth_luma != sps.bit_depth_chroma {
        return Err(crate::unsupported(
            "MBAFF mixed component depths are not connected",
        ));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("MBAFF pixel count overflow"))?;
    let count = pixels / 256;
    let storage = pixels
        .checked_mul(6)
        .and_then(|n| {
            count
                .checked_mul(80 + std::mem::size_of::<MbaffIntraBlock>())
                .and_then(|c| n.checked_add(c))
        })
        .ok_or_else(|| invalid("MBAFF reconstruction budget overflow"))?;
    if storage > budget {
        return Err(invalid("MBAFF reconstruction exceeds memory budget"));
    }

    let scaling = ScalingMatrices::new(sps, pps)?;
    let mut readiness = Readiness420::new(w / 16, h / 16, true, count * 7)?;
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|n| n as usize),
        bit_depth: sps.bit_depth_luma,
        y: plane(pixels)?,
        cb: plane(pixels / 4)?,
        cr: plane(pixels / 4)?,
    };
    let mut seen = 0;
    let mut deblocking = Vec::new();
    deblocking
        .try_reserve_exact(count)
        .map_err(|_| invalid("cannot allocate MBAFF deblocking metadata"))?;
    for (index, header) in headers.iter().enumerate() {
        if header.first_mb as usize * 2 != seen {
            return Err(invalid("MBAFF slice coverage gap or overlap"));
        }
        let end = headers
            .get(index + 1)
            .map_or(count, |h| h.first_mb as usize * 2);
        readiness.reset_slice();
        let mut cavlc = if pps.cabac {
            None
        } else {
            Some(IntraCavlcReader::new_mbaff(header, sps, pps, count)?)
        };
        let mut cabac = if pps.cabac {
            Some(IntraCabacReader::new_mbaff(header, sps, pps, count)?)
        } else {
            None
        };
        loop {
            let mb = match (&mut cavlc, &mut cabac) {
                (Some(reader), _) => reader.read_macroblock()?,
                (_, Some(reader)) => reader.read_macroblock()?,
                _ => unreachable!(),
            };
            let Some(mb) = mb else { break };
            let address = mb.address as usize;
            if address != seen || seen >= end {
                return Err(invalid("MBAFF intra slice coverage gap"));
            }
            let field = match (&cavlc, &cabac) {
                (Some(reader), _) => reader.field_decoding(),
                (_, Some(reader)) => reader.field_decoding(),
                _ => unreachable!(),
            };
            let pcm = matches!(mb.luma, IntraLuma::Pcm { .. });
            let qp = if pcm { 0 } else { mb.qp };
            let chroma = |offset| {
                i32::from(super::avc_picture::chroma_qp(
                    qp,
                    offset,
                    sps.bit_depth_chroma,
                )) - 6 * (i32::from(sps.bit_depth_chroma) - 8)
            };
            deblocking.push(MbaffIntraBlock {
                qp: if pcm {
                    [0; 3]
                } else {
                    [
                        qp,
                        chroma(pps.chroma_qp_offset),
                        chroma(pps.second_chroma_qp_offset),
                    ]
                },
                field,
                transform8: matches!(mb.luma, IntraLuma::Blocks8 { .. }),
                slice: index,
                disable: u8::try_from(header.disable_deblocking_filter_idc)
                    .map_err(|_| invalid("invalid MBAFF deblocking disable value"))?,
                offsets: [header.alpha_offset, header.beta_offset],
            });
            reconstruct_intra_macroblock(
                &mut picture,
                mb,
                field,
                sps,
                pps,
                &scaling,
                &mut readiness,
            )?;
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete MBAFF intra slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete MBAFF intra picture"));
    }
    for (component, samples, width, height) in [
        (0, &mut picture.y, w, h),
        (1, &mut picture.cb, w / 2, h / 2),
        (2, &mut picture.cr, w / 2, h / 2),
    ] {
        mbaff_intra_plane(
            samples,
            width,
            height,
            picture.bit_depth,
            component,
            &deblocking,
        )?;
    }
    Ok(picture)
}

#[cfg(test)]
mod inter_tests {
    use super::super::{avc_compensation::Reference420, avc_inter_coefficients::InterCoefficients};
    use super::*;
    #[test]
    fn complete_mbaff_b_skip_pair_reconstructs_spatial_temporal_and_weighted_samples() {
        use super::super::{
            avc_direct::MbaffDirectPrediction, avc_poc::FieldOrder, avc_references::FrameReference,
            avc_slice::SliceType,
        };
        fn hex(s: &str) -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        }
        let mut sps =
            Sps::parse(&hex("6742c01fda03c045fbc044000003000400000300f03c60ca80")).unwrap();
        let mut pps = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &sps).unwrap();
        let mut header =
            SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &sps, &pps).unwrap();
        sps.width_mbs = 1;
        sps.height_map_units = 1;
        sps.crop = [0; 4];
        sps.frame_mbs_only = false;
        sps.mb_adaptive_frame_field = true;
        header.slice_type = SliceType::B;
        header.idr = false;
        header.refs_l0 = 1;
        header.refs_l1 = 1;
        header.header_bits = 0;
        header.entropy_bit_offset = 0;
        header.first_mb = 0;
        header.rbsp = vec![0x70]; // owned syntax: mb_skip_run=2, rbsp_stop_one_bit.
        let a = IntraPicture {
            coded_width: 16,
            coded_height: 32,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![20; 512],
            cb: vec![60; 128],
            cr: vec![100; 128],
        };
        let b = IntraPicture {
            y: vec![100; 512],
            cb: vec![180; 128],
            cr: vec![220; 128],
            coded_width: 16,
            coded_height: 32,
            crop: [0; 4],
            bit_depth: 8,
        };
        let l0 = [FrameReference {
            id: 42,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let l1 = [FrameReference {
            id: 70,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        let o0 = [FieldOrder {
            top: Some(0),
            bottom: Some(4),
        }];
        let o1 = [FieldOrder {
            top: Some(8),
            bottom: Some(12),
        }];
        for spatial in [false, true] {
            for implicit in [false, true] {
                pps.weighted_bipred = if implicit { 2 } else { 0 };
                let direct = MbaffDirectPrediction {
                    spatial,
                    inference8: true,
                    current_order: FieldOrder {
                        top: Some(2),
                        bottom: Some(6),
                    },
                    list0: &l0,
                    list1: &l1,
                    list0_orders: &o0,
                    list1_orders: &o1,
                    colocated: None,
                };
                let (picture, motion) = decode_inter_slices(
                    &[&header],
                    &sps,
                    &pps,
                    &[[&[&a], &[&b]]],
                    &[Some(&direct)],
                    1 << 20,
                )
                .unwrap();
                assert_eq!(picture.y, vec![if implicit { 40 } else { 60 }; 512]);
                assert_eq!(picture.cb, vec![if implicit { 90 } else { 120 }; 128]);
                assert_eq!(picture.cr, vec![if implicit { 130 } else { 160 }; 128]);
                let saved = motion
                    .snapshot_mbaff_slices(&[(0, [&[42], &[70]])], 65536)
                    .unwrap();
                for y in [0, 15, 16, 31] {
                    let (lists, field) = saved.at_mbaff([0, y]).unwrap();
                    assert!(!field);
                    assert_eq!(lists[0].unwrap().picture_id, 42);
                    assert_eq!(lists[1].unwrap().picture_id, 70);
                    assert_eq!(lists[0].unwrap().vector, [0, 0]);
                }
                assert!(
                    decode_inter_slices(
                        &[&header],
                        &sps,
                        &pps,
                        &[[&[&a], &[&b]]],
                        &[None],
                        1 << 20
                    )
                    .is_err()
                );
            }
        }
    }
    fn picture(depth: u8) -> IntraPicture {
        IntraPicture {
            coded_width: 32,
            coded_height: 32,
            crop: [0; 4],
            bit_depth: depth,
            y: vec![9; 1024],
            cb: vec![9; 256],
            cr: vec![9; 256],
        }
    }
    fn coefficients() -> InterCoefficients {
        InterCoefficients {
            luma4: [[0; 16]; 16],
            luma8: [[0; 64]; 4],
            chroma_dc: [[0; 4]; 2],
            chroma_ac: [[[0; 16]; 4]; 2],
            luma_counts: [0; 16],
            chroma_counts: [[0; 4]; 2],
        }
    }
    #[test]
    fn inter_residuals_publish_only_owned_frame_or_field_samples() {
        let scaling = ScalingMatrices {
            four: [[16; 16]; 6],
            eight: [[16; 64]; 2],
        };
        for depth in [8, 10] {
            let reference = Reference420::new(
                [&[50; 256], &[20; 64], &[20; 64]],
                16,
                16,
                [16, 8, 8],
                depth,
            )
            .unwrap();
            for field in [false, true] {
                for address in 0..4 {
                    for eight in [false, true] {
                        let mut p = picture(depth);
                        let mut readiness = Readiness420::new(2, 2, true, 28).unwrap();
                        let mut c = coefficients();
                        c.luma4[0][0] = 5;
                        c.luma8[0][0] = 5;
                        c.chroma_dc[0][0] = -2;
                        c.chroma_dc[1][0] = 7;
                        reconstruct_inter_macroblock_ready(
                            &mut p,
                            address,
                            field,
                            reference.predict([0; 2], [0; 2], [16; 2]).unwrap(),
                            Some(&c),
                            eight,
                            [0; 3],
                            true,
                            &scaling,
                            &mut readiness,
                        )
                        .unwrap();
                        assert!(
                            reconstruct_inter_macroblock_ready(
                                &mut p,
                                address,
                                field,
                                reference.predict([0; 2], [0; 2], [16; 2]).unwrap(),
                                None,
                                false,
                                [0; 3],
                                false,
                                &scaling,
                                &mut readiness
                            )
                            .is_err()
                        );
                        for (component, samples, width, height, base, first) in [
                            (0, &p.y, 32usize, 32usize, 50, 55),
                            (1, &p.cb, 16usize, 16usize, 20, 18),
                            (2, &p.cr, 16usize, 16usize, 20, 27),
                        ] {
                            let sub = if component == 0 { [1, 1] } else { [2, 2] };
                            let target = layout(address, 2, 2, true, field, sub).unwrap();
                            for y in 0..height {
                                for x in 0..width {
                                    let dx = x.checked_sub(target.origin[0]);
                                    let dy = y.checked_sub(target.origin[1]);
                                    let inside = dx.is_some_and(|v| v < target.size[0])
                                        && dy.is_some_and(|v| {
                                            v % target.row_step == 0
                                                && v / target.row_step < target.size[1]
                                        });
                                    let expected = if !inside {
                                        9
                                    } else if [x, y] == target.origin {
                                        first
                                    } else {
                                        base
                                    };
                                    assert_eq!(
                                        readiness.available(component, [x, y]).unwrap(),
                                        inside
                                    );
                                    assert_eq!(
                                        samples[y * width + x],
                                        expected,
                                        "component {component} address {address} field {field} at {x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn invalid_plane_or_residual_never_partially_publishes() {
        let scaling = ScalingMatrices {
            four: [[16; 16]; 6],
            eight: [[16; 64]; 2],
        };
        let reference =
            Reference420::new([&[50; 256], &[20; 64], &[20; 64]], 16, 16, [16, 8, 8], 8).unwrap();
        for case in 0..6 {
            let mut p = picture(8);
            let mut readiness =
                Readiness420::new(if case == 5 { 1 } else { 2 }, 2, true, 28).unwrap();
            if case == 0 {
                p.cr.pop();
            }
            if case == 1 {
                p.bit_depth = 10;
            }
            let c = coefficients();
            let mut prediction = reference.predict([0; 2], [0; 2], [16; 2]).unwrap();
            if case == 4 {
                prediction.cr[0] = 256;
            }
            assert!(
                reconstruct_inter_macroblock_ready(
                    &mut p,
                    if case == 2 { 4 } else { 0 },
                    true,
                    prediction,
                    Some(&c),
                    false,
                    if case == 3 { [255; 3] } else { [0; 3] },
                    false,
                    &scaling,
                    &mut readiness
                )
                .is_err()
            );
            assert!(p.y.iter().chain(&p.cb).chain(&p.cr).all(|&v| v == 9));
            for component in 0..3 {
                assert!(!readiness.available(component, [0, 0]).unwrap());
            }
        }
    }
}
