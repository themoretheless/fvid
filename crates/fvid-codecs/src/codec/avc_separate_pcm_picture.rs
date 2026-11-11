//! Assemble independently coded intra colour planes without external codecs.
use super::super::{
    avc::{Pps, SliceGroups, Sps},
    avc_slice::{SliceHeader, SliceType},
    bits::BitReader,
};
use super::{IntraPicture, decoded_storage};
use crate::{Result, invalid, unsupported};

pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    if !matches!(pps.slice_groups, SliceGroups::Single) {
        return Err(invalid(
            "AVC High 4:4:4 Predictive forbids FMO slice groups",
        ));
    }
    let field = headers.first().is_some_and(|h| h.field_pic);
    if headers
        .iter()
        .any(|h| h.field_pic != field || h.slice_type != SliceType::I)
    {
        return Err(unsupported(
            "AVC separate-plane intra requires I frame or field slices",
        ));
    }
    if field
        || sps.mb_adaptive_frame_field
        || pps.cabac
        || headers.iter().any(|h| h.disable_deblocking_filter_idc != 1)
    {
        return decode_components(headers, sps, pps, budget);
    }
    let (width, height) = sps.coded_dimensions();
    let (w, h) = (width as usize, height as usize);
    let count = w
        .checked_mul(h)
        .ok_or_else(|| invalid("AVC separate-plane geometry overflow"))?;
    if sps.chroma_format != 3
        || w == 0
        || h == 0
        || w % 16 != 0
        || h % 16 != 0
        || !(8..=14).contains(&sps.bit_depth_luma)
    {
        return Err(invalid("invalid AVC separate-plane PCM geometry"));
    }
    if decoded_storage(sps, w, h)? > budget {
        return Err(invalid("AVC separate-plane PCM exceeds memory budget"));
    }
    let allocate = || -> Result<Vec<u16>> {
        let mut plane = Vec::new();
        plane
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate AVC separate plane"))?;
        plane.resize(count, 0);
        Ok(plane)
    };
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|v| v as usize),
        bit_depth: sps.bit_depth_luma,
        chroma_bit_depth: sps.bit_depth_luma,
        y: allocate()?,
        cb: allocate()?,
        cr: allocate()?,
    };
    let capacities = picture
        .y
        .capacity()
        .checked_add(picture.cb.capacity())
        .and_then(|n| n.checked_add(picture.cr.capacity()))
        .and_then(|n| n.checked_mul(2));
    if capacities.is_none_or(|n| n > budget) {
        return Err(invalid(
            "AVC separate-plane capacities exceed memory budget",
        ));
    }
    let mut seen = [0usize; 3];
    let mut ordered = headers.to_vec();
    ordered.sort_by_key(|h| (h.colour_plane_id, h.first_mb));
    for header in ordered {
        let c = usize::from(header.colour_plane_id);
        if c >= 3 || header.first_mb as usize != seen[c] {
            return Err(invalid("AVC separate-plane slice coverage gap or overlap"));
        }
        let plane = match c {
            0 => &mut picture.y,
            1 => &mut picture.cb,
            _ => &mut picture.cr,
        };
        let mut bits = BitReader::new(&header.rbsp);
        bits.skip(header.entropy_bit_offset)?;
        while bits.more_rbsp_data() {
            if seen[c] >= count / 256 {
                return Err(invalid("AVC separate-plane macroblock exceeds picture"));
            }
            if bits.unsigned_golomb()? != 25 {
                drop(picture);
                return decode_components(headers, sps, pps, budget);
            }
            while bits.position() % 8 != 0 {
                if bits.bit()? {
                    return Err(invalid("nonzero AVC separate-plane PCM alignment bit"));
                }
            }
            let x = seen[c] % (w / 16) * 16;
            let y = seen[c] / (w / 16) * 16;
            for row in 0..16 {
                for col in 0..16 {
                    plane[(y + row) * w + x + col] = bits.read(sps.bit_depth_luma)? as u16;
                }
            }
            seen[c] += 1;
        }
        bits.finish_rbsp()?;
    }
    if seen != [count / 256; 3] {
        return Err(invalid("incomplete AVC separate-plane picture"));
    }
    Ok(picture)
}

fn decode_components(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    let mut mono = sps.clone();
    mono.chroma_format = 0;
    mono.separate_colour_plane = false;
    mono.bit_depth_chroma = mono.bit_depth_luma;
    mono.scaling_lists = None;
    let mut planes: [Vec<u16>; 3] = std::array::from_fn(|_| Vec::new());
    let mut retained = 0usize;
    for (component, output) in planes.iter_mut().enumerate() {
        let mut slices: Vec<_> = headers
            .iter()
            .copied()
            .filter(|h| usize::from(h.colour_plane_id) == component)
            .collect();
        slices.sort_by_key(|h| h.first_mb);
        if slices.is_empty() {
            return Err(invalid("missing AVC separate-plane slices"));
        }
        let slice_bytes = slices
            .capacity()
            .checked_mul(std::mem::size_of::<&SliceHeader>())
            .ok_or_else(|| invalid("AVC separate-plane slice storage overflow"))?;
        let available = budget
            .checked_sub(retained)
            .and_then(|n| n.checked_sub(slice_bytes))
            .ok_or_else(|| invalid("AVC separate-plane reconstruction exceeds memory budget"))?;
        let scaling =
            super::super::avc_scaling::ScalingMatrices::for_colour_plane(sps, pps, component)?;
        let picture = if mono.mb_adaptive_frame_field && !slices[0].field_pic {
            super::super::avc_mbaff_picture::decode_intra_slices_scaled(
                &slices, &mono, pps, available, scaling,
            )?
        } else {
            super::decode_component_intra_slices_scaled(&slices, &mono, pps, available, scaling)?
        };
        retained = picture
            .y
            .capacity()
            .checked_mul(2)
            .and_then(|n| retained.checked_add(n))
            .filter(|&n| n <= budget)
            .ok_or_else(|| invalid("AVC separate-plane capacities exceed memory budget"))?;
        *output = picture.y;
    }
    let [y, cb, cr] = planes;
    let (width, height) = sps.coded_dimensions();
    let divisor = if headers[0].field_pic { 2 } else { 1 };
    let mut crop = sps.crop.map(|v| v as usize);
    crop[2] /= divisor;
    crop[3] /= divisor;
    Ok(IntraPicture {
        coded_width: width as usize,
        coded_height: height as usize / divisor,
        crop,
        bit_depth: sps.bit_depth_luma,
        chroma_bit_depth: sps.bit_depth_luma,
        y,
        cb,
        cr,
    })
}
