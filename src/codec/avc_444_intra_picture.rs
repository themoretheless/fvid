//! Nonseparate full-resolution CAVLC/CABAC intra: shared macroblock syntax, three residual contexts.
use super::super::{
    avc::{Pps, SliceGroups, Sps},
    avc_cabac_macroblock::IntraCabacReader,
    avc_macroblock::{IntraCavlcReader, IntraLuma, IntraMacroblock},
    avc_scaling::ScalingMatrices,
    avc_slice::{SliceHeader, SliceType},
    bits::BitReader,
};
use super::{IntraPicture, decoded_storage, reconstruct_macroblock};
use crate::{Result, invalid, unsupported};
enum SliceReader<'a> {
    Cavlc {
        readers: [IntraCavlcReader<'a>; 3],
        bits: BitReader<'a>,
    },
    Cabac(IntraCabacReader<'a>),
}
impl SliceReader<'_> {
    fn field_decoding(&self) -> bool {
        match self {
            Self::Cabac(reader) => reader.field_decoding(),
            Self::Cavlc { readers, .. } => readers[0].field_decoding(),
        }
    }
    fn read(&mut self) -> Result<Option<[IntraMacroblock; 3]>> {
        match self {
            Self::Cabac(reader) => reader.read_444_macroblock(),
            Self::Cavlc { readers, bits } => {
                if !bits.more_rbsp_data() {
                    bits.finish_rbsp()?;
                    return Ok(None);
                }
                readers[0].resume_444(bits);
                let y = readers[0]
                    .read_macroblock()?
                    .ok_or_else(|| invalid("missing AVC 4:4:4 macroblock"))?;
                let advance = readers[0]
                    .bit_position()
                    .checked_sub(bits.position())
                    .ok_or_else(|| invalid("AVC 4:4:4 entropy cursor moved backwards"))?;
                bits.skip(advance)?;
                if readers[0].pair_field(y.address as usize / 2).is_some() {
                    let field = readers[0].field_decoding();
                    readers[1].record_pair_mode(y.address as usize, field)?;
                    readers[2].record_pair_mode(y.address as usize, field)?;
                }
                let cb = readers[1].read_444_component(bits, &y)?;
                let cr = readers[2].read_444_component(bits, &y)?;
                Ok(Some([y, cb, cr]))
            }
        }
    }
}
pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    let field = headers
        .first()
        .ok_or_else(|| invalid("missing AVC 4:4:4 intra slices"))?
        .field_pic;
    if sps.chroma_format != 3
        || sps.separate_colour_plane
        || (sps.frame_mbs_only && field)
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || headers.iter().any(|h| {
            h.field_pic != field
                || h.slice_type != SliceType::I
                || h.disable_deblocking_filter_idc > 2
        })
    {
        return Err(unsupported(
            "AVC 4:4:4 intra requires I frame or field slices",
        ));
    }
    let mbaff = sps.mb_adaptive_frame_field && !field;
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize / if field { 2 } else { 1 });
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("AVC 4:4:4 picture overflow"))?;
    let count = pixels / 256;
    // Three readers (40 bytes/MB each), readiness grids, cloned macroblocks,
    // resolved matrices, component QPs, transform flags, loop-filter edge grid
    // and temporary header state. PCM probe was freed first.
    let storage = decoded_storage(sps, w, h)?;
    let base = storage
        .checked_mul(if mbaff { 2 } else { 1 })
        .ok_or_else(|| invalid("AVC 4:4:4 view overflow"))?;
    let needed = base
        .checked_add(
            count
                .checked_mul(512)
                .ok_or_else(|| invalid("AVC 4:4:4 context overflow"))?,
        )
        .and_then(|n| n.checked_add(32768))
        .ok_or_else(|| invalid("AVC 4:4:4 storage overflow"))?;
    if needed > budget {
        return Err(invalid("AVC 4:4:4 intra exceeds memory budget"));
    }
    let projected: [Sps; 3] = std::array::from_fn(|p| {
        let mut c = sps.clone();
        c.chroma_format = 0;
        c.separate_colour_plane = false;
        c.bit_depth_luma = if p == 0 {
            sps.bit_depth_luma
        } else {
            sps.bit_depth_chroma
        };
        c.bit_depth_chroma = c.bit_depth_luma;
        c.scaling_lists = None;
        c
    });
    let scaling = [
        ScalingMatrices::for_component(sps, pps, 0)?,
        ScalingMatrices::for_component(sps, pps, 1)?,
        ScalingMatrices::for_component(sps, pps, 2)?,
    ];
    let allocate = |n| -> Result<Vec<u16>> {
        let mut v = Vec::new();
        v.try_reserve_exact(n)
            .map_err(|_| invalid("cannot allocate AVC 4:4:4 plane"))?;
        v.resize(n, 0);
        Ok(v)
    };
    let mut pictures = Vec::new();
    for c in &projected {
        pictures.push(IntraPicture {
            coded_width: w,
            coded_height: h,
            crop: [0; 4],
            bit_depth: c.bit_depth_luma,
            chroma_bit_depth: c.bit_depth_luma,
            y: allocate(pixels)?,
            cb: Vec::new(),
            cr: Vec::new(),
        });
    }
    let capacities = pictures
        .iter()
        .try_fold(0usize, |n, p| n.checked_add(p.y.capacity() * 2))
        .ok_or_else(|| invalid("AVC 4:4:4 capacity overflow"))?;
    if capacities > budget - (needed - decoded_storage(sps, w, h)?) {
        return Err(invalid("AVC 4:4:4 capacities exceed memory budget"));
    }
    let mut ready = [
        crate::buffer(pixels / 16)?,
        crate::buffer(pixels / 16)?,
        crate::buffer(pixels / 16)?,
    ];
    let mut mbaff_ready = Vec::new();
    let mut mbaff_edges = Vec::new();
    if mbaff {
        for _ in 0..3 {
            mbaff_ready.push(super::super::avc_mbaff::Readiness420::new(
                w / 16,
                h / 16,
                true,
                count * 7,
            )?);
        }
        mbaff_edges
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC 4:4:4 MBAFF edges"))?;
        mbaff_edges.resize(
            count,
            super::super::avc_deblock::MbaffIntraBlock {
                qp: [0; 3],
                field: false,
                transform8: false,
                slice: 0,
                disable: 1,
                offsets: [0; 2],
            },
        );
    }
    let mut qps = [Vec::new(), Vec::new(), Vec::new()];
    for grid in &mut qps {
        grid.try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC 4:4:4 filter QPs"))?;
        grid.resize(count, 0i32);
    }
    let mut eight = crate::buffer(count)?;
    let mut seen = 0;
    for (slice, &header) in headers.iter().enumerate() {
        if header.first_mb as usize * if mbaff { 2 } else { 1 } != seen {
            return Err(invalid("AVC 4:4:4 slice coverage gap or overlap"));
        }
        for grid in &mut ready {
            grid.fill(0);
        }
        for grid in &mut mbaff_ready {
            grid.reset_slice();
        }
        let cavlc = |p: usize| {
            if mbaff {
                IntraCavlcReader::new_mbaff(header, &projected[p], pps, count)
            } else {
                IntraCavlcReader::new(header, &projected[p], pps, count)
            }
        };
        let mut reader = if pps.cabac {
            SliceReader::Cabac(IntraCabacReader::new_444(
                header,
                &projected[0],
                pps,
                count,
                sps.bit_depth_chroma,
            )?)
        } else {
            let mut bits = BitReader::new(&header.rbsp);
            bits.skip(header.entropy_bit_offset)?;
            SliceReader::Cavlc {
                readers: [cavlc(0)?, cavlc(1)?, cavlc(2)?],
                bits,
            }
        };
        while let Some([y, cb, cr]) = reader.read()? {
            if seen >= count {
                return Err(invalid("AVC 4:4:4 macroblock exceeds picture"));
            }
            if y.address as usize != seen {
                return Err(invalid("AVC 4:4:4 macroblock coverage gap"));
            }
            let pair_field = reader.field_decoding();
            if mbaff {
                mbaff_edges[seen] = super::super::avc_deblock::MbaffIntraBlock {
                    qp: [0; 3],
                    field: pair_field,
                    transform8: matches!(y.luma, IntraLuma::Blocks8 { .. }),
                    slice,
                    disable: header.disable_deblocking_filter_idc as u8,
                    offsets: [header.alpha_offset, header.beta_offset],
                };
            }
            eight[seen] = u8::from(matches!(y.luma, IntraLuma::Blocks8 { .. }));
            let yprime = y.qp + 6 * (i32::from(sps.bit_depth_luma) - 8);
            let bypass = sps.transform_bypass && yprime == 0;
            for (p, mut mb) in [y, cb, cr].into_iter().enumerate() {
                let mut component = projected[p].clone();
                component.transform_bypass = bypass;
                let depth_offset = 6 * (i32::from(component.bit_depth_luma) - 8);
                let actual_qp = if p == 0 {
                    mb.qp
                } else {
                    let offset = if p == 1 {
                        pps.chroma_qp_offset
                    } else {
                        pps.second_chroma_qp_offset
                    };
                    i32::from(super::chroma_qp(mb.qp, offset, sps.bit_depth_chroma)) - depth_offset
                };
                // Deblocking uses the actual component QP even when lossless
                // reconstruction projects the shared QP'Y bypass flag onto C.
                qps[p][seen] = actual_qp;
                mb.qp = if bypass { -depth_offset } else { actual_qp };
                if mbaff {
                    super::super::avc_mbaff_picture::reconstruct_intra_macroblock(
                        &mut pictures[p],
                        mb,
                        pair_field,
                        &component,
                        pps,
                        &scaling[p],
                        &mut mbaff_ready[p],
                    )?;
                } else {
                    reconstruct_macroblock(
                        &mut pictures[p],
                        &mb,
                        &component,
                        pps,
                        &scaling[p],
                        &mut ready[p],
                    )?;
                }
            }
            seen += 1;
        }
        if mbaff && seen % 2 != 0 {
            return Err(invalid("AVC 4:4:4 incomplete MBAFF slice pair"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete AVC 4:4:4 intra picture"));
    }
    // H.264 8.7: full-resolution Cb/Cr use luma-style edge filtering on
    // the shared 4x4/8x8 grid, with their own mapped QPs and sample depths.
    // Run after reconstruction so prediction always sees unfiltered neighbours.
    for p in 0..3 {
        if mbaff {
            for (edge, &qp) in mbaff_edges.iter_mut().zip(&qps[p]) {
                edge.qp[0] = qp;
            }
            super::super::avc_deblock::mbaff_intra_plane_subsampled(
                &mut pictures[p].y,
                w,
                h,
                projected[p].bit_depth_luma,
                0,
                [1, 1],
                &mbaff_edges,
            )?;
            continue;
        }
        super::super::avc_deblock::intra_plane_owned(
            &mut pictures[p].y,
            w,
            h,
            &qps[p],
            projected[p].bit_depth_luma,
            headers,
            false,
            &eight,
            None,
        )?;
    }
    let mut pictures = pictures.into_iter();
    let y = pictures.next().unwrap().y;
    let cb = pictures.next().unwrap().y;
    let cr = pictures.next().unwrap().y;
    Ok(IntraPicture {
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
        y,
        cb,
        cr,
    })
}
