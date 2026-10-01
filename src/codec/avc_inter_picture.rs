//! Single-slice progressive CAVLC/CABAC P/B-picture reconstruction, resolved scaling lists.
use super::{
    avc::{Pps, SliceGroups, Sps},
    avc_boundary::{BlockEdge, DecodedBlockEdges, MotionReference},
    avc_compensation::{ComponentWeight, InterLumaResidual, Reference420},
    avc_deblock::inter_plane,
    avc_inter::{Partition, Prediction},
    avc_inter_coefficients::InterCoefficients,
    avc_inter_prediction::predict_macroblock,
    avc_inter_slice::{InterCavlcSlice, InterMacroblock},
    avc_macroblock::IntraMacroblock,
    avc_motion_field::MotionField,
    avc_mv::Neighbour,
    avc_picture::{IntraPicture, chroma_qp},
    avc_slice::{SliceHeader, SliceType},
};
use super::{avc_boundary::row_edges, avc_deblock::MacroblockEdges};
use crate::{Result, invalid};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// Everything an inter macroblock needs to be predicted and reconstructed
/// once entropy decoding and motion derivation (both serial) are done.
struct InterJob {
    slice: usize,
    origin: [usize; 2],
    parts: Vec<Partition>,
    vectors: Vec<[Neighbour; 2]>,
    weights: Option<Vec<[[ComponentWeight; 3]; 2]>>,
    coefficients: Option<Box<InterCoefficients>>,
    eight: bool,
    qps: [u8; 3],
    bypass: bool,
}
/// Decode-order record replayed by pass B (see below).
enum Order {
    SliceBegin,
    Inter([usize; 2]),
    Intra(Box<IntraMacroblock>),
}

/// Predict and reconstruct one inter macroblock into its row band
/// (`y` is 16 rows, `cb`/`cr` 8 rows, all starting at the band's top).
fn reconstruct_inter_job(
    job: InterJob,
    depth: u8,
    scaling: &super::avc_scaling::ScalingMatrices,
    refs: &[Vec<&Reference420<'_>>; 2],
    w: usize,
    y: &mut [u16],
    cb: &mut [u16],
    cr: &mut [u16],
) -> Result<()> {
    let prediction = predict_macroblock(
        job.origin.map(|n| n as i32),
        depth,
        &job.parts,
        &job.vectors,
        [&refs[0], &refs[1]],
        job.weights.as_deref(),
    )?;
    let prediction = if let Some(c) = &job.coefficients {
        let luma = if job.eight {
            InterLumaResidual::Blocks8(&c.luma8)
        } else {
            InterLumaResidual::Blocks4(&c.luma4)
        };
        if job.bypass {
            prediction.reconstruct_inter_bypass(luma, &c.chroma_dc, &c.chroma_ac)?
        } else {
            prediction.reconstruct_inter(
                luma,
                &c.chroma_dc,
                &c.chroma_ac,
                job.qps,
                &[scaling.four[3], scaling.four[4], scaling.four[5]],
                &scaling.eight[1],
            )?
        }
    } else {
        prediction
    };
    let x = job.origin[0];
    for (plane, src, stride, size, px) in [
        (y, prediction.y.as_slice(), w, 16, x),
        (cb, prediction.cb.as_slice(), w / 2, 8, x / 2),
        (cr, prediction.cr.as_slice(), w / 2, 8, x / 2),
    ] {
        for row in 0..size {
            plane[row * stride + px..][..size].copy_from_slice(&src[row * size..][..size]);
        }
    }
    Ok(())
}
/// Reference order is the already modified L0 list. Picture identities must be
/// stable; aliases of one reference must point to the same picture object.
pub fn decode_p_picture(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    references: &[&IntraPicture],
    budget: usize,
) -> Result<IntraPicture> {
    decode_p_picture_with_motion(header, sps, pps, references, budget).map(|(picture, _)| picture)
}
/// Retains the decoded working motion field for reference-picture snapshots.
/// The returned field is already included in the reconstruction memory budget.
pub fn decode_p_picture_with_motion(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    references: &[&IntraPicture],
    budget: usize,
) -> Result<(IntraPicture, MotionField)> {
    decode_inter_picture_with_motion(header, sps, pps, [references, &[]], None, budget)
}
/// Reconstruct a progressive P/B slice. B pictures require reference metadata
/// and retained co-located motion matching the supplied picture lists.
pub fn decode_inter_picture_with_motion(
    header: &SliceHeader,
    sps: &Sps,
    pps: &Pps,
    references: [&[&IntraPicture]; 2],
    direct: Option<&super::avc_direct::DirectPrediction<'_>>,
    budget: usize,
) -> Result<(IntraPicture, MotionField)> {
    decode_inter_slices_with_motion(&[header], sps, pps, references, direct, budget)
}
/// Shared reconstruction for slices using the same resolved reference lists.
pub fn decode_inter_slices_with_motion(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: [&[&IntraPicture]; 2],
    direct: Option<&super::avc_direct::DirectPrediction<'_>>,
    budget: usize,
) -> Result<(IntraPicture, MotionField)> {
    let header = *headers
        .first()
        .ok_or_else(|| invalid("missing inter slices"))?;
    if headers.iter().any(|h| {
        h.slice_type != header.slice_type
            || h.refs_l0 != header.refs_l0
            || h.refs_l1 != header.refs_l1
            || h.modifications_l0 != header.modifications_l0
            || h.modifications_l1 != header.modifications_l1
            || h.field_pic
            || h.redundant_pic_cnt != 0
            || h.disable_deblocking_filter_idc > 2
    }) {
        return Err(invalid(
            "inter slices require individually resolved reference lists",
        ));
    }
    decode_inter_resolved_slices_with_motion(
        headers,
        sps,
        pps,
        &vec![references; headers.len()],
        &vec![direct; headers.len()],
        budget,
    )
}
/// Reconstruct slices with their independently resolved picture and direct lists.
pub fn decode_inter_resolved_slices_with_motion(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references_by_slice: &[[&[&IntraPicture]; 2]],
    direct_by_slice: &[Option<&super::avc_direct::DirectPrediction<'_>>],
    budget: usize,
) -> Result<(IntraPicture, MotionField)> {
    let header = *headers
        .first()
        .ok_or_else(|| invalid("missing inter slices"))?;
    if headers.len() != references_by_slice.len()
        || headers.len() != direct_by_slice.len()
        || headers.iter().any(|h| {
            !matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B)
                || h.field_pic
                || h.redundant_pic_cnt != 0
                || h.disable_deblocking_filter_idc > 2
        })
    {
        return Err(invalid("invalid resolved inter slice contexts"));
    }
    if !matches!(header.slice_type, SliceType::I | SliceType::P | SliceType::B)
        || header.first_mb != 0
        || header.disable_deblocking_filter_idc > 2
        || header.field_pic
        || !sps.frame_mbs_only
        || sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.bit_depth_luma != sps.bit_depth_chroma
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || header.redundant_pic_cnt != 0
    {
        return Err(invalid("unsupported inter-picture reconstruction tools"));
    }
    let scaling = super::avc_scaling::ScalingMatrices::new(sps, pps)?;
    let (width, height) = sps.coded_dimensions();
    let (w, h) = (width as usize, height as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("inter-picture size overflow"))?;
    let count = pixels / 256;
    // Conservative upper bound for motion/count fields, edge state and transient
    // grids, including scratch coefficients. Reference pictures are caller owned.
    let required = pixels
        .checked_mul(3)
        .and_then(|n| count.checked_mul(4096).and_then(|m| n.checked_add(m)))
        .and_then(|n| n.checked_add(16384))
        .ok_or_else(|| invalid("inter-picture budget overflow"))?;
    if w == 0 || h == 0 || count > 65536 || required > budget {
        return Err(invalid("inter-picture exceeds memory budget"));
    }
    let mut slice_planes = Vec::with_capacity(headers.len());
    for ((header, references), direct) in
        headers.iter().zip(references_by_slice).zip(direct_by_slice)
    {
        let is_b = header.slice_type == SliceType::B;
        if is_b && direct.is_none() {
            return Err(invalid("B-slice direct metadata is missing"));
        }
        let lengths = [
            if header.slice_type==SliceType::I {0} else {header.refs_l0 as usize},
            if is_b { header.refs_l1 as usize } else { 0 },
        ];
        for list in 0..2 {
            if references[list].len() != lengths[list]
                || lengths[list] > 32
                || (list == 0 || is_b) && header.slice_type!=SliceType::I && lengths[list] == 0
            {
                return Err(invalid("inter-picture reference count mismatch"));
            }
        }
        if let Some(context) = direct {
            if context.list0.len() != lengths[0]
                || context.list1.len() != lengths[1]
                || context.inference8 != sps.direct_8x8_inference
                || context
                    .colocated
                    .is_some_and(|field| field.dimensions() != [w, h])
            {
                return Err(invalid("direct metadata reference count mismatch"));
            }
        }
        let mut planes = [Vec::new(), Vec::new()];
        for list in 0..2 {
            for r in references[list] {
                if r.coded_width != w || r.coded_height != h || r.bit_depth != sps.bit_depth_luma {
                    return Err(invalid("inter-picture reference format mismatch"));
                }
                planes[list].push(Reference420::from_decoded(
                    [&r.y, &r.cb, &r.cr],
                    w,
                    h,
                    [w, w / 2, w / 2],
                    r.bit_depth,
                )?);
            }
        }
        slice_planes.push(planes);
    }
    let refs: Vec<_> = slice_planes
        .iter()
        .map(|planes| {
            [
                planes[0].iter().collect::<Vec<_>>(),
                planes[1].iter().collect::<Vec<_>>(),
            ]
        })
        .collect();
    let mut ready = vec![0u8; count * 16];
    let mut motion = MotionField::new(w, h, count * 4096)?;
    let mut out = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|v| v as usize),
        bit_depth: sps.bit_depth_luma,
        y: vec![0; pixels],
        cb: vec![0; pixels / 4],
        cr: vec![0; pixels / 4],
    };
    let width_mbs = w / 16;
    let row_count = h / 16;
    // Streaming reconstruction. This thread parses macroblocks in decode
    // order (entropy decoding, motion derivation, edge metadata) and marks
    // each finished row; worker threads reconstruct a finished row's inter
    // macroblocks into their own band of the output planes and derive its
    // deblocking edges while parsing continues. Intra macroblocks are
    // reconstructed afterwards in decode order (pass B below).
    let mut order: Vec<Order> = Vec::with_capacity(count);
    let edge_rows: Vec<Mutex<Vec<DecodedBlockEdges>>> = (0..row_count)
        .map(|_| Mutex::new(Vec::with_capacity(width_mbs)))
        .collect();
    let grid_rows: Vec<Mutex<Option<[Vec<MacroblockEdges>; 3]>>> =
        (0..row_count).map(|_| Mutex::new(None)).collect();
    // Rows the parser has completed; `usize::MAX` tells workers to stop.
    let progress = (Mutex::new(0usize), Condvar::new());
    let row_done = |seen: usize| {
        if seen % width_mbs == 0 {
            let (rows, signal) = &progress;
            *rows.lock().unwrap_or_else(|e| e.into_inner()) = seen / width_mbs;
            signal.notify_all();
        }
    };
    {
        let bands: Vec<Mutex<(&mut [u16], &mut [u16], &mut [u16], Vec<InterJob>)>> = out
            .y
            .chunks_mut(16 * w)
            .zip(out.cb.chunks_mut(8 * (w / 2)))
            .zip(out.cr.chunks_mut(8 * (w / 2)))
            .map(|((y, cb), cr)| Mutex::new((y, cb, cr, Vec::new())))
            .collect();
        let next = AtomicUsize::new(0);
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(row_count)
            .max(1);
        let depth = sps.bit_depth_luma;
        let refs = &refs;
        let scaling = &scaling;
        let results: Vec<Result<()>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let (bands, next, edge_rows, grid_rows, progress) =
                        (&bands, &next, &edge_rows, &grid_rows, &progress);
                    scope.spawn(move || -> Result<()> {
                        loop {
                            let row = next.fetch_add(1, Ordering::Relaxed);
                            if row >= bands.len() {
                                return Ok(());
                            }
                            {
                                let (rows, signal) = progress;
                                let mut done = rows.lock().unwrap_or_else(|e| e.into_inner());
                                while *done <= row {
                                    done = signal.wait(done).unwrap_or_else(|e| e.into_inner());
                                }
                                if *done == usize::MAX {
                                    return Ok(());
                                }
                            }
                            let mut band = bands[row].lock().unwrap_or_else(|e| e.into_inner());
                            let (y, cb, cr, jobs) = &mut *band;
                            for job in jobs.drain(..) {
                                let slice = job.slice;
                                reconstruct_inter_job(
                                    job,
                                    depth,
                                    scaling,
                                    &refs[slice],
                                    w,
                                    y,
                                    cb,
                                    cr,
                                )?;
                            }
                            drop(band);
                            let current = edge_rows[row].lock().unwrap_or_else(|e| e.into_inner());
                            let previous = row
                                .checked_sub(1)
                                .map(|r| edge_rows[r].lock().unwrap_or_else(|e| e.into_inner()));
                            let grid = row_edges(previous.as_deref().map(Vec::as_slice), &current)?;
                            *grid_rows[row].lock().unwrap_or_else(|e| e.into_inner()) = Some(grid);
                        }
                    })
                })
                .collect();
            let parsed: Result<()> = (|| {
                let mut seen = 0;
                for (slice_index, header) in headers.iter().enumerate() {
                    let slice_id = slice_index as u32;
                    let is_b = header.slice_type == SliceType::B;
                    let explicit_weights = if is_b { pps.weighted_bipred == 1 } else { header.slice_type==SliceType::P && pps.weighted_pred };
                    let lengths = [
                        if header.slice_type==SliceType::I {0} else {header.refs_l0 as usize},
                        if is_b { header.refs_l1 as usize } else { 0 },
                    ];
                    let references = references_by_slice[slice_index];
                    let all_references: Vec<_> =
                        references.into_iter().flatten().copied().collect();
                    let direct = direct_by_slice[slice_index];
                    let slice_direct = direct.map(|context| super::avc_direct::DirectPrediction {
                        spatial: header.direct_spatial_mv_pred,
                        inference8: context.inference8,
                        current_poc: context.current_poc,
                        list0: context.list0,
                        list1: context.list1,
                        colocated: context.colocated,
                    });
                    let direct = slice_direct.as_ref();
                    if header.first_mb as usize != seen {
                        return Err(invalid("inter slice coverage gap or overlap"));
                    }
                    let end = headers
                        .get(slice_index + 1)
                        .map_or(count, |next| next.first_mb as usize);
                    if end <= seen || end > count {
                        return Err(invalid("invalid inter slice range"));
                    }
                    if explicit_weights
                        && header
                            .weights
                            .as_ref()
                            .is_none_or(|w| w.l0.len() != lengths[0] || w.l1.len() != lengths[1])
                    {
                        return Err(invalid("inter slice weight table is incomplete"));
                    }
                    order.push(Order::SliceBegin);
                    let mut cavlc = if pps.cabac || header.slice_type==SliceType::I {
                        None
                    } else {
                        Some(InterCavlcSlice::new_mixed(header, sps, pps, count * 4096)?)
                    };
                    let mut cabac = if pps.cabac && header.slice_type!=SliceType::I {
                        Some(super::avc_cabac_slice::InterCabacSlice::new(
                            header,
                            sps,
                            pps,
                            count * 4096,
                        )?)
                    } else {
                        None
                    };

                    let mut intra_cavlc = if !pps.cabac && header.slice_type==SliceType::I {
                        Some(super::avc_macroblock::IntraCavlcReader::new(header,sps,pps,count*4096)?)
                    } else {None};
                    let mut intra_cabac = if pps.cabac && header.slice_type==SliceType::I {
                        Some(super::avc_cabac_macroblock::IntraCabacReader::new(header,sps,pps,count*4096)?)
                    } else {None};
                    while let Some(mb) = match (&mut cabac, &mut cavlc, &mut intra_cabac, &mut intra_cavlc) {
                        (Some(reader), _, _, _) => reader.read_macroblock()?,
                        (_, Some(reader), _, _) => reader.read_macroblock()?,
                        (_, _, Some(reader), _) => reader.read_macroblock()?.map(|b|InterMacroblock::Intra(Box::new(b))),
                        (_, _, _, Some(reader)) => reader.read_macroblock()?.map(|b|InterMacroblock::Intra(Box::new(b))),
                        _ => return Err(invalid("missing AVC entropy reader")),
                    } {
                        if let InterMacroblock::Intra(block) = mb {
                            let address = block.address as usize;
                            if address != seen || seen >= end {
                                return Err(invalid("inter slice exceeds assigned range"));
                            }
                            motion.store(
                                [address % (w / 16) * 16, address / (w / 16) * 16],
                                [16, 16],
                                slice_id,
                                [Neighbour::NoPrediction; 2],
                            )?;
                            let bd = 6 * (i32::from(sps.bit_depth_luma) - 8);
                            let qps = [
                                block.qp,
                                i32::from(chroma_qp(
                                    block.qp,
                                    pps.chroma_qp_offset,
                                    sps.bit_depth_chroma,
                                )) - bd,
                                i32::from(chroma_qp(
                                    block.qp,
                                    pps.second_chroma_qp_offset,
                                    sps.bit_depth_chroma,
                                )) - bd,
                            ];
                            edge_rows[address / width_mbs]
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push(DecodedBlockEdges {
                                    blocks: [BlockEdge {
                                        intra: true,
                                        switching_slice: false,
                                        nonzero_luma: false,
                                        motion: [None; 2],
                                    }; 16],
                                    qp: qps,
                                    slice_id,
                                    disable_filter: header.disable_deblocking_filter_idc as u8,
                                    offsets: [header.alpha_offset, header.beta_offset],
                                    transform8: matches!(
                                        block.luma,
                                        super::avc_macroblock::IntraLuma::Blocks8 { .. }
                                    ),
                                });
                            order.push(Order::Intra(block));
                            seen += 1;
                            row_done(seen);
                            continue;
                        }

                        let (address, qp, parts, coefficients, eight) = match mb {
                            InterMacroblock::Intra(_) => {
                                return Err(invalid(
                                    "mixed intra picture reconstruction is not connected",
                                ));
                            }
                            InterMacroblock::Skip { address, qp } if is_b => {
                                let super::avc_inter::MacroblockType::Inter { partitions, .. } =
                                    super::avc_inter::macroblock_type(SliceType::B, 0)?
                                else {
                                    return Err(invalid("invalid B-skip partition layout"));
                                };
                                (address, qp, partitions, None, false)
                            }
                            InterMacroblock::Skip { address, qp } => (
                                address,
                                qp,
                                vec![Partition {
                                    origin: [0, 0],
                                    size: [16, 16],
                                    prediction: Prediction::L0,
                                    group: 0,
                                    references: [Some(0), None],
                                    differences: [[0; 2]; 2],
                                }],
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
                        };
                        if address != seen || seen >= end {
                            return Err(invalid("inter slice exceeds assigned range"));
                        }
                        let origin = [address % (w / 16) * 16, address / (w / 16) * 16];
                        let vectors = if coefficients.is_none() && !is_b {
                            vec![[
                                Neighbour::Inter {
                                    reference: 0,
                                    vector: motion.decode_p_skip(origin, slice_id)?,
                                },
                                Neighbour::NoPrediction,
                            ]]
                        } else {
                            motion
                                .decode_macroblock_with_direct(origin, slice_id, &parts, direct)?
                        };
                        let weights = if explicit_weights || (is_b && pps.weighted_bipred == 2) {
                            let mut weights = Vec::with_capacity(parts.len());
                            for vector in &vectors {
                                let mut weight = [[ComponentWeight::default(); 3]; 2];
                                if explicit_weights {
                                    let table = header.weights.as_ref().unwrap();
                                    for list in 0..2 {
                                        if let Neighbour::Inter { reference, .. } = vector[list] {
                                            let entry = [&table.l0, &table.l1][list]
                                                .get(reference as usize)
                                                .ok_or_else(|| {
                                                    invalid("missing explicit reference weight")
                                                })?;
                                            weight[list] = [
                                                ComponentWeight {
                                                    weight: entry.luma.0,
                                                    offset: entry.luma.1,
                                                    denominator: table.luma_denom,
                                                },
                                                ComponentWeight {
                                                    weight: entry.chroma[0].0,
                                                    offset: entry.chroma[0].1,
                                                    denominator: table.chroma_denom,
                                                },
                                                ComponentWeight {
                                                    weight: entry.chroma[1].0,
                                                    offset: entry.chroma[1].1,
                                                    denominator: table.chroma_denom,
                                                },
                                            ];
                                        }
                                    }
                                } else if let [
                                    Neighbour::Inter { reference: a, .. },
                                    Neighbour::Inter { reference: b, .. },
                                ] = *vector
                                {
                                    let context = direct.unwrap();
                                    let a = context
                                        .list0
                                        .get(a as usize)
                                        .ok_or_else(|| invalid("implicit L0 reference missing"))?;
                                    let b = context
                                        .list1
                                        .get(b as usize)
                                        .ok_or_else(|| invalid("implicit L1 reference missing"))?;
                                    let values = super::avc_mv::implicit_weights(
                                        context.current_poc.into(),
                                        a.poc.into(),
                                        b.poc.into(),
                                        a.long_term_index.is_some() || b.long_term_index.is_some(),
                                    );
                                    for list in 0..2 {
                                        weight[list] = [ComponentWeight {
                                            weight: values[list],
                                            offset: 0,
                                            denominator: 5,
                                        };
                                            3];
                                    }
                                }
                                weights.push(weight);
                            }
                            Some(weights)
                        } else {
                            None
                        };
                        let bd = 6 * (i32::from(sps.bit_depth_luma) - 8);
                        let qps = [
                            (qp + bd) as u8,
                            chroma_qp(qp, pps.chroma_qp_offset, sps.bit_depth_chroma),
                            chroma_qp(qp, pps.second_chroma_qp_offset, sps.bit_depth_chroma),
                        ];
                        let empty = BlockEdge {
                            intra: false,
                            switching_slice: false,
                            nonzero_luma: false,
                            motion: [None; 2],
                        };
                        let mut blocks = [empty; 16];
                        for (p, v) in parts.iter().zip(&vectors) {
                            for y in usize::from(p.origin[1]) / 4
                                ..usize::from(p.origin[1] + p.size[1]) / 4
                            {
                                for x in usize::from(p.origin[0]) / 4
                                    ..usize::from(p.origin[0] + p.size[0]) / 4
                                {
                                    blocks[y * 4 + x].motion = std::array::from_fn(|list| {
                                        let n = v[list];
                                        if let Neighbour::Inter { reference, vector } = n {
                                            Some(MotionReference {
                                                picture: all_references
                                                    .iter()
                                                    .position(|r| {
                                                        std::ptr::eq(
                                                            *r,
                                                            references[list]
                                                                [usize::from(reference)],
                                                        )
                                                    })
                                                    .unwrap()
                                                    as u64,
                                                vector,
                                            })
                                        } else {
                                            None
                                        }
                                    });
                                }
                            }
                        }
                        if let Some(c) = &coefficients {
                            for i in 0..16 {
                                blocks[i].nonzero_luma = if eight {
                                    c.luma8[(i / 4 / 2) * 2 + (i % 4 / 2)]
                                        .iter()
                                        .any(|&v| v != 0)
                                } else {
                                    c.luma_counts[i] != 0
                                };
                            }
                        }
                        edge_rows[address / width_mbs]
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(DecodedBlockEdges {
                                blocks,
                                qp: qps.map(|q| i32::from(q) - bd),
                                slice_id,
                                disable_filter: header.disable_deblocking_filter_idc as u8,
                                offsets: [header.alpha_offset, header.beta_offset],
                                transform8: eight,
                            });
                        order.push(Order::Inter(origin));
                        bands[origin[1] / 16]
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .3
                            .push(InterJob {
                                slice: slice_index,
                                origin,
                                parts,
                                vectors,
                                weights,
                                coefficients,
                                eight,
                                bypass: sps.transform_bypass && qps[0] == 0,
                                qps,
                            });
                        seen += 1;
                        row_done(seen);
                    }
                    if seen != end {
                        return Err(invalid("incomplete inter slice"));
                    }
                }
                if seen != count {
                    return Err(invalid("incomplete inter picture"));
                }
                Ok(())
            })();
            {
                // Release every waiting worker: all rows are done, or abort.
                let (rows, signal) = &progress;
                *rows.lock().unwrap_or_else(|e| e.into_inner()) = if parsed.is_ok() {
                    row_count
                } else {
                    usize::MAX
                };
                signal.notify_all();
            }
            let mut results: Vec<Result<()>> = handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(invalid("AVC reconstruction thread panicked")))
                })
                .collect();
            results.push(parsed);
            results
        });
        for result in results {
            result?;
        }
    }
    // Pass B replays decode order: inter macroblocks mark their availability,
    // intra macroblocks predict from neighbours decoded before them, so a
    // later inter macroblock (e.g. the top-right of a right-column 4x4 block)
    // is still unavailable exactly as in single-pass decoding.
    for step in order {
        match step {
            Order::SliceBegin => ready.fill(0),
            Order::Inter(origin) => {
                for y in 0..4 {
                    for x in 0..4 {
                        ready[(origin[1] / 4 + y) * (w / 4) + origin[0] / 4 + x] =
                            u8::from(!pps.constrained_intra_pred);
                    }
                }
            }
            Order::Intra(block) => {
                super::avc_picture::reconstruct_macroblock(
                    &mut out, &block, sps, pps, &scaling, &mut ready,
                )?;
            }
        }
    }
    let mut grids: [Vec<MacroblockEdges>; 3] = std::array::from_fn(|_| Vec::with_capacity(count));
    for row in &grid_rows {
        let [y, cb, cr] = row
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| invalid("missing AVC deblocking edges"))?;
        grids[0].extend(y);
        grids[1].extend(cb);
        grids[2].extend(cr);
    }
    // The three planes deblock independently; run them on their own threads.
    let depth = sps.bit_depth_luma;
    let [y, cb, cr] = std::thread::scope(|scope| {
        let planes = [&mut out.y, &mut out.cb, &mut out.cr];
        let handles: Vec<_> = planes
            .into_iter()
            .zip(&grids)
            .enumerate()
            .map(|(component, (plane, grid))| {
                let scale = if component == 0 { 1 } else { 2 };
                scope.spawn(move || {
                    inter_plane(plane, w / scale, h / scale, depth, component != 0, grid)
                })
            })
            .collect();
        let mut results = handles.into_iter().map(|h| {
            h.join()
                .unwrap_or_else(|_| Err(invalid("AVC deblocking thread panicked")))
        });
        [
            results.next().unwrap(),
            results.next().unwrap(),
            results.next().unwrap(),
        ]
    });
    y?;
    cb?;
    cr?;
    Ok((out, motion))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn b_skip_reconstructs_spatial_temporal_and_implicit_weighted_pixels() {
        use super::super::{avc_direct::DirectPrediction, avc_references::FrameReference};
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut s = Sps::parse(&bytes).unwrap();
        s.width_mbs = 1;
        s.height_map_units = 1;
        s.crop = [0; 4];
        let mut p = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &s).unwrap();
        let mut header = SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &s, &p).unwrap();
        header.slice_type = SliceType::B;
        header.idr = false;
        header.refs_l0 = 1;
        header.refs_l1 = 1;
        header.header_bits = 0;
        header.entropy_bit_offset = 0;
        header.disable_deblocking_filter_idc = 0;
        header.rbsp = vec![0x50]; // mb_skip_run=1, rbsp_stop_one_bit.
        let a = IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![20; 256],
            cb: vec![60; 64],
            cr: vec![180; 64],
        };
        let b = IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![100; 256],
            cb: vec![180; 64],
            cr: vec![60; 64],
        };
        let l0 = [FrameReference {
            id: 10,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let l1 = [FrameReference {
            id: 20,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        for spatial in [false, true] {
            header.direct_spatial_mv_pred = spatial;
            let context = DirectPrediction {
                spatial,
                inference8: s.direct_8x8_inference,
                current_poc: 2,
                list0: &l0,
                list1: &l1,
                colocated: None,
            };
            for implicit in [false, true] {
                p.weighted_bipred = if implicit { 2 } else { 0 };
                let (picture, motion) = decode_inter_picture_with_motion(
                    &header,
                    &s,
                    &p,
                    [&[&a], &[&b]],
                    Some(&context),
                    1 << 20,
                )
                .unwrap();
                assert_eq!(picture.y, vec![if implicit { 40 } else { 60 }; 256]);
                assert_eq!(picture.cb, vec![if implicit { 90 } else { 120 }; 64]);
                assert_eq!(picture.cr, vec![if implicit { 150 } else { 120 }; 64]);
                let retained = motion.snapshot([&[10], &[20]], 65536).unwrap();
                let cell = retained.at([15, 15]).unwrap();
                assert_eq!(cell[0].unwrap().picture_id, 10);
                assert_eq!(cell[1].unwrap().picture_id, 20);
            }
        }
    }
    #[test]
    fn two_slice_p_skip_reconstructs_shared_planes_and_motion() {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut s = Sps::parse(&bytes).unwrap();
        s.width_mbs = 2;
        s.height_map_units = 1;
        s.crop = [0; 4];
        let p = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &s).unwrap();
        let mut headers: Vec<_> = (0..2)
            .map(|i| {
                let mut h =
                    SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &s, &p).unwrap();
                h.slice_type = SliceType::P;
                h.idr = false;
                h.refs_l0 = 1;
                h.first_mb = i;
                h.header_bits = 0;
                h.entropy_bit_offset = 0;
                h.disable_deblocking_filter_idc = 2;
                h.rbsp = vec![0x50];
                h
            })
            .collect();
        let reference = IntraPicture {
            coded_width: 32,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![100; 512],
            cb: vec![90; 128],
            cr: vec![180; 128],
        };
        let (picture, motion) = decode_inter_slices_with_motion(
            &[&headers[0], &headers[1]],
            &s,
            &p,
            [&[&reference], &[]],
            None,
            1 << 20,
        )
        .unwrap();
        assert_eq!(picture.y, reference.y);
        assert_eq!(picture.cb, reference.cb);
        assert_eq!(picture.cr, reference.cr);
        let saved = motion
            .snapshot_slices(&[(0, [&[12], &[]]), (1, [&[12], &[]])], 65536)
            .unwrap();
        assert_eq!(saved.at([0, 0]).unwrap()[0].unwrap().picture_id, 12);
        assert_eq!(saved.at([16, 0]).unwrap()[0].unwrap().vector, [0, 0]);
        let other = IntraPicture {
            coded_width: 32,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![200; 512],
            cb: vec![130; 128],
            cr: vec![50; 128],
        };
        let (mixed, motion) = decode_inter_resolved_slices_with_motion(
            &[&headers[0], &headers[1]],
            &s,
            &p,
            &[[&[&reference], &[]], [&[&other], &[]]],
            &[None, None],
            1 << 20,
        )
        .unwrap();
        for row in mixed.y.chunks_exact(32) {
            assert_eq!(&row[..16], &[100; 16]);
            assert_eq!(&row[16..], &[200; 16]);
        }
        let saved = motion
            .snapshot_slices(&[(0, [&[12], &[]]), (1, [&[24], &[]])], 65536)
            .unwrap();
        assert_eq!(saved.at([0, 0]).unwrap()[0].unwrap().picture_id, 12);
        assert_eq!(saved.at([16, 0]).unwrap()[0].unwrap().picture_id, 24);
        headers[1].first_mb = 0;
        assert!(
            decode_inter_slices_with_motion(
                &[&headers[0], &headers[1]],
                &s,
                &p,
                [&[&reference], &[]],
                None,
                1 << 20
            )
            .is_err()
        );
    }
    #[test]
    fn two_slice_b_skip_supports_independent_direct_modes() {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut s = Sps::parse(&bytes).unwrap();
        s.width_mbs = 2;
        s.height_map_units = 1;
        s.crop = [0; 4];
        let p = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &s).unwrap();
        let mut headers: Vec<_> = (0..2)
            .map(|i| {
                let mut h =
                    SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &s, &p).unwrap();
                h.slice_type = SliceType::B;
                h.refs_l1 = 1;
                h.direct_spatial_mv_pred = i == 0;
                h.idr = false;
                h.refs_l0 = 1;
                h.first_mb = i;
                h.header_bits = 0;
                h.entropy_bit_offset = 0;
                h.disable_deblocking_filter_idc = 2;
                h.rbsp = vec![0x50];
                h
            })
            .collect();
        let reference = IntraPicture {
            coded_width: 32,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: vec![100; 512],
            cb: vec![90; 128],
            cr: vec![180; 128],
        };
        let l0 = [super::super::avc_references::FrameReference {
            id: 12,
            frame_num: 0,
            poc: 0,
            long_term_index: None,
        }];
        let l1 = [super::super::avc_references::FrameReference {
            id: 13,
            frame_num: 1,
            poc: 8,
            long_term_index: None,
        }];
        let mut colocated = MotionField::new(32, 16, 65536).unwrap();
        for x in [0, 16] {
            colocated
                .store(
                    [x, 0],
                    [16, 16],
                    0,
                    [
                        Neighbour::Inter {
                            reference: 0,
                            vector: [8, 0],
                        },
                        Neighbour::NoPrediction,
                    ],
                )
                .unwrap();
        }
        let colocated = colocated.snapshot([&[12], &[]], 65536).unwrap();
        let context = super::super::avc_direct::DirectPrediction {
            spatial: true,
            inference8: s.direct_8x8_inference,
            current_poc: 2,
            list0: &l0,
            list1: &l1,
            colocated: Some(&colocated),
        };
        let (picture, motion) = decode_inter_slices_with_motion(
            &[&headers[0], &headers[1]],
            &s,
            &p,
            [&[&reference], &[&reference]],
            Some(&context),
            1 << 20,
        )
        .unwrap();
        assert_eq!(picture.y, reference.y);
        assert_eq!(picture.cb, reference.cb);
        assert_eq!(picture.cr, reference.cr);
        let saved = motion
            .snapshot_slices(&[(0, [&[12], &[13]]), (1, [&[12], &[13]])], 65536)
            .unwrap();
        assert_eq!(saved.at([0, 0]).unwrap()[0].unwrap().picture_id, 12);
        assert_eq!(saved.at([0, 0]).unwrap()[0].unwrap().vector, [0, 0]);
        assert_eq!(saved.at([16, 0]).unwrap()[0].unwrap().vector, [2, 0]);
        assert_eq!(saved.at([16, 0]).unwrap()[1].unwrap().vector, [-6, 0]);
        headers[1].first_mb = 0;
        assert!(
            decode_inter_slices_with_motion(
                &[&headers[0], &headers[1]],
                &s,
                &p,
                [&[&reference], &[&reference]],
                Some(&context),
                1 << 20
            )
            .is_err()
        );
    }
    #[test]
    fn full_skip_and_zero_residual_pictures_copy_reference() {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut s = Sps::parse(&bytes).unwrap();
        s.width_mbs = 1;
        s.height_map_units = 1;
        s.crop = [0; 4];
        let p = Pps::parse(&[0x68, 0xce, 0x09, 0xc8], &s).unwrap();
        let mut header = SliceHeader::parse(&[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80], &s, &p).unwrap();
        header.slice_type = SliceType::P;
        header.idr = false;
        header.refs_l0 = 1;
        header.header_bits = 0;
        header.entropy_bit_offset = 0;
        header.disable_deblocking_filter_idc = 0;
        let reference = IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: 8,
            y: (0..256).map(|i| i as u16).collect(),
            cb: vec![90; 64],
            cr: vec![180; 64],
        };
        // skip_run=1 then stop; or skip_run=0 + zero-residual inter MB then stop.
        for payload in [0x50, 0xfc] {
            header.rbsp = vec![payload];
            let result = decode_p_picture(&header, &s, &p, &[&reference], 1 << 20).unwrap();
            assert_eq!(result.y, reference.y);
            assert_eq!(result.cb, reference.cb);
            assert_eq!(result.cr, reference.cr);
        }
        assert!(decode_p_picture(&header, &s, &p, &[&reference], 1).is_err());
    }
}
