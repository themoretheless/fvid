//! Nonseparate full-resolution inter: shared motion, independent component residuals.
use super::super::{
    avc_inter_slice::colour444::{Macroblock, Reader as CavlcReader},
    avc_macroblock::IntraLuma,
};
use super::*;
use crate::unsupported;

#[path = "avc_444_field_inter_picture.rs"]
mod field;
struct FieldContext<'a> {
    direct: Option<&'a [super::super::avc_direct::FieldDirectPrediction<'a>]>,
    implicit: Option<&'a super::super::avc_field_picture::ImplicitFieldWeights<'a>>,
    identities: &'a [[Vec<u64>; 2]],
}
pub(super) fn decode_fields(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[(&super::super::avc_field_picture::PcmField, u64)]; 2]],
    implicit: Option<&super::super::avc_field_picture::ImplicitFieldWeights<'_>>,
    direct: Option<&[super::super::avc_direct::FieldDirectPrediction<'_>]>,
    budget: usize,
    retain_motion: bool,
) -> Result<(
    super::super::avc_field_picture::PcmField,
    Option<MotionField>,
)> {
    field::decode(
        headers,
        sps,
        pps,
        references,
        implicit,
        direct,
        budget,
        retain_motion,
    )
}

#[path = "avc_444_mbaff_inter_picture.rs"]
mod mbaff;
pub(super) fn decode_mbaff(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[Option<&IntraPicture>]; 2]],
    direct: Option<&[Option<&super::super::avc_direct::MbaffDirectPrediction<'_>>]>,
    budget: usize,
    filtered: bool,
) -> Result<(IntraPicture, MotionField)> {
    mbaff::decode(headers, sps, pps, references, direct, budget, filtered)
}

enum Reader<'a> {
    Cavlc(CavlcReader<'a>),
    Cabac(super::super::avc_cabac_slice::InterCabacSlice<'a>),
}
impl Reader<'_> {
    fn pair_field(&self, pair: usize) -> Option<bool> {
        match self {
            Self::Cavlc(r) => r.pair_field(pair),
            Self::Cabac(r) => r.pair_field(pair),
        }
    }
    fn field_decoding(&self) -> bool {
        match self {
            Self::Cavlc(r) => r.field_decoding(),
            Self::Cabac(r) => r.field_decoding(),
        }
    }
    fn read(&mut self) -> Result<Option<Macroblock>> {
        match self {
            Self::Cavlc(reader) => reader.read(),
            Self::Cabac(reader) => reader.read_444_macroblock(),
        }
    }
}

fn weights(
    header: &SliceHeader,
    pps: &Pps,
    vectors: &[[Neighbour; 2]],
    direct: Option<&super::super::avc_direct::DirectPrediction<'_>>,
    field_orders: Option<(i32, [&[(i32, bool)]; 2])>,
) -> Result<Option<Vec<[[ComponentWeight; 3]; 2]>>> {
    let explicit = if header.slice_type == SliceType::B {
        pps.weighted_bipred == 1
    } else {
        pps.weighted_pred
    };
    let implicit = header.slice_type == SliceType::B && pps.weighted_bipred == 2;
    if !explicit && !implicit {
        return Ok(None);
    }
    let mut result = Vec::with_capacity(vectors.len());
    for vector in vectors {
        let mut out = [[ComponentWeight::default(); 3]; 2];
        if explicit {
            let table = header
                .weights
                .as_ref()
                .ok_or_else(|| invalid("missing 444 explicit weights"))?;
            for list in 0..2 {
                if let Neighbour::Inter { reference, .. } = vector[list] {
                    let entry = [&table.l0, &table.l1][list]
                        .get(reference as usize)
                        .ok_or_else(|| invalid("missing 444 reference weight"))?;
                    out[list][0] = ComponentWeight {
                        weight: entry.luma.0,
                        offset: entry.luma.1,
                        denominator: table.luma_denom,
                    };
                    for p in 1..3 {
                        out[list][p] = ComponentWeight {
                            weight: entry.chroma[p - 1].0,
                            offset: entry.chroma[p - 1].1,
                            denominator: table.chroma_denom,
                        };
                    }
                }
            }
        } else if let [
            Neighbour::Inter { reference: a, .. },
            Neighbour::Inter { reference: b, .. },
        ] = *vector
        {
            let values = if let Some((poc, lists)) = field_orders {
                let a = lists[0]
                    .get(a as usize)
                    .ok_or_else(|| invalid("444 field L0 weight reference missing"))?;
                let b = lists[1]
                    .get(b as usize)
                    .ok_or_else(|| invalid("444 field L1 weight reference missing"))?;
                super::super::avc_mv::implicit_weights(
                    poc.into(),
                    a.0.into(),
                    b.0.into(),
                    a.1 || b.1,
                )
            } else {
                let context =
                    direct.ok_or_else(|| invalid("missing 444 implicit-weight metadata"))?;
                let a = context
                    .list0
                    .get(a as usize)
                    .ok_or_else(|| invalid("444 L0 weight reference missing"))?;
                let b = context
                    .list1
                    .get(b as usize)
                    .ok_or_else(|| invalid("444 L1 weight reference missing"))?;
                let values = super::super::avc_mv::implicit_weights(
                    context.current_poc.into(),
                    a.poc.into(),
                    b.poc.into(),
                    a.long_term_index.is_some() || b.long_term_index.is_some(),
                );
                values
            };
            for list in 0..2 {
                out[list] = [ComponentWeight {
                    weight: values[list],
                    offset: 0,
                    denominator: 5,
                }; 3];
            }
        }
        result.push(out);
    }
    Ok(Some(result))
}
pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[Option<&IntraPicture>]; 2]],
    direct: &[Option<&super::super::avc_direct::DirectPrediction<'_>>],
    budget: usize,
) -> Result<(IntraPicture, MotionField)> {
    decode_impl(headers, sps, pps, references, direct, budget, None)
}
fn decode_impl(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    references: &[[&[Option<&IntraPicture>]; 2]],
    direct: &[Option<&super::super::avc_direct::DirectPrediction<'_>>],
    budget: usize,
    field_context: Option<&FieldContext<'_>>,
) -> Result<(IntraPicture, MotionField)> {
    let field = field_context.is_some();
    if sps.chroma_format != 3
        || sps.separate_colour_plane
        || (field && sps.frame_mbs_only)
        || (!field && sps.mb_adaptive_frame_field)
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || headers.is_empty()
        || headers.len() != references.len()
        || headers.len() != direct.len()
        || headers.iter().any(|h| {
            h.field_pic != field
                || !matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B)
                || h.redundant_pic_cnt != 0
                || h.disable_deblocking_filter_idc > 2
        })
    {
        return Err(unsupported(
            "nonseparate 444 inter requires admitted I/P/B frame or field slices",
        ));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize / if field { 2 } else { 1 });
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("444 inter geometry overflow"))?;
    let count = pixels / 256;
    // References borrow decoded pixels, but their full-resolution plane views and
    // pointer lists are live together. Reserve the largest slice's metadata too.
    let view_count = references
        .iter()
        .try_fold(0usize, |maximum, lists| {
            lists[0]
                .len()
                .checked_add(lists[1].len())
                .and_then(|n| n.checked_mul(3))
                .map(|n| maximum.max(n))
        })
        .ok_or_else(|| invalid("444 reference view count overflow"))?;
    let view_bytes = view_count
        .checked_mul(
            std::mem::size_of::<Reference420<'_>>() + std::mem::size_of::<&Reference420<'_>>(),
        )
        .ok_or_else(|| invalid("444 reference view storage overflow"))?;
    let required = super::super::avc_picture::decoded_storage(sps, w, h)?
        .checked_add(
            count
                .checked_mul(4096)
                .ok_or_else(|| invalid("444 inter contexts overflow"))?,
        )
        .and_then(|n| n.checked_add(32768))
        .and_then(|n| n.checked_add(view_bytes))
        .ok_or_else(|| invalid("444 inter storage overflow"))?;
    if count == 0 || count > 65536 || headers.len() > count || required > budget {
        return Err(invalid("444 inter exceeds memory budget"));
    }
    let projected: [Sps; 3] = std::array::from_fn(|p| {
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
        super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 0)?,
        super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 1)?,
        super::super::avc_scaling::ScalingMatrices::for_component(sps, pps, 2)?,
    ];
    let mut pictures = Vec::new();
    let mut ready = Vec::new();
    let mut edges: [Vec<DecodedBlockEdges>; 3] = std::array::from_fn(|_| Vec::new());
    for mono in &projected {
        let mut plane = Vec::new();
        plane
            .try_reserve_exact(pixels)
            .map_err(|_| invalid("cannot allocate 444 inter plane"))?;
        plane.resize(pixels, 0u16);
        pictures.push(IntraPicture {
            coded_width: w,
            coded_height: h,
            crop: [0; 4],
            bit_depth: mono.bit_depth_luma,
            chroma_bit_depth: mono.bit_depth_luma,
            y: plane,
            cb: Vec::new(),
            cr: Vec::new(),
        });
        ready.push(crate::buffer(count * 16)?);
    }
    for grid in &mut edges {
        grid.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate 444 inter edges"))?;
    }
    let mut motion = if field {
        MotionField::new_field(w, h, count * 4096)?
    } else {
        MotionField::new(w, h, count * 4096)?
    };
    let mut seen = 0;
    for (slice, header) in headers.iter().enumerate() {
        if header.first_mb as usize != seen {
            return Err(invalid("444 inter slice coverage gap or overlap"));
        }
        let end = headers
            .get(slice + 1)
            .map_or(count, |h| h.first_mb as usize);
        if end <= seen || end > count {
            return Err(invalid("invalid 444 inter slice range"));
        }
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
                return Err(invalid("444 inter reference count mismatch"));
            }
        }
        if is_b && !field && direct[slice].is_none() {
            return Err(invalid("missing 444 B direct metadata"));
        }
        if let Some(context) = direct[slice] {
            if context.list0.len() != lengths[0]
                || context.list1.len() != lengths[1]
                || context.inference8 != sps.direct_8x8_inference
                || context.colocated.is_some_and(|m| m.dimensions() != [w, h])
            {
                return Err(invalid("invalid 444 direct reference geometry"));
            }
        }
        let mut views: [Vec<Reference420<'_>>; 6] = std::array::from_fn(|_| Vec::new());
        for p in 0..3 {
            for list in 0..2 {
                views[p * 2 + list]
                    .try_reserve_exact(lengths[list])
                    .map_err(|_| invalid("cannot allocate 444 reference views"))?;
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
                            return Err(invalid("444 inter reference format mismatch"));
                        }
                        Reference420::from_decoded_monochrome(
                            [&picture.y, &picture.cb, &picture.cr][p],
                            w,
                            h,
                            [projected[p].bit_depth_luma; 2],
                        )?
                    } else {
                        Reference420::unavailable(projected[p].bit_depth_luma)?
                    };
                    views[p * 2 + list].push(view);
                }
            }
        }
        let refs: [Vec<&Reference420<'_>>; 6] = std::array::from_fn(|i| views[i].iter().collect());
        let view_capacity = views
            .iter()
            .map(|v| v.capacity() * std::mem::size_of::<Reference420<'_>>())
            .sum::<usize>()
            + refs
                .iter()
                .map(|v| v.capacity() * std::mem::size_of::<&Reference420<'_>>())
                .sum::<usize>();
        if view_capacity > budget - (required - view_bytes) {
            return Err(invalid("444 reference capacities exceed memory budget"));
        }
        for grid in &mut ready {
            grid.fill(0)
        }
        let mut reader = if pps.cabac {
            Reader::Cabac(super::super::avc_cabac_slice::InterCabacSlice::new_444(
                header,
                &projected[0],
                pps,
                sps.bit_depth_chroma,
                count * 4096,
            )?)
        } else {
            Reader::Cavlc(CavlcReader::new(header, &projected, pps, count)?)
        };
        while let Some(mb) = reader.read()? {
            let address = match &mb {
                Macroblock::Intra(m) => m[0].address as usize,
                Macroblock::Skip { address, .. } | Macroblock::Coded { address, .. } => *address,
            };
            if address != seen || seen >= end {
                return Err(invalid("444 inter macroblock exceeds slice range"));
            }
            let origin = [address % (w / 16) * 16, address / (w / 16) * 16];
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
                (qp + 6 * (i32::from(sps.bit_depth_luma) - 8)) as u8,
                chroma_qp(qp, pps.chroma_qp_offset, sps.bit_depth_chroma),
                chroma_qp(qp, pps.second_chroma_qp_offset, sps.bit_depth_chroma),
            ];
            let actual: [i32; 3] = std::array::from_fn(|p| {
                i32::from(primes[p]) - 6 * (i32::from(projected[p].bit_depth_luma) - 8)
            });
            let empty = BlockEdge {
                intra,
                switching_slice: false,
                nonzero_luma: false,
                motion: [None; 2],
            };
            let mut blocks = [[empty; 16]; 3];
            match mb {
                Macroblock::Intra(components) => {
                    motion.store(origin, [16, 16], slice_id, [Neighbour::NoPrediction; 2])?;
                    let bypass = sps.transform_bypass && primes[0] == 0;
                    for (p, mut component) in components.into_iter().enumerate() {
                        let mut mono = projected[p].clone();
                        mono.transform_bypass = bypass;
                        component.qp = if bypass {
                            -6 * (i32::from(mono.bit_depth_luma) - 8)
                        } else {
                            actual[p]
                        };
                        super::super::avc_picture::reconstruct_macroblock(
                            &mut pictures[p],
                            &component,
                            &mono,
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
                                let super::super::avc_inter::MacroblockType::Inter {
                                    partitions,
                                    ..
                                } = super::super::avc_inter::macroblock_type(SliceType::B, 0)?
                                else {
                                    return Err(invalid("invalid B skip"));
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
                                vector: motion.decode_p_skip(origin, slice_id)?,
                            },
                            Neighbour::NoPrediction,
                        ]]
                    } else if let Some(fields) = field_context {
                        motion.decode_field_macroblock_with_direct(
                            origin,
                            slice_id,
                            &parts,
                            fields.direct.map(|contexts| &contexts[slice]),
                        )?
                    } else {
                        motion.decode_macroblock_with_direct(
                            origin,
                            slice_id,
                            &parts,
                            direct[slice],
                        )?
                    };
                    let field_orders = field_context
                        .and_then(|f| f.implicit.map(|i| (i.poc, i.references[slice])));
                    let weights = weights(header, pps, &vectors, direct[slice], field_orders)?;
                    for (part, vector) in parts.iter().zip(&vectors) {
                        let mut identity = [None; 2];
                        for list in 0..2 {
                            if let Neighbour::Inter { reference, vector } = vector[list] {
                                let picture = references[slice][list]
                                    .get(reference as usize)
                                    .and_then(|r| *r)
                                    .ok_or_else(|| {
                                        invalid("444 inter samples missing reference")
                                    })?;
                                identity[list] = Some(MotionReference {
                                    picture: field_context.map_or(
                                        picture as *const IntraPicture as usize as u64,
                                        |fields| fields.identities[slice][list][reference as usize],
                                    ),
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
                                for grid in &mut blocks {
                                    grid[y * 4 + x].motion = identity;
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
                        let prediction = predict_macroblock_components(
                            origin.map(|n| n as i32),
                            [projected[p].bit_depth_luma; 2],
                            &parts,
                            &vectors,
                            [&refs[p * 2], &refs[p * 2 + 1]],
                            component_weights.as_deref(),
                        )?;
                        let prediction = if let Some(coefficients) = &coefficients {
                            let c = &coefficients[p];
                            let residual = if eight {
                                InterLumaResidual::Blocks8(&c.luma8)
                            } else {
                                InterLumaResidual::Blocks4(&c.luma4)
                            };
                            for i in 0..16 {
                                blocks[p][i].nonzero_luma = if eight {
                                    c.luma8[(i / 4 / 2) * 2 + i % 4 / 2].iter().any(|&v| v != 0)
                                } else {
                                    c.luma_counts[i] != 0
                                };
                            }
                            if sps.transform_bypass && primes[0] == 0 {
                                prediction.reconstruct_inter_bypass(
                                    residual,
                                    &c.chroma_dc,
                                    &c.chroma_ac,
                                )?
                            } else {
                                prediction.reconstruct_inter(
                                    residual,
                                    &c.chroma_dc,
                                    &c.chroma_ac,
                                    [primes[p]; 3],
                                    &[scaling[p].four[3]; 3],
                                    &scaling[p].eight[1],
                                )?
                            }
                        } else {
                            prediction
                        };
                        for row in 0..16 {
                            let at = (origin[1] + row) * w + origin[0];
                            pictures[p].y[at..at + 16]
                                .copy_from_slice(&prediction.y[row * 16..row * 16 + 16]);
                        }
                        for y in 0..4 {
                            for x in 0..4 {
                                ready[p][(origin[1] / 4 + y) * (w / 4) + origin[0] / 4 + x] =
                                    u8::from(!pps.constrained_intra_pred);
                            }
                        }
                    }
                }
            }
            for p in 0..3 {
                edges[p].push(DecodedBlockEdges {
                    blocks: blocks[p],
                    qp: [actual[p]; 3],
                    slice_id,
                    disable_filter: header.disable_deblocking_filter_idc as u8,
                    offsets: [header.alpha_offset, header.beta_offset],
                    transform8: eight,
                });
            }
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete 444 inter slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete 444 inter picture"));
    }
    for p in 0..3 {
        let grid = if field {
            let mut grid = Vec::new();
            for row in 0..h / 16 {
                let width = w / 16;
                let current = &edges[p][row * width..(row + 1) * width];
                let previous = if row == 0 {
                    None
                } else {
                    Some(&edges[p][(row - 1) * width..row * width])
                };
                let [mut next, _, _] =
                    super::super::avc_boundary::row_edges_field(previous, current)?;
                grid.append(&mut next);
            }
            grid
        } else {
            super::super::avc_boundary::picture_edges(&edges[p], w / 16, 0)?
        };
        inter_plane(
            &mut pictures[p].y,
            w,
            h,
            projected[p].bit_depth_luma,
            false,
            &grid,
        )?;
    }
    let mut planes = pictures.into_iter();
    Ok((
        IntraPicture {
            coded_width: w,
            coded_height: h,
            crop: [
                sps.crop[0] as usize,
                sps.crop[1] as usize,
                sps.crop[2] as usize / if field { 2 } else { 1 },
                sps.crop[3] as usize / if field { 2 } else { 1 },
            ],
            bit_depth: sps.bit_depth_luma,
            chroma_bit_depth: sps.bit_depth_chroma,
            y: planes.next().unwrap().y,
            cb: planes.next().unwrap().y,
            cr: planes.next().unwrap().y,
        },
        motion,
    ))
}
