//! Owned PCM field reconstruction foundation. Stateful field playback is separate.
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
