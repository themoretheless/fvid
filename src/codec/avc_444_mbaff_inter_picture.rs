//! Nonseparate MBAFF: common pair/motion syntax, three full-resolution residual planes.
use super::super::super::{
    avc_deblock::{MbaffBlockEdges, mbaff_inter_plane_subsampled},
    avc_direct::MbaffDirectPrediction,
    avc_mbaff::Readiness420,
    avc_mbaff_picture::{
        reconstruct_inter_macroblock, reconstruct_inter_macroblock_ready,
        reconstruct_intra_macroblock,
    },
};
use super::*;
pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[Option<&IntraPicture>]; 2]],
    direct: Option<&[Option<&MbaffDirectPrediction<'_>>]>,
    budget: usize,
    filtered: bool,
) -> Result<(IntraPicture, MotionField)> {
    if headers.is_empty()
        || headers.len() != references.len()
        || direct.is_some_and(|d| d.len() != headers.len())
        || sps.chroma_format != 3
        || sps.separate_colour_plane
        || sps.frame_mbs_only
        || !sps.mb_adaptive_frame_field
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || headers.iter().any(|h| {
            h.field_pic
                || !matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B)
                || h.redundant_pic_cnt != 0
                || h.disable_deblocking_filter_idc > 2
        })
    {
        return Err(unsupported(
            "nonseparate 444 MBAFF requires single-group I/P/B frame slices",
        ));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("444 MBAFF picture overflow"))?;
    let count = pixels / 256;
    let view_count = references
        .iter()
        .try_fold(0usize, |maximum, lists| {
            lists[0]
                .len()
                .checked_add(lists[1].len())
                .and_then(|n| n.checked_mul(3))
                .map(|n| maximum.max(n))
        })
        .ok_or_else(|| invalid("444 MBAFF view count overflow"))?;
    let view_bytes = view_count
        .checked_mul(
            std::mem::size_of::<Reference420<'_>>() + std::mem::size_of::<&Reference420<'_>>(),
        )
        .ok_or_else(|| invalid("444 MBAFF reference view overflow"))?;
    // Three edge grids plus common motion, entropy/count/pair/readiness and
    // reference-identity metadata. A monochrome prediction view can be live.
    let per_mb = 3 * std::mem::size_of::<MbaffBlockEdges>() + 2048;
    let required = super::super::super::avc_picture::decoded_storage(sps, w, h)?
        .checked_mul(2)
        .and_then(|n| count.checked_mul(per_mb).and_then(|m| n.checked_add(m)))
        .and_then(|n| n.checked_add(view_bytes))
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| invalid("444 MBAFF storage overflow"))?;
    if count == 0 || count > 65536 || headers.len() > count / 2 || required > budget {
        return Err(invalid("444 MBAFF inter exceeds memory budget"));
    }
    let first = headers[0];
    if headers.iter().any(|h| {
        h.pps_id != first.pps_id
            || h.frame_num != first.frame_num
            || h.idr != first.idr
            || h.idr_pic_id != first.idr_pic_id
            || h.poc_lsb != first.poc_lsb
            || h.delta_poc_bottom != first.delta_poc_bottom
            || h.delta_poc != first.delta_poc
            || (h.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
    }) {
        return Err(invalid("444 MBAFF slices belong to different pictures"));
    }
    let mono: [Sps; 3] = std::array::from_fn(|p| {
        let mut c = sps.clone();
        c.chroma_format = 0;
        c.separate_colour_plane = false;
        c.scaling_lists = None;
        c.bit_depth_luma = if p == 0 {
            sps.bit_depth_luma
        } else {
            sps.bit_depth_chroma
        };
        c.bit_depth_chroma = c.bit_depth_luma;
        c
    });
    let scaling = [
        super::super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 0)?,
        super::super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 1)?,
        super::super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 2)?,
    ];
    let mut pictures = Vec::new();
    let mut ready = Vec::new();
    let mut edges: [Vec<MbaffBlockEdges>; 3] = std::array::from_fn(|_| Vec::new());
    for component in &mono {
        let mut y = Vec::new();
        y.try_reserve_exact(pixels)
            .map_err(|_| invalid("cannot allocate 444 MBAFF plane"))?;
        y.resize(pixels, 0u16);
        pictures.push(IntraPicture {
            coded_width: w,
            coded_height: h,
            crop: [0; 4],
            bit_depth: component.bit_depth_luma,
            chroma_bit_depth: component.bit_depth_luma,
            y,
            cb: Vec::new(),
            cr: Vec::new(),
        });
        ready.push(Readiness420::new(w / 16, h / 16, true, count * 7)?);
    }
    for grid in &mut edges {
        grid.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate 444 MBAFF edges"))?;
    }
    let mut unique: Vec<&IntraPicture> = Vec::new();
    for lists in references {
        for list in lists {
            for picture in list.iter().flatten() {
                if !unique.iter().any(|p| std::ptr::eq(*p, *picture)) {
                    unique
                        .try_reserve(1)
                        .map_err(|_| invalid("cannot allocate 444 MBAFF identities"))?;
                    unique.push(picture);
                }
            }
        }
    }
    let mut motion = MotionField::new(w, h, count * 2048)?;
    let mut seen = 0;
    for (slice, header) in headers.iter().enumerate() {
        if header.first_mb as usize * 2 != seen {
            return Err(invalid("444 MBAFF slice coverage gap or overlap"));
        }
        let end = headers
            .get(slice + 1)
            .map_or(count, |h| h.first_mb as usize * 2);
        if end <= seen || end > count {
            return Err(invalid("invalid 444 MBAFF slice range"));
        }
        let context = direct.and_then(|d| d[slice]);
        let is_b = header.slice_type == SliceType::B;
        let lengths = [
            if header.slice_type == SliceType::I {
                0
            } else {
                header.refs_l0 as usize
            },
            if is_b { header.refs_l1 as usize } else { 0 },
        ];
        for list in 0..2 {
            if references[slice][list].len() != lengths[list] || lengths[list] > 32 {
                return Err(invalid("444 MBAFF reference count mismatch"));
            }
        }
        if is_b && context.is_none() {
            return Err(invalid("missing 444 MBAFF direct metadata"));
        }
        if let Some(context) = context {
            if context.list0.len() != lengths[0]
                || context.list1.len() != lengths[1]
                || context.list0_orders.len() != lengths[0]
                || context.list1_orders.len() != lengths[1]
                || context.inference8 != sps.direct_8x8_inference
                || context.colocated.is_some_and(|m| m.dimensions() != [w, h])
            {
                return Err(invalid("invalid 444 MBAFF direct geometry"));
            }
        }
        let mut views: [Vec<Reference420<'_>>; 6] = std::array::from_fn(|_| Vec::new());
        for p in 0..3 {
            for list in 0..2 {
                views[p * 2 + list]
                    .try_reserve_exact(lengths[list])
                    .map_err(|_| invalid("cannot allocate 444 MBAFF reference views"))?;
                for picture in references[slice][list] {
                    let view = if let Some(picture) = picture {
                        if picture.coded_width != w
                            || picture.coded_height != h
                            || picture.component_depths()
                                != [
                                    sps.bit_depth_luma,
                                    sps.bit_depth_chroma,
                                    sps.bit_depth_chroma,
                                ]
                            || picture.chroma_format()? != 3
                        {
                            return Err(invalid("444 MBAFF reference format mismatch"));
                        }
                        Reference420::from_decoded_monochrome(
                            [&picture.y, &picture.cb, &picture.cr][p],
                            w,
                            h,
                            [mono[p].bit_depth_luma; 2],
                        )?
                    } else {
                        Reference420::unavailable(mono[p].bit_depth_luma)?
                    };
                    views[p * 2 + list].push(view);
                }
            }
        }
        let refs: [Vec<&Reference420<'_>>; 6] = std::array::from_fn(|i| views[i].iter().collect());
        let capacity = views
            .iter()
            .map(|v| v.capacity() * std::mem::size_of::<Reference420<'_>>())
            .sum::<usize>()
            + refs
                .iter()
                .map(|v| v.capacity() * std::mem::size_of::<&Reference420<'_>>())
                .sum::<usize>();
        if capacity > budget - (required - view_bytes) {
            return Err(invalid(
                "444 MBAFF reference capacities exceed memory budget",
            ));
        }
        for grid in &mut ready {
            grid.reset_slice()
        }
        let mut reader = if pps.cabac {
            Reader::Cabac(
                super::super::super::avc_cabac_slice::InterCabacSlice::new_444(
                    header,
                    &mono[0],
                    pps,
                    sps.bit_depth_chroma,
                    count * 4096,
                )?,
            )
        } else {
            Reader::Cavlc(CavlcReader::new(header, &mono, pps, count)?)
        };
        while let Some(mb) = reader.read()? {
            let address = match &mb {
                Macroblock::Intra(m) => m[0].address as usize,
                Macroblock::Skip { address, .. } | Macroblock::Coded { address, .. } => *address,
            };
            if address != seen || seen >= end {
                return Err(invalid("444 MBAFF macroblock exceeds slice range"));
            }
            let field = reader.field_decoding();
            let slice_id = slice as u32;
            let (qp, eight, intra) = match &mb {
                Macroblock::Intra(m) => (
                    m[0].qp,
                    matches!(m[0].luma, IntraLuma::Blocks8 { .. }),
                    true,
                ),
                Macroblock::Skip { qp, .. } => (*qp, false, false),
                Macroblock::Coded { header, .. } => {
                    (header.residual.qp, header.residual.transform8, false)
                }
            };
            let primes = [
                (super::super::super::avc_residual_syntax::update_qp(qp, 0, sps.bit_depth_luma)?
                    + 6 * (i32::from(sps.bit_depth_luma) - 8)) as u8,
                chroma_qp(qp, pps.chroma_qp_offset, sps.bit_depth_chroma),
                chroma_qp(qp, pps.second_chroma_qp_offset, sps.bit_depth_chroma),
            ];
            let actual: [i32; 3] = std::array::from_fn(|p| {
                i32::from(primes[p]) - 6 * (i32::from(mono[p].bit_depth_luma) - 8)
            });
            let mut blocks = [[BlockEdge {
                intra,
                switching_slice: false,
                nonzero_luma: false,
                motion: [None; 2],
            }; 16]; 3];
            match mb {
                Macroblock::Intra(components) => {
                    motion.store_mbaff(
                        address,
                        [0, 0],
                        [16, 16],
                        slice_id,
                        field,
                        [Neighbour::NoPrediction; 2],
                    )?;
                    let bypass = sps.transform_bypass && primes[0] == 0;
                    for (p, mut mb) in components.into_iter().enumerate() {
                        let mut component = mono[p].clone();
                        component.transform_bypass = bypass;
                        mb.qp = if bypass {
                            -6 * (i32::from(component.bit_depth_luma) - 8)
                        } else {
                            actual[p]
                        };
                        reconstruct_intra_macroblock(
                            &mut pictures[p],
                            mb,
                            field,
                            &component,
                            pps,
                            &scaling[p],
                            &mut ready[p],
                        )?;
                    }
                }
                inter => {
                    let (skip, parts, coefficients) = match inter {
                        Macroblock::Skip { .. } => {
                            let parts = if is_b {
                                let super::super::super::avc_inter::MacroblockType::Inter {
                                    partitions,
                                    ..
                                } = super::super::super::avc_inter::macroblock_type(
                                    SliceType::B,
                                    0,
                                )?
                                else {
                                    return Err(invalid("invalid MBAFF B skip"));
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
                            };
                            (true, parts, None)
                        }
                        Macroblock::Coded {
                            header,
                            coefficients,
                            ..
                        } => (false, header.partitions, Some(coefficients)),
                        _ => unreachable!(),
                    };
                    let vectors = if skip && !is_b {
                        vec![[
                            Neighbour::Inter {
                                reference: 0,
                                vector: motion.decode_p_skip_mbaff(address, slice_id, |p| {
                                    reader.pair_field(p)
                                })?,
                            },
                            Neighbour::NoPrediction,
                        ]]
                    } else {
                        motion.decode_macroblock_mbaff_with_direct(
                            address,
                            slice_id,
                            &parts,
                            |p| reader.pair_field(p),
                            context,
                        )?
                    };
                    let weights = super::super::super::avc_inter_prediction::mbaff_weights(
                        address, field, header, pps, &vectors, context,
                    )?;
                    for (part, vector) in parts.iter().zip(&vectors) {
                        let mut ids = [None; 2];
                        for list in 0..2 {
                            if let Neighbour::Inter { reference, vector } = vector[list] {
                                let picture = references[slice][list]
                                    .get(reference as usize / if field { 2 } else { 1 })
                                    .and_then(|p| *p)
                                    .ok_or_else(|| {
                                        invalid("444 MBAFF selected reference missing")
                                    })?;
                                let identity = unique
                                    .iter()
                                    .position(|p| std::ptr::eq(*p, picture))
                                    .ok_or_else(|| {
                                        invalid("444 MBAFF reference identity missing")
                                    })? as u64;
                                ids[list] = Some(MotionReference {
                                    picture: identity * 3
                                        + if field {
                                            1 + ((address % 2) ^ (reference as usize % 2)) as u64
                                        } else {
                                            0
                                        },
                                    vector,
                                });
                            }
                        }
                        for y in part.origin[1] as usize / 4
                            ..(part.origin[1] + part.size[1]) as usize / 4
                        {
                            for x in part.origin[0] as usize / 4
                                ..(part.origin[0] + part.size[0]) as usize / 4
                            {
                                for cells in &mut blocks {
                                    cells[y * 4 + x].motion = ids;
                                }
                            }
                        }
                    }
                    for p in 0..3 {
                        let component_weights = weights.as_ref().map(|weights| {
                            weights
                                .iter()
                                .map(|w| [[w[0][p]; 3], [w[1][p]; 3]])
                                .collect::<Vec<_>>()
                        });
                        let prediction=super::super::super::avc_inter_prediction::predict_macroblock_mbaff_components(address,[w/16,h/16],field,[mono[p].bit_depth_luma;2],&parts,&vectors,[&refs[p*2],&refs[p*2+1]],component_weights.as_deref())?;
                        let coefficients = coefficients.as_ref().map(|c| c[p].as_ref());
                        if let Some(c) = coefficients {
                            for i in 0..16 {
                                blocks[p][i].nonzero_luma = if eight {
                                    c.luma8[(i / 4 / 2) * 2 + i % 4 / 2].iter().any(|&v| v != 0)
                                } else {
                                    c.luma_counts[i] != 0
                                };
                            }
                        }
                        if pps.constrained_intra_pred {
                            // Inter samples remain ineligible for subsequent intra prediction.
                            reconstruct_inter_macroblock(
                                &mut pictures[p],
                                address,
                                field,
                                prediction,
                                coefficients,
                                eight,
                                [primes[p]; 3],
                                sps.transform_bypass && primes[0] == 0,
                                &scaling[p],
                            )?;
                        } else {
                            reconstruct_inter_macroblock_ready(
                                &mut pictures[p],
                                address,
                                field,
                                prediction,
                                coefficients,
                                eight,
                                [primes[p]; 3],
                                sps.transform_bypass && primes[0] == 0,
                                &scaling[p],
                                &mut ready[p],
                            )?;
                        }
                    }
                }
            }
            for p in 0..3 {
                edges[p].push(MbaffBlockEdges {
                    field,
                    edges: DecodedBlockEdges {
                        blocks: blocks[p],
                        qp: [actual[p]; 3],
                        slice_id,
                        disable_filter: header.disable_deblocking_filter_idc as u8,
                        offsets: [header.alpha_offset, header.beta_offset],
                        transform8: eight,
                    },
                });
            }
            seen += 1;
        }
        if seen != end || seen % 2 != 0 {
            return Err(invalid("incomplete 444 MBAFF slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete 444 MBAFF picture"));
    }
    if filtered {
        for p in 0..3 {
            mbaff_inter_plane_subsampled(
                &mut pictures[p].y,
                w,
                h,
                mono[p].bit_depth_luma,
                0,
                [1, 1],
                &edges[p],
            )?;
        }
    }
    let mut planes = pictures.into_iter();
    Ok((
        IntraPicture {
            coded_width: w,
            coded_height: h,
            crop: sps.crop.map(|v| v as usize),
            bit_depth: sps.bit_depth_luma,
            chroma_bit_depth: sps.bit_depth_chroma,
            y: planes.next().unwrap().y,
            cb: planes.next().unwrap().y,
            cr: planes.next().unwrap().y,
        },
        motion,
    ))
}
