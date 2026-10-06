//! Bounded AVC access-unit preparation; arbitrary slice order is normalized before reconstruction.
use super::{
    avc::{Pps, Sps},
    avc_slice::SliceHeader,
    config::NalUnits,
};
use crate::{Result, invalid};
pub struct Slice<'a> {
    pub nal: &'a [u8],
    pub header: SliceHeader,
    pub macroblocks: std::ops::Range<u32>,
}
/// Parse every slice before changing POC/reference state. Slice data stays owned
/// by the input packet; parsed RBSP storage is bounded by the packet byte limit.
pub fn prepare<'a>(
    packet: &'a [u8],
    length_size: u8,
    sps: &Sps,
    pps: &Pps,
    max_bytes: usize,
) -> Result<Vec<Slice<'a>>> {
    if packet.len() > max_bytes {
        return Err(invalid("AVC access unit exceeds preparation byte limit"));
    }
    let (width, height) = sps.coded_dimensions();
    let count = (width / 16)
        .checked_mul(height / 16)
        .ok_or_else(|| invalid("AVC macroblock count overflow"))?;
    let mut slices: Vec<Slice<'a>> = Vec::new();
    for nal in NalUnits::new(packet, length_size)? {
        let nal = nal?;
        match nal[0] & 31 {
            6 | 7 | 8 | 9 | 12 => continue,
            1 | 5 => {}
            _ => return Err(invalid("unsupported in-band AVC NAL")),
        }
        if slices.len() >= count as usize {
            return Err(invalid("too many AVC slices for picture"));
        }
        let header = SliceHeader::parse(nal, sps, pps)?;
        if header.first_mb >= count {
            return Err(invalid("AVC slice starts outside picture"));
        }
        if !slices.is_empty() {
            let first = &slices[0].header;
            if header.pps_id != first.pps_id
                || header.frame_num != first.frame_num
                || header.field_pic != first.field_pic
                || header.bottom_field != first.bottom_field
                || header.idr != first.idr
                || header.idr_pic_id != first.idr_pic_id
                || header.colour_plane_id != first.colour_plane_id
                || header.poc_lsb != first.poc_lsb
                || header.delta_poc_bottom != first.delta_poc_bottom
                || header.delta_poc != first.delta_poc
                || header.slice_group_change_cycle != first.slice_group_change_cycle
                || (header.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
                || header.no_output_of_prior_pics != first.no_output_of_prior_pics
                || header.long_term_reference != first.long_term_reference
                || header.adaptive_reference_marking != first.adaptive_reference_marking
                || header.memory_operations != first.memory_operations
            {
                return Err(invalid(
                    "AVC slices disagree on picture identity or reference marking",
                ));
            }
        }
        let start = header.first_mb;
        slices.push(Slice {
            nal,
            header,
            macroblocks: start..count,
        });
    }
    slices.sort_unstable_by_key(|slice| slice.header.first_mb);
    if slices.first().is_some_and(|s| s.header.first_mb != 0) {
        return Err(invalid("AVC access unit must cover macroblock zero"));
    }
    for index in 0..slices.len().saturating_sub(1) {
        let end = slices[index + 1].header.first_mb;
        if slices[index].header.first_mb == end {
            return Err(invalid("AVC slice macroblock addresses overlap"));
        }
        slices[index].macroblocks.end = end;
    }
    Ok(slices)
}
