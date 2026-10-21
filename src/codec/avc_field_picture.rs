//! Compact AVC fields with intra/inter prediction and complementary weaving.
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
/// caller-owned headers/RBSP and other pictures are excluded. Separate colour
/// planes use independent monochrome reconstruction. Nonseparate 444 fields
/// share intra modes and keep independent full-resolution residual contexts.
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
        || sps.chroma_format > 3
        || !(8..=14).contains(&sps.bit_depth_chroma)
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
    if sps.separate_colour_plane {
        return Ok(PcmField {
            picture: super::avc_picture::decode_intra_slices(headers, sps, pps, budget)?,
            bottom: first.bottom_field,
            frame_num: first.frame_num,
            pps_id: first.pps_id,
        });
    }
    let mut ordered = headers.to_vec();
    ordered.sort_by_key(|h| h.first_mb);
    if ordered.iter().all(|h| {
        h.slice_type == SliceType::I
            && h.disable_deblocking_filter_idc == 1
    }) {
        match decode_pcm_slices(&ordered, sps, pps, budget) {
            Ok(field) => return Ok(field),
            Err(crate::Error::Unsupported(_)) => {}
            Err(error) => return Err(error),
        }
    }
    let picture = if sps.chroma_format==3 {
        super::avc_picture::decode_full_resolution_intra_slices(&ordered,sps,pps,budget)?
    } else if sps.chroma_format==1 {
        super::avc_picture::decode_intra_slices(&ordered, sps, pps, budget)?
    } else {
        super::avc_picture::decode_component_intra_slices(&ordered,sps,pps,budget)?
    };
    Ok(PcmField {
        picture,
        bottom: first.bottom_field,
        frame_num: first.frame_num,
        pps_id: first.pps_id,
    })
}

fn copy_cabac_pcm<const C: usize>(
    reader: &mut super::avc_cabac::AvcCabac<'_>,
    picture: &mut IntraPicture, sps: &Sps, address: usize, sub: [usize; 2],
) -> Result<()> {
    let (y, chroma) = reader.pcm_raw::<C>(sps.bit_depth_luma, sps.bit_depth_chroma, sps.chroma_format != 0)?;
    let w = picture.coded_width;
    for ((dst, src), factors) in [&mut picture.y, &mut picture.cb, &mut picture.cr]
        .into_iter().zip([y.as_slice(), chroma[0].as_slice(), chroma[1].as_slice()])
        .zip([[1,1],sub,sub]) {
        if dst.is_empty() {continue;}
        let [sx,sy]=factors;let bw=16/sx;let bh=16/sy;let stride=w/sx;
        let x=address%(w/16)*bw;let y=address/(w/16)*bh;
        for row in 0..bh {dst[(y+row)*stride+x..(y+row)*stride+x+bw].copy_from_slice(&src[row*bw..(row+1)*bw]);}
    }
    Ok(())
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
        || sps.chroma_format > 3
        || sps.separate_colour_plane
        || !(8..=14).contains(&sps.bit_depth_chroma)
    {
        return Err(invalid("invalid AVC PCM field configuration"));
    }
    if !matches!(pps.slice_groups, SliceGroups::Single) {
        return Err(unsupported(
            "PCM field foundation requires a single slice group",
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
    let chroma = match sps.chroma_format {0=>0,1=>pixels/4,2=>pixels/2,3=>pixels,_=>unreachable!()};
    let sub = if sps.chroma_format==1 {[2,2]}else if sps.chroma_format==2 {[2,1]}else{[1,1]};
    if super::avc_picture::decoded_storage(sps,w,h)?
        .checked_add(count)
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
        chroma_bit_depth: sps.bit_depth_chroma,
        y: samples(pixels)?,
        cb: samples(chroma)?,
        cr: samples(chroma)?,
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
        if pps.cabac {
            let start=seen;let mw=w/16;
            let mut reader=super::avc_cabac::AvcCabac::new(&header.rbsp,header.entropy_bit_offset,SliceType::I,0,header.slice_qp)?;
            loop {
                if seen>=count {return Err(invalid("AVC CABAC PCM field macroblock exceeds picture"));}
                // Every preceding macroblock of this probe slice is PCM.
                // Neighbours in other slices are unavailable for mb_type.
                let left=seen%mw!=0 && seen>start;
                let top=seen>=mw && seen-mw>=start;
                if !reader.decision(3+usize::from(left)+usize::from(top))? || !reader.terminate()? {
                    return Err(unsupported("non-PCM AVC CABAC field reconstruction is not connected"));
                }
                match sps.chroma_format {
                    0|1=>copy_cabac_pcm::<64>(&mut reader,&mut picture,sps,seen,sub)?,
                    2=>copy_cabac_pcm::<128>(&mut reader,&mut picture,sps,seen,sub)?,
                    3=>copy_cabac_pcm::<256>(&mut reader,&mut picture,sps,seen,sub)?,
                    _=>unreachable!(),
                }
                covered[seen]=1;seen+=1;
                if reader.terminate()? {reader.finish_slice()?;break;}
            }
            continue;
        }
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset)?;
        let mut partition_b = header.partition_cursor(true)?;
        while bits.more_rbsp_data() {
            if seen >= count {
                return Err(invalid("AVC PCM field macroblock exceeds picture"));
            }
            if bits.unsigned_golomb()? != 25 {
                return Err(unsupported(
                    "non-PCM AVC field reconstruction is not connected",
                ));
            }
            // mb_type stays in A; alignment and all PCM samples are category3.
            if let Some(partition) = &mut partition_b {
                std::mem::swap(&mut bits, &mut partition.bits);
            }
            while bits.position() % 8 != 0 {
                if bits.bit()? {
                    return Err(invalid("nonzero AVC PCM alignment bit"));
                }
            }
            for (plane, bw, bh, width, depth) in [
                (&mut picture.y, 16, 16, w, sps.bit_depth_luma),
                (&mut picture.cb, 16/sub[0], 16/sub[1], w/sub[0], sps.bit_depth_chroma),
                (&mut picture.cr, 16/sub[0], 16/sub[1], w/sub[0], sps.bit_depth_chroma),
            ] {
                if plane.is_empty() {continue;}
                let bx = seen % (w / 16) * bw;
                let by = seen / (w / 16) * bh;
                for y in 0..bh {
                    for x in 0..bw {
                        plane[(by + y) * width + bx + x] = bits.read(depth)? as u16;
                    }
                }
            }
            if let Some(partition) = &mut partition_b {
                std::mem::swap(&mut bits, &mut partition.bits);
            }
            covered[seen] = 1;
            seen += 1;
        }
        bits.finish_rbsp()?;
        if let Some(partition) = &mut partition_b { partition.finish()?; }
        if let Some(mut partition) = header.partition_cursor(false)? { partition.finish()?; }
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
pub(super) fn picture_bytes(p: &IntraPicture) -> Result<usize> {
    p.chroma_format()?;
    p.y.len().checked_add(p.cb.len()).and_then(|n|n.checked_add(p.cr.len())).and_then(|n|n.checked_mul(2)).ok_or_else(||invalid("AVC field component storage overflow"))
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
    let format=picture.chroma_format()?;
    let [sx,sy]=match format {1=>[2,2],2=>[2,1],_=>[1,1]};
    let bytes = picture_bytes(picture)?/2;
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
            chroma_bit_depth: picture.chroma_bit_depth,
            y: split(&picture.y, w, h)?,
            cb: if format==0 {Vec::new()}else{split(&picture.cb, w/sx, h/sy)?},
            cr: if format==0 {Vec::new()}else{split(&picture.cr, w/sx, h/sy)?},
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
        || a.bit_depth != b.bit_depth || a.chroma_bit_depth != b.chroma_bit_depth
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
    let format=a.chroma_format()?;
    if format!=b.chroma_format()? {return Err(invalid("AVC field component formats differ"));}
    let [sx,sy]=match format {1=>[2,2],2=>[2,1],_=>[1,1]};
    let chroma=if format==0 {0}else{pixels/sx/sy};
    if picture_bytes(a)?.checked_mul(2).is_none_or(|n| n > budget) {
        return Err(invalid("AVC woven frame exceeds memory budget"));
    }
    let mut result = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: [a.crop[0], a.crop[1], crop_top, crop_bottom],
        bit_depth: a.bit_depth,
        chroma_bit_depth: a.chroma_bit_depth,
        y: samples(pixels)?,
        cb: samples(chroma)?,
        cr: samples(chroma)?,
    };
    for (dst, top, bottom, width, height) in [
        (&mut result.y, &a.y, &b.y, w, h / 2),
        (&mut result.cb, &a.cb, &b.cb, w/sx, h/sy/2),
        (&mut result.cr, &a.cr, &b.cr, w/sx, h/sy/2),
    ] {
        if dst.is_empty() && top.is_empty() && bottom.is_empty() {continue;}
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
    let (w, height) = sps.coded_dimensions();
    let pixels = (w as usize)
        .checked_mul(height as usize / 2)
        .ok_or_else(|| invalid("AVC P field size overflow"))?;
    if sps.separate_colour_plane {
        if headers.len() != 3
            || pps.cabac
            || !matches!(pps.slice_groups, SliceGroups::Single)
            || headers
                .iter()
                .any(|slice| slice.slice_type != SliceType::P || slice.weights.is_some())
        {
            return Err(unsupported(
                "general AVC P field reconstruction is not connected",
            ));
        }
        let mut seen = [false; 3];
        if !h.field_pic
            || h.pps_id != pps.id
            || pps.sps_id != sps.id
            || sps.chroma_format != 3
            || !(8..=14).contains(&sps.bit_depth_luma)
            || reference.picture.coded_width != w as usize
            || reference.picture.coded_height != height as usize / 2
            || reference.picture.bit_depth != sps.bit_depth_luma
            || reference.picture.chroma_bit_depth != sps.bit_depth_luma
            || reference.picture.chroma_format()? != 3
        {
            return Err(invalid("invalid AVC P field configuration"));
        }
        if reference.bottom != h.bottom_field {
            return Err(unsupported(
                "opposite-parity AVC P field interpolation is not connected",
            ));
        }
        if super::avc_picture::decoded_storage(sps, w as usize, height as usize / 2)? > budget {
            return Err(invalid("AVC P field exceeds memory budget"));
        }
        for slice in headers {
            let c = usize::from(slice.colour_plane_id);
            if c >= 3
                || seen[c]
                || !slice.field_pic
                || slice.first_mb != 0
                || slice.redundant_pic_cnt != 0
                || slice.pps_id != h.pps_id
                || slice.frame_num != h.frame_num
                || slice.bottom_field != h.bottom_field
                || slice.idr != h.idr
                || slice.idr_pic_id != h.idr_pic_id
                || slice.poc_lsb != h.poc_lsb
                || slice.delta_poc != h.delta_poc
                || (slice.nal_ref_idc == 0) != (h.nal_ref_idc == 0)
                || slice.memory_operations != h.memory_operations
            {
                return Err(invalid("invalid AVC P field configuration"));
            }
            seen[c] = true;
            let mut bits = BitReader::new(&slice.rbsp);
            bits.skip(slice.entropy_bit_offset)?;
            if bits.unsigned_golomb()? as usize != pixels / 256 {
                return Err(unsupported(
                    "coded AVC P field macroblocks are not connected",
                ));
            }
            bits.finish_rbsp()?;
        }
        let mut y = samples(pixels)?;
        let mut cb = samples(pixels)?;
        let mut cr = samples(pixels)?;
        y.copy_from_slice(&reference.picture.y);
        cb.copy_from_slice(&reference.picture.cb);
        cr.copy_from_slice(&reference.picture.cr);
        return Ok(PcmField {
            picture: IntraPicture {
                coded_width: w as usize,
                coded_height: height as usize / 2,
                crop: reference.picture.crop,
                bit_depth: sps.bit_depth_luma,
                chroma_bit_depth: sps.bit_depth_luma,
                y,
                cb,
                cr,
            },
            bottom: h.bottom_field,
            frame_num: h.frame_num,
            pps_id: h.pps_id,
        });
    }
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
    if !h.field_pic
        || h.first_mb != 0
        || h.redundant_pic_cnt != 0
        || h.pps_id != pps.id
        || pps.sps_id != sps.id
        || reference.picture.coded_width != w as usize
        || reference.picture.coded_height != height as usize / 2
        || reference.picture.bit_depth != sps.bit_depth_luma || reference.picture.chroma_bit_depth != sps.bit_depth_chroma
        || sps.chroma_format > 2
        || reference.picture.chroma_format()? != sps.chroma_format as u8
        || !(8..=14).contains(&sps.bit_depth_luma)
        || !(8..=14).contains(&sps.bit_depth_chroma)
    {
        return Err(invalid("invalid AVC P field configuration"));
    }
    if reference.bottom != h.bottom_field {
        return Err(unsupported(
            "opposite-parity AVC P field interpolation is not connected",
        ));
    }
    if super::avc_picture::decoded_storage(sps,w as usize,height as usize/2)? > budget {
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
    // A pure skip slice has no category3/4 data; reject surplus B/C streams.
    for intra in [true, false] {
        if let Some(mut partition) = h.partition_cursor(intra)? { partition.finish()?; }
    }
    let mut y = samples(pixels)?;
    let mut cb = samples(if sps.chroma_format==0 {0}else if sps.chroma_format==2 {pixels/2}else{pixels/4})?;
    let mut cr = samples(if sps.chroma_format==0 {0}else if sps.chroma_format==2 {pixels/2}else{pixels/4})?;
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
            chroma_bit_depth: sps.bit_depth_chroma,
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
        && (sps.chroma_format!=3 || sps.separate_colour_plane)
        && matches!(pps.slice_groups, SliceGroups::Single)
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
        || h.slice_type == SliceType::Si && pps.cabac
        || h.redundant_pic_cnt != 0
        || ((pps.cabac || sps.separate_colour_plane) && !matches!(pps.slice_groups, SliceGroups::Single))
        || h.disable_deblocking_filter_idc > 2
    {
        return Err(unsupported(
            "general AVC P field reconstruction is not connected",
        ));
    }
    let slice_limit = if sps.separate_colour_plane {
        count.saturating_mul(3)
    } else {
        count
    };
    if count == 0 || count > 65536 || headers.len() > slice_limit {
        return Err(invalid("invalid AVC P field slice count"));
    }
    for current in headers {
        let ordinary_mixed = matches!(current.slice_type, SliceType::I | SliceType::P | SliceType::B)
            && matches!(h.slice_type, SliceType::I | SliceType::P | SliceType::B);
        let switching_mixed = !pps.cabac
            && matches!(current.slice_type, SliceType::I | SliceType::Si | SliceType::P | SliceType::Sp | SliceType::B)
            && matches!(h.slice_type, SliceType::I | SliceType::Si | SliceType::P | SliceType::Sp | SliceType::B);
        if !current.field_pic
            // Each slice keeps its own entropy syntax and active reference lists.
            || current.slice_type != h.slice_type && !(ordinary_mixed || switching_mixed)
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
    if sps.chroma_format==3 && !sps.separate_colour_plane {
        return super::avc_inter_picture::decode_full_resolution_inter_fields(headers,sps,pps,references,implicit,direct,budget,retain_motion);
    }
    if sps.separate_colour_plane {
        if reference.picture.coded_width != w as usize
            || reference.picture.coded_height != height as usize / 2
            || reference.picture.bit_depth != sps.bit_depth_luma
            || reference.picture.chroma_bit_depth != sps.bit_depth_luma
            || sps.chroma_format != 3
            || reference.picture.chroma_format()? != 3
            || !(8..=14).contains(&sps.bit_depth_luma)
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
                || candidate.picture.chroma_format()? != 3
                || candidate.picture.bit_depth != reference.picture.bit_depth
                || candidate.picture.chroma_bit_depth != reference.picture.chroma_bit_depth
            {
                return Err(unsupported("AVC field reference geometry differs"));
            }
        }
        if super::avc_picture::decoded_storage(sps, w as usize, height as usize / 2)? > budget {
            return Err(invalid("AVC P field exceeds memory budget"));
        }
        let mut planes: [Vec<u16>; 3] = std::array::from_fn(|_| Vec::new());
        let mut retained_motion = None;
        let mut retained = 0usize;
        for (component, output) in planes.iter_mut().enumerate() {
            let mut indices_and_headers: Vec<(usize, &SliceHeader)> = headers
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, slice)| usize::from(slice.colour_plane_id) == component)
                .collect();
            if indices_and_headers.is_empty() || indices_and_headers.len() > count {
                return Err(invalid("invalid AVC P field slice count"));
            }
            indices_and_headers.sort_by_key(|(_, slice)| slice.first_mb);
            let plane_headers: Vec<&SliceHeader> =
                indices_and_headers.iter().map(|&(_, slice)| slice).collect();
            let (mono_sps, mono_pps) =
                super::avc_decoder::project_separate_parameters(sps, pps, component as u8)?;
            let mut mono_fields: Vec<[Vec<(PcmField, u64)>; 2]> =
                Vec::with_capacity(indices_and_headers.len());
            for &(idx, _) in &indices_and_headers {
                let mut slice_lists: [Vec<(PcmField, u64)>; 2] = [Vec::new(), Vec::new()];
                for list in 0..2 {
                    for &(cand, id) in references[idx][list] {
                        let src = match component {
                            0 => &cand.picture.y,
                            1 => &cand.picture.cb,
                            _ => &cand.picture.cr,
                        };
                        let mut y = samples(pixels)?;
                        y.copy_from_slice(src);
                        slice_lists[list].push((
                            PcmField {
                                picture: IntraPicture {
                                    coded_width: cand.picture.coded_width,
                                    coded_height: cand.picture.coded_height,
                                    crop: cand.picture.crop,
                                    bit_depth: sps.bit_depth_luma,
                                    chroma_bit_depth: sps.bit_depth_luma,
                                    y,
                                    cb: Vec::new(),
                                    cr: Vec::new(),
                                },
                                bottom: cand.bottom,
                                frame_num: cand.frame_num,
                                pps_id: cand.pps_id,
                            },
                            id,
                        ));
                    }
                }
                mono_fields.push(slice_lists);
            }
            let mono_ref_lists: Vec<[Vec<(&PcmField, u64)>; 2]> = mono_fields
                .iter()
                .map(|lists| {
                    [
                        lists[0].iter().map(|(f, id)| (f, *id)).collect(),
                        lists[1].iter().map(|(f, id)| (f, *id)).collect(),
                    ]
                })
                .collect();
            let mono_ref_views: Vec<[&[(&PcmField, u64)]; 2]> = mono_ref_lists
                .iter()
                .map(|lists| [lists[0].as_slice(), lists[1].as_slice()])
                .collect();
            let plane_implicit_refs: Vec<[&[(i32, bool)]; 2]> = if let Some(ctx) = implicit {
                indices_and_headers
                    .iter()
                    .map(|&(idx, _)| ctx.references[idx])
                    .collect()
            } else {
                Vec::new()
            };
            let plane_implicit = implicit.map(|ctx| ImplicitFieldWeights {
                poc: ctx.poc,
                references: &plane_implicit_refs,
            });
            let plane_direct: Option<Vec<super::avc_direct::FieldDirectPrediction<'_>>> =
                direct.map(|contexts| {
                    indices_and_headers
                        .iter()
                        .map(|&(idx, _)| super::avc_direct::FieldDirectPrediction {
                            spatial: contexts[idx].spatial,
                            inference8: contexts[idx].inference8,
                            current_poc: contexts[idx].current_poc,
                            current_bottom: contexts[idx].current_bottom,
                            lists: contexts[idx].lists,
                            colocated_is_frame: contexts[idx].colocated_is_frame,
                            colocated: contexts[idx].colocated,
                        })
                        .collect()
                });
            let available = budget
                .checked_sub(retained)
                .ok_or_else(|| invalid("AVC P field exceeds memory budget"))?;
            let (plane_field, motion) = decode_inter_field_impl(
                &plane_headers,
                &mono_sps,
                &mono_pps,
                &mono_ref_views,
                plane_implicit.as_ref(),
                plane_direct.as_deref(),
                available,
                retain_motion && component == 0,
            )?;
            if component == 0 {
                retained_motion = motion;
            }
            retained = plane_field
                .picture
                .y
                .capacity()
                .checked_mul(2)
                .and_then(|n| retained.checked_add(n))
                .filter(|&n| n <= budget)
                .ok_or_else(|| invalid("AVC P field exceeds memory budget"))?;
            *output = plane_field.picture.y;
        }
        let [y, cb, cr] = planes;
        return Ok((
            PcmField {
                picture: IntraPicture {
                    coded_width: w as usize,
                    coded_height: height as usize / 2,
                    crop: reference.picture.crop,
                    bit_depth: sps.bit_depth_luma,
                    chroma_bit_depth: sps.bit_depth_luma,
                    y,
                    cb,
                    cr,
                },
                bottom: h.bottom_field,
                frame_num: h.frame_num,
                pps_id: h.pps_id,
            },
            retained_motion,
        ));
    }
    if reference.picture.coded_width != w as usize
        || reference.picture.coded_height != height as usize / 2
        || reference.picture.bit_depth != sps.bit_depth_luma || reference.picture.chroma_bit_depth != sps.bit_depth_chroma
        || sps.chroma_format > 2
        || reference.picture.chroma_format()? != sps.chroma_format as u8
        || !(8..=14).contains(&sps.bit_depth_luma)
        || !(8..=14).contains(&sps.bit_depth_chroma)
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
            || candidate.picture.chroma_format()? != sps.chroma_format as u8
            || candidate.picture.bit_depth != reference.picture.bit_depth || candidate.picture.chroma_bit_depth != reference.picture.chroma_bit_depth
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
    let required = super::avc_picture::decoded_storage(sps,w as usize,height as usize/2)?
        .checked_add(0)
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
    let required=required.checked_add(if sps.chroma_format==2 {count.checked_mul(std::mem::size_of::<super::avc_inter_coefficients::InterChroma422>()+128).ok_or_else(||invalid("422 field scratch overflow"))?}else{0}).ok_or_else(||invalid("422 field budget overflow"))?;
    if required > budget {
        return Err(invalid("AVC P field exceeds memory budget"));
    }
    let mut output = PcmField {
        picture: IntraPicture {
            coded_width: w as usize,
            coded_height: height as usize / 2,
            crop: reference.picture.crop,
            bit_depth: sps.bit_depth_luma,
            chroma_bit_depth: sps.bit_depth_chroma,
            y: samples(pixels)?,
            cb: samples(if sps.chroma_format==0 {0}else if sps.chroma_format==2 {pixels/2}else{pixels/4})?,
            cr: samples(if sps.chroma_format==0 {0}else if sps.chroma_format==2 {pixels/2}else{pixels/4})?,
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
    let mut coefficients = if sps.chroma_format==2 {
        super::avc_coefficient_field::CoefficientField::new_422(w as usize/16,height as usize/32,count*128)?
    } else {super::avc_coefficient_field::CoefficientField::new(w as usize/16,height as usize/32,count*64)?};
    let scaling = super::avc_scaling::ScalingMatrices::new(sps, pps)?;
    let fmo = !matches!(pps.slice_groups, SliceGroups::Single);

    let mut edges = Vec::new();
    if filtered {
        edges
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC field edges"))?;
        edges.resize_with(count, || super::avc_boundary::DecodedBlockEdges {
            blocks: [super::avc_boundary::BlockEdge {
                intra: false,
                switching_slice: false,
                nonzero_luma: false,
                motion: [None; 2],
            }; 16],
            qp: [0; 3],
            offsets: [0; 2],
            transform8: false,
            slice_id: 0,
            disable_filter: 1,
        });
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
    let mut covered = vec![false; count];
    let mut seen = 0usize;
    for (slice_index, (reference_index, h)) in ordered.iter().enumerate() {
        let lists = references[*reference_index];
        let direct_context = direct.map(|contexts| &contexts[*reference_index]);
        let slice_id = slice_index as u32;
        let end = ordered
            .get(slice_index + 1)
            .map_or(count, |(_, h)| h.first_mb as usize);
        let mb_map = if fmo {
            let map = super::avc_slice_group_map::map_units(
                &pps.slice_groups,
                w as usize / 16,
                height as usize / 32,
                h.slice_group_change_cycle,
                count,
            )?;
            Some(super::avc_slice_group_map::macroblocks(
                &map,
                w as usize / 16,
                false,
                true,
                false,
                count,
            )?)
        } else {
            None
        };
        let mut qp = h.slice_qp;
        let mut intra = if pps.cabac {
            None
        } else if fmo {
            Some(super::avc_macroblock::IntraCavlcReader::new_context_fmo(
                h, sps, pps, count,
            )?)
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
        let mut partition_c = h.partition_cursor(false)?;
        let mut address = h.first_mb as usize;
        while (if fmo { address < count } else { address < end })
            && (pps.cabac || bits.more_rbsp_data())
        {
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
            if let Some(map) = &mb_map {
                let group = map[address];
                let remaining = (address..count)
                    .filter(|&a| map[a] == group && !covered[a])
                    .count();
                if skipped > remaining {
                    return Err(invalid("AVC P field skip exceeds picture"));
                }
            } else if skipped > end - address {
                return Err(invalid("AVC P field skip exceeds picture"));
            }
            for step in 0..if pps.cabac { 1 } else { skipped + 1 } {
                if fmo {
                    if step == skipped && (address >= count || (skipped > 0 && !bits.more_rbsp_data())) {
                        break;
                    }
                    if address >= count || covered[address] {
                        return Err(invalid("invalid AVC P field slice coverage"));
                    }
                } else if address == end {
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
                        if sps.chroma_format==2 {let (luma,chroma)=reader.counts_422(address)?;coefficients.store_422(address,slice_id,luma,chroma)?;}
                        else {let (luma,chroma)=reader.counts(address)?;coefficients.store(address,slice_id,luma,chroma)?;}
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
                        edges[address] = super::avc_boundary::DecodedBlockEdges {
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
                        };
                    }
                    covered[address] = true;
                    seen += 1;
                    address = if let Some(map) = &mb_map {
                        super::avc_slice_group_map::next_address(map, address)?
                    } else {
                        address + 1
                    };
                    continue;
                }
                let mut residual = None;
                let predictions = if step < skipped {
                    if sps.chroma_format==2 {coefficients.store_422(address,slice_id,[0;16],[[0;8];2])?;}else{coefficients.store(address,slice_id,[0;16],[[0;4];2])?;}
                    if let Some(reader)=&mut intra {if sps.chroma_format==2 {reader.record_inter_422(address,[0;16],[[0;8];2])?;}else{reader.record_inter(address,[0;16],[[0;4];2])?;}}
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
                            chroma_array_type: sps.chroma_format as u8,
                            transform8_enabled: pps.transform_8x8,
                            direct8_inference: sps.direct_8x8_inference,
                        };
                        let header =
                            super::avc_inter::read_inter_header_field(&mut bits, &syntax, true)?;
                        let coefficient_bits = match &mut partition_c {
                            Some(partition) => &mut partition.bits,
                            None => &mut bits,
                        };
                        let c=if sps.chroma_format==2 {
                            super::avc_inter_coefficients::read_inter_coefficients_422_field(coefficient_bits,header.residual.pattern,header.residual.transform8,coefficients.neighbours(address,slice_id)?,coefficients.neighbours_422(address,slice_id)?,true)?
                        }else{super::avc_inter_coefficients::read_inter_coefficients_field(coefficient_bits,header.residual.pattern,header.residual.transform8,coefficients.neighbours(address,slice_id)?,true)?};
                        (header, c)
                    };
                    if sps.chroma_format==2 {
                        let counts=c.chroma422.as_ref().map_or([[0;8];2],|wide|wide.counts);
                        coefficients.store_422(address,slice_id,c.luma_counts,counts)?;
                        if let Some(reader)=&mut intra {reader.record_inter_422(address,c.luma_counts,counts)?;}
                    }else{coefficients.store(address,slice_id,c.luma_counts,c.chroma_counts)?;if let Some(reader)=&mut intra {reader.record_inter(address,c.luma_counts,c.chroma_counts)?;}}
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
                        if component!=0 && sps.chroma_format==0 {continue;}
                        let depth = if component == 0 { sps.bit_depth_luma } else { sps.bit_depth_chroma };
                        let sub=if component==0 {[1,1]}else if sps.chroma_format==2 {[2,1]}else{[2,2]};
                        let [bw,bh]=[dimensions[0]/sub[0],dimensions[1]/sub[1]];
                        let xy = [
                            (origin[0] + offset[0]) / sub[0],
                            (origin[1] + offset[1]) / sub[1],
                        ];
                        let pw = w as usize / sub[0];
                        let ph = height as usize / 2 / sub[1];
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
                                    depth,
                                )?;
                                let mut mv = vector.map(i32::from);
                                if component != 0 && sps.chroma_format==1 {
                                    mv[1] += match (picture.bottom, h.bottom_field) {
                                        (false, true) => 2,
                                        (true, false) => -2,
                                        _ => 0,
                                    };
                                }
                                if component!=0 && sps.chroma_format==2 {mv[1]*=2;}
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
                                depth,
                            )?;
                        } else if h.weights.is_some() {
                            super::avc_motion::weight_block(
                                &mut prediction[used][..bw * bh],
                                weights[used].0,
                                weights[used].1,
                                denominator,
                                depth,
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
                    edges[address] = super::avc_boundary::DecodedBlockEdges {
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
                    };
                }
                // SP skips and coded zero-CBP blocks still quantize prediction.
                if h.slice_type == SliceType::Sp && residual.is_none() {
                    residual = Some((super::avc_inter_coefficients::InterCoefficients {
            chroma422:None,
                        luma4: [[0;16];16], luma8: [[0;64];4],
                        chroma_dc: [[0;4];2], chroma_ac: [[[0;16];4];2],
                        luma_counts: [0;16], chroma_counts: [[0;4];2],
                    }, false));
                }
                if let Some((c, eight)) = residual {
                    let mut prediction=if sps.chroma_format==2 {super::avc_compensation::Prediction420::empty_components_422([sps.bit_depth_luma,sps.bit_depth_chroma])}else{super::avc_compensation::Prediction420::empty_components([sps.bit_depth_luma,sps.bit_depth_chroma])};
                    let sub=prediction.chroma_subsampling();
                    for (component,(dst,src)) in prediction.planar_planes_mut().into_iter().zip([&output.picture.y[..],&output.picture.cb[..],&output.picture.cr[..]]).enumerate() {
                        if src.is_empty(){continue;}let [sx,sy]=if component==0 {[1,1]}else{sub};let bw=16/sx;let bh=16/sy;let stride=w as usize/sx;
                        for row in 0..bh {let at=(origin[1]/sy+row)*stride+origin[0]/sx;dst[row*bw..(row+1)*bw].copy_from_slice(&src[at..at+bw]);}
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
                    let reconstructed=if let Some(wide)=&c.chroma422 {reconstructed.reconstruct_chroma422(wide,qps,&[scaling.four[3],scaling.four[4],scaling.four[5]],sps.transform_bypass&&qps[0]==0)?}else{reconstructed};
                    let sub=reconstructed.chroma_subsampling();let [cb,cr]=reconstructed.chroma_planes();
                    for (component,(dst,src)) in [&mut output.picture.y,&mut output.picture.cb,&mut output.picture.cr].into_iter().zip([&reconstructed.y[..],cb,cr]).enumerate() {
                        if dst.is_empty(){continue;}let [sx,sy]=if component==0 {[1,1]}else{sub};let bw=16/sx;let bh=16/sy;let stride=w as usize/sx;
                        for row in 0..bh {let at=(origin[1]/sy+row)*stride+origin[0]/sx;dst[at..at+bw].copy_from_slice(&src[row*bw..(row+1)*bw]);}
                    }
                }
                if !pps.constrained_intra_pred {
                    for by in 0..4 {
                        for bx in 0..4 {
                            ready[(origin[1] / 4 + by) * (w as usize / 4) + origin[0] / 4 + bx] = 1;
                        }
                    }
                }
                covered[address] = true;
                seen += 1;
                address = if let Some(map) = &mb_map {
                    super::avc_slice_group_map::next_address(map, address)?
                } else {
                    address + 1
                };
            }
        }
        if !fmo && address != end {
            return Err(invalid("incomplete AVC P field slice"));
        }
        if !pps.cabac {
            bits.finish_rbsp()?;
            if let Some(partition) = &mut partition_c { partition.finish()?; }
            if let Some(reader) = &mut intra { reader.finish_partition()?; }
        } else if cabac.as_mut().unwrap().read_macroblock()?.is_some() {
            return Err(invalid("CABAC field slice exceeds coverage"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete AVC P field slice"));
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
            if plane.is_empty() {continue;}
            if i!=0 && sps.chroma_format==2 {
                super::avc_deblock::inter_plane_subsampled(plane,w as usize/2,height as usize/2,sps.bit_depth_chroma,[2,1],grid)?;continue;
            }
            let divisor = if i == 0 { 1 } else { 2 };
            super::avc_deblock::inter_plane(
                plane,
                w as usize / divisor,
                height as usize / 2 / divisor,
                if i == 0 { sps.bit_depth_luma } else { sps.bit_depth_chroma },
                i != 0,
                grid,
            )?;
        }
    }
    Ok((output, if retain_motion { Some(motion) } else { None }))
}

#[cfg(test)]
mod component_geometry_tests {
    use super::*;

    #[test]
    fn split_weave_preserves_all_component_formats_and_exact_storage_boundaries() {
        for format in 0..=3 {
            for (yd, cd) in [(8, 8), (10, 12), (14, 8)] {
                let chroma = match format {0 => 0, 1 => 256, 2 => 512, _ => 1024};
                let plane = |n: usize, depth: u8, bias: usize| -> Vec<u16> {
                    (0..n).map(|i| ((i * 179 + i / 7 * 31 + bias) & ((1usize << depth) - 1)) as u16).collect()
                };
                let source = IntraPicture {
                    coded_width: 32, coded_height: 32, crop: [0, 0, 2, 2],
                    bit_depth: yd, chroma_bit_depth: cd,
                    y: plane(1024, yd, 0), cb: plane(chroma, cd, 43), cr: plane(chroma, cd, 91),
                };
                let bytes = (1024 + 2 * chroma) * 2;
                assert_eq!(picture_bytes(&source).unwrap(), bytes);
                assert!(split_frame(&source, false, 0, 0, bytes / 2 - 1).is_err());
                let top = split_frame(&source, false, 0, 0, bytes / 2).unwrap();
                let bottom = split_frame(&source, true, 0, 0, bytes / 2).unwrap();
                assert_eq!(top.picture.chroma_format().unwrap(), format);
                assert!(weave_pair(&top, &bottom, bytes - 1).is_err());
                for (first, second) in [(&top, &bottom), (&bottom, &top)] {
                    let woven = weave_pair(first, second, bytes).unwrap();
                    assert_eq!(woven.y, source.y);
                    assert_eq!(woven.cb, source.cb);
                    assert_eq!(woven.cr, source.cr);
                    assert_eq!(woven.crop, source.crop);
                    assert_eq!(woven.component_depths(), source.component_depths());
                }
            }
        }
    }
}
