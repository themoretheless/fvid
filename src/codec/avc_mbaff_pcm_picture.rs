//! Non-420 MBAFF PCM publication in native component geometry.
use super::super::{avc::SliceGroups, avc_slice::SliceType};
use super::{
    layout, plane, write_samples, IntraCabacReader, IntraCavlcReader, IntraLuma, IntraPicture, Pps,
    SliceHeader, Sps,
};
use crate::{invalid, unsupported, Result};

pub(super) fn decode(
    headers: &[&SliceHeader],
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<IntraPicture> {
    let first = *headers
        .first()
        .ok_or_else(|| invalid("missing component MBAFF PCM slices"))?;
    if sps.frame_mbs_only
        || !sps.mb_adaptive_frame_field
        || sps.separate_colour_plane
        || !matches!(sps.chroma_format, 0 | 2 | 3)
        || !matches!(pps.slice_groups, SliceGroups::Single)
        || headers.iter().any(|h| {
            h.field_pic
                || h.slice_type != SliceType::I
                || if sps.chroma_format == 3 {
                    h.disable_deblocking_filter_idc > 2
                } else {
                    h.disable_deblocking_filter_idc != 1
                }
        })
    {
        return Err(unsupported(
            "non-420 MBAFF PCM requires unfiltered single-group I frame slices",
        ));
    }
    if headers.iter().any(|h| {
        h.frame_num != first.frame_num
            || h.pps_id != pps.id
            || pps.sps_id != sps.id
            || h.idr != first.idr
            || h.idr_pic_id != first.idr_pic_id
            || h.poc_lsb != first.poc_lsb
            || h.delta_poc != first.delta_poc
            || h.memory_operations != first.memory_operations
            || h.redundant_pic_cnt != 0
            || h.nal_ref_idc != first.nal_ref_idc
    }) {
        return Err(invalid("component MBAFF PCM slice identity mismatch"));
    }
    let (w, h) = sps.coded_dimensions();
    let (w, h) = (w as usize, h as usize);
    let pixels = w
        .checked_mul(h)
        .ok_or_else(|| invalid("component MBAFF geometry overflow"))?;
    let count = pixels / 256;
    if !(8..=14).contains(&sps.bit_depth_luma)
        || !(8..=14).contains(&sps.bit_depth_chroma)
        || w == 0
        || h == 0
        || w % 16 != 0
        || h % 32 != 0
        || count > 65536
        || headers.len() > count / 2
    {
        return Err(invalid("invalid component MBAFF PCM geometry"));
    }
    let storage = super::super::avc_picture::decoded_storage(sps, w, h)?;
    // Entropy grids, sorted header references and a live wide PCM macroblock.
    // Raw RBSP and caller-held reference pictures are outside this allowance.
    let working = count
        .checked_mul(256)
        .and_then(|n| n.checked_add(16384))
        .ok_or_else(|| invalid("component MBAFF PCM scratch overflow"))?;
    if storage.checked_add(working).is_none_or(|n| n > budget) {
        return Err(invalid("component MBAFF PCM exceeds memory budget"));
    }
    let chroma = if sps.chroma_format == 0 {
        0
    } else if sps.chroma_format == 2 {
        pixels / 2
    } else {
        pixels
    };
    let mut picture = IntraPicture {
        coded_width: w,
        coded_height: h,
        crop: sps.crop.map(|v| v as usize),
        bit_depth: sps.bit_depth_luma,
        chroma_bit_depth: sps.bit_depth_chroma,
        y: plane(pixels)?,
        cb: plane(chroma)?,
        cr: plane(chroma)?,
    };
    let capacity = picture
        .y
        .capacity()
        .checked_add(picture.cb.capacity())
        .and_then(|n| n.checked_add(picture.cr.capacity()))
        .and_then(|n| n.checked_mul(2));
    if capacity
        .and_then(|n| n.checked_add(working))
        .is_none_or(|n| n > budget)
    {
        return Err(invalid("component MBAFF PCM capacities exceed budget"));
    }
    let mut ordered = headers.to_vec();
    ordered.sort_by_key(|h| h.first_mb);
    let mut seen = 0;
    for (i, header) in ordered.iter().enumerate() {
        if header.first_mb as usize * 2 != seen {
            return Err(invalid("component MBAFF PCM coverage gap or overlap"));
        }
        let end = ordered
            .get(i + 1)
            .map_or(count, |h| h.first_mb as usize * 2);
        if end > count || end <= seen {
            return Err(invalid("invalid component MBAFF PCM slice range"));
        }
        if sps.chroma_format == 3 {
            let mw = w / 16;
            let mh = h / 16;
            let mut pair_fields = crate::buffer(count / 2)?;
            pair_fields.fill(255);
            if pps.cabac {
                let mut reader = super::super::avc_cabac::AvcCabac::new(
                    &header.rbsp,
                    header.entropy_bit_offset,
                    SliceType::I,
                    0,
                    header.slice_qp,
                )?;
                loop {
                    if seen >= end {
                        return Err(invalid("component MBAFF CABAC PCM exceeds slice range"));
                    }
                    let pair = seen / 2;
                    let field = if seen % 2 == 0 {
                        let left = pair % mw != 0 && pair_fields[pair - 1] == 1;
                        let top = pair
                            .checked_sub(mw)
                            .is_some_and(|p| pair_fields[p] == 1);
                        let f = super::super::avc_cabac_inter::field_decoding_flag(
                            &mut reader,
                            [left, top],
                        )?;
                        pair_fields[pair] = u8::from(f);
                        f
                    } else {
                        pair_fields[pair] == 1
                    };
                    let [a, b] = super::super::avc_mbaff::macroblock_neighbours(
                        seen,
                        mw,
                        mh,
                        true,
                        [1, 1],
                        |p| match pair_fields.get(p).copied() {
                            Some(0) => Some(false),
                            Some(1) => Some(true),
                            _ => None,
                        },
                    )?;
                    let inc = usize::from(a.is_some()) + usize::from(b.is_some());
                    if !reader.decision(3 + inc)? || !reader.terminate()? {
                        return Err(unsupported(
                            "PCM fast path encountered compressed MBAFF syntax",
                        ));
                    }
                    let (y, [cb, cr]) = reader.pcm_raw::<256>(
                        sps.bit_depth_luma,
                        sps.bit_depth_chroma,
                        true,
                    )?;
                    let target = layout(seen, mw, mh, true, field, [1, 1])?;
                    write_samples(&mut picture.y, w, target, &y)?;
                    write_samples(&mut picture.cb, w, target, &cb)?;
                    write_samples(&mut picture.cr, w, target, &cr)?;
                    seen += 1;
                    if seen % 2 == 0 && reader.terminate()? {
                        reader.finish_slice()?;
                        break;
                    }
                }
            } else {
                let mut bits = super::super::bits::BitReader::new(&header.rbsp);
                bits.skip(header.entropy_bit_offset)?;
                while bits.more_rbsp_data() {
                    if seen >= end {
                        return Err(invalid("component MBAFF CAVLC PCM exceeds slice range"));
                    }
                    let pair = seen / 2;
                    let field = if seen % 2 == 0 {
                        let f = bits.bit()?;
                        pair_fields[pair] = u8::from(f);
                        f
                    } else {
                        pair_fields[pair] == 1
                    };
                    if bits.unsigned_golomb()? != 25 {
                        return Err(unsupported(
                            "PCM fast path encountered compressed MBAFF syntax",
                        ));
                    }
                    while bits.position() % 8 != 0 {
                        if bits.bit()? {
                            return Err(invalid("nonzero AVC PCM alignment bit"));
                        }
                    }
                    let mut y = [0u16; 256];
                    let mut cb = [0u16; 256];
                    let mut cr = [0u16; 256];
                    for sample in &mut y {
                        *sample = bits.read(sps.bit_depth_luma)? as u16;
                    }
                    for sample in &mut cb {
                        *sample = bits.read(sps.bit_depth_chroma)? as u16;
                    }
                    for sample in &mut cr {
                        *sample = bits.read(sps.bit_depth_chroma)? as u16;
                    }
                    let target = layout(seen, mw, mh, true, field, [1, 1])?;
                    write_samples(&mut picture.y, w, target, &y)?;
                    write_samples(&mut picture.cb, w, target, &cb)?;
                    write_samples(&mut picture.cr, w, target, &cr)?;
                    seen += 1;
                }
                if seen % 2 != 0 {
                    return Err(invalid("MBAFF slice ends with incomplete macroblock pair"));
                }
                bits.finish_rbsp()?;
            }
            if seen != end {
                return Err(invalid("incomplete component MBAFF PCM slice"));
            }
            continue;
        }
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
            let Some(mb) = mb else {
                break;
            };
            if mb.address as usize != seen || seen >= end {
                return Err(invalid("component MBAFF PCM macroblock coverage mismatch"));
            }
            let field = match (&cavlc, &cabac) {
                (Some(reader), _) => reader.field_decoding(),
                (_, Some(reader)) => reader.field_decoding(),
                _ => unreachable!(),
            };
            let IntraLuma::Pcm { y, .. } = &mb.luma else {
                return Err(unsupported(
                    "PCM fast path encountered compressed MBAFF syntax",
                ));
            };
            write_samples(
                &mut picture.y,
                w,
                layout(seen, w / 16, h / 16, true, field, [1, 1])?,
                y,
            )?;
            if sps.chroma_format == 2 {
                let wide = mb
                    .chroma422
                    .as_ref()
                    .ok_or_else(|| invalid("missing MBAFF 422 PCM samples"))?;
                let target = layout(seen, w / 16, h / 16, true, field, [2, 1])?;
                write_samples(&mut picture.cb, w / 2, target, &wide.pcm[0])?;
                write_samples(&mut picture.cr, w / 2, target, &wide.pcm[1])?;
            }
            seen += 1;
        }
        if seen != end {
            return Err(invalid("incomplete component MBAFF PCM slice"));
        }
    }
    if seen != count {
        return Err(invalid("incomplete component MBAFF PCM picture"));
    }
    Ok(picture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::config::{AvcConfig, NalUnits};
    #[test]
    fn component_pcm_storage_boundary_matches_native_plane_sizes() {
        let cases:&[(&[u8],&[u8],usize,u8)]=&[
            (include_bytes!("../../tests/fixtures/playback-errors/avc-component-mbaff-pcm/mbaff-pcm-f0-y8-c8-e0-frame-a0-crop0.avcc"),
             include_bytes!("../../tests/fixtures/playback-errors/avc-component-mbaff-pcm/mbaff-pcm-f0-y8-c8-e0-frame-a0-crop0-0.packet"),22528,0),
            (include_bytes!("../../tests/fixtures/playback-errors/avc-component-mbaff-pcm/mbaff-pcm-f2-y8-c8-e1-checker-a0-crop0.avcc"),
             include_bytes!("../../tests/fixtures/playback-errors/avc-component-mbaff-pcm/mbaff-pcm-f2-y8-c8-e1-checker-a0-crop0-0.packet"),26624,2),
        ];
        for &(configuration, packet, limit, format) in cases {
            let config = AvcConfig::parse(configuration).unwrap();
            let sps = Sps::parse(config.sps[0]).unwrap();
            let pps = Pps::parse(config.pps[0], &sps).unwrap();
            let headers = NalUnits::new(packet, config.length_size)
                .unwrap()
                .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                .collect::<Vec<_>>();
            let headers = headers.iter().collect::<Vec<_>>();
            let error = decode(&headers, &sps, &pps, limit - 1)
                .err()
                .expect("undersized budget accepted");
            assert!(error.to_string().contains("memory budget"));
            assert_eq!(
                decode(&headers, &sps, &pps, limit)
                    .unwrap()
                    .chroma_format()
                    .unwrap(),
                format
            );
        }
    }
}
