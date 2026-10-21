//! Bounded AVC access-unit preparation; arbitrary slice order is normalized before reconstruction.
use super::{
    avc::{Pps, Sps},
    avc_slice::SliceHeader,
    config::NalUnits,
};
use crate::{Result, invalid};
use super::bits::{BitReader, unescape_rbsp};
struct PartitionOwner {
    index: Option<usize>,
    reference: bool,
    seen_intra: bool,
    seen_inter: bool,
}

#[path = "avc_redundant_recovery.rs"]
mod recovery;

/// Complete coded coverage selected before POC/DPB mutation.
/// Disjoint slices may come from multiple redundant ranks with one PPS.
/// Slice headers are reconstruction views: source counter bits remain untouched
/// in their RBSP, while redundant_pic_cnt is normalized for existing builders.
pub struct PreparedPicture<'a> {
    pub slices: Vec<Slice<'a>>,
    pub pps_id: u32,
    /// Lowest source rank used; individual wire ranks remain in `slices[*].nal`.
    pub source_redundant_pic_cnt: u32,
}

/// Ordinary primary-only packets retain their fast path. Packets carrying
/// redundancy validate primary coverage before selecting recovery areas.
pub fn prepare_for_decode<'a>(
    packet: &'a [u8], length_size: u8, sps: &Sps, pps: &Pps,
    parameters: &[(Sps, Pps)], max_bytes: usize, context_budget: usize,
) -> Result<PreparedPicture<'a>> {
    if packet.len() > max_bytes { return Err(invalid("AVC access unit exceeds preparation byte limit")); }
    let mut has_redundant = false;
    if let Some(slices) = prepare_nals(NalUnits::new(packet, length_size)?, sps, pps, Some(parameters), 0, true, &mut has_redundant)? {
        if !has_redundant {
            if slices.first().is_some_and(|s| s.header.first_mb != 0) {
                return Err(invalid("AVC access unit must cover macroblock zero"));
            }
            return Ok(PreparedPicture { slices, pps_id: pps.id, source_redundant_pic_cnt: 0 });
        }
        // Only streams carrying redundancy pay for an entropy coverage probe.
        // Missing/corrupt primary areas are recovered before POC/DPB mutation.
        if !recovery::can_probe(sps, pps) || recovery::complete(&slices, sps, pps, context_budget).unwrap_or(false) {
            return Ok(PreparedPicture { slices, pps_id: pps.id, source_redundant_pic_cnt: 0 });
        }
    }
    recovery::prepare(packet, length_size, sps, pps, parameters, context_budget)
}

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
    prepare_impl(packet, length_size, sps, pps, None, max_bytes)
}

/// Prepare a primary picture while resolving redundant slice headers against
/// their own PPS. Redundant coded areas do not participate in reconstruction
/// when a complete primary coded picture is present (H.264 clause8).
pub fn prepare_with_parameters<'a>(
    packet: &'a [u8], length_size: u8, sps: &Sps, pps: &Pps,
    parameters: &[(Sps, Pps)], max_bytes: usize,
) -> Result<Vec<Slice<'a>>> {
    prepare_impl(packet, length_size, sps, pps, Some(parameters), max_bytes)
}

fn prepare_impl<'a>(
    packet: &'a [u8], length_size: u8, sps: &Sps, pps: &Pps,
    parameters: Option<&[(Sps, Pps)]>, max_bytes: usize,
) -> Result<Vec<Slice<'a>>> {
    if packet.len() > max_bytes {
        return Err(invalid("AVC access unit exceeds preparation byte limit"));
    }
    prepare_nals(NalUnits::new(packet, length_size)?, sps, pps, parameters, 0, false, &mut false)?
        .ok_or_else(|| crate::unsupported("AVC redundant-only access unit requires a primary coded picture"))
}

fn prepare_nals<'a>(
    nals: impl Iterator<Item = Result<&'a [u8]>>,
    sps: &Sps, pps: &Pps, parameters: Option<&[(Sps, Pps)]>, selected_rank: u32,
    allow_partial: bool, saw_redundant: &mut bool,
) -> Result<Option<Vec<Slice<'a>>>> {
    let (width, height) = sps.coded_dimensions();
    let count = (width / 16)
        .checked_mul(height / 16)
        .ok_or_else(|| invalid("AVC macroblock count overflow"))?;
    let mut slices: Vec<Slice<'a>> = Vec::new();
    let mut owners = std::collections::BTreeMap::new();
    // H.264 7.4.1.2.3: primary precedes redundant coded pictures, whose
    // counters are ascending. Their PPS may change at a picture boundary.
    let mut current_rank = None;
    let mut active_redundant_flag = None;
    for nal in nals {
        let nal = nal?;
        match nal[0] & 31 {
            6 | 7 | 8 | 9 | 10 | 11 | 12 => continue,
            1 | 2 | 5 => {}
            3 | 4 => {
                if nal[0] & 0x80 != 0 { return Err(invalid("AVC partition forbidden_zero_bit")); }
                let rbsp = unescape_rbsp(&nal[1..])?;
                let mut bits = BitReader::new(&rbsp);
                let id = bits.unsigned_golomb()?;
                let plane = if sps.separate_colour_plane { bits.read(2)? as u8 } else { 0 };
                let redundant_flag = active_redundant_flag
                    .ok_or_else(|| invalid("AVC residual partition precedes partition A"))?;
                let rank = if redundant_flag { bits.unsigned_golomb()? } else { 0 };
                if rank > 127 { return Err(invalid("AVC partition redundant_pic_cnt outside range")); }
                if current_rank != Some(rank) {
                    return Err(invalid("AVC residual partition has no matching partition A in current coded picture"));
                }
                let owner: &mut PartitionOwner = owners.get_mut(&(id, plane, rank))
                    .ok_or_else(|| invalid("AVC residual partition has no matching partition A"))?;
                if owner.reference != (nal[0] & 0x60 != 0) {
                    return Err(invalid("AVC partition reference class mismatch"));
                }
                let intra = nal[0] & 31 == 3;
                if (intra && (owner.seen_intra || owner.seen_inter)) || (!intra && owner.seen_inter) {
                    return Err(invalid("AVC duplicate or out-of-order residual partition"));
                }
                if bits.remaining() == 0 { return Err(invalid("AVC residual partition has no entropy payload")); }
                let offset = bits.position();
                if intra { owner.seen_intra = true; } else { owner.seen_inter = true; }
                if let Some(index) = owner.index {
                    let partitions = slices[index].header.data_partitions.as_mut().unwrap();
                    let slot = if intra { &mut partitions.intra } else { &mut partitions.inter };
                    *slot = Some(super::avc_slice::ResidualPartition { rbsp, entropy_bit_offset: offset });
                }
                continue;
            }
            _ => return Err(invalid("unsupported in-band AVC NAL")),
        }
        let id = SliceHeader::parameter_set_id(nal)?;
        let (slice_sps, slice_pps) = if id == pps.id { (sps, pps) } else {
            parameters.and_then(|pairs| pairs.iter().find(|(_, p)| p.id == id))
                .map(|(s, p)| (s, p)).ok_or_else(|| invalid("unknown AVC PPS in access unit"))?
        };
        let mut header = SliceHeader::parse(nal, slice_sps, slice_pps)?;
        if current_rank.is_some_and(|rank| header.redundant_pic_cnt < rank) {
            return Err(invalid("AVC coded pictures are out of redundant counter order"));
        }
        *saw_redundant |= header.redundant_pic_cnt != 0;
        current_rank = Some(header.redundant_pic_cnt);
        // B/C headers use the active PPS (7.4.1.2.1 / 7.4.2.2), not a flag
        // frozen at the primary picture. ASO remains valid within each picture.
        active_redundant_flag = Some(slice_pps.redundant_pic_cnt_present);
        if let Some(partitions) = &header.data_partitions {
            let key = (partitions.slice_id, header.colour_plane_id, header.redundant_pic_cnt);
            if owners.insert(key, PartitionOwner {
                index: (header.redundant_pic_cnt == selected_rank).then_some(slices.len()),
                reference: header.nal_ref_idc != 0, seen_intra: false, seen_inter: false,
            }).is_some() {
                return Err(invalid("AVC duplicate partition A slice_id"));
            }
        }
        if header.redundant_pic_cnt == selected_rank {
            // Preserve the wire RBSP and entropy offsets. Reconstruction sees
            // one selected coded picture; its source rank is carried separately.
            header.redundant_pic_cnt = 0;
        }
        if header.redundant_pic_cnt != 0 {
            let Some(first) = slices.first().map(|slice| &slice.header) else {
                return Ok(None);
            };
            if slice_pps.bottom_field_pic_order_present != pps.bottom_field_pic_order_present {
                return Err(invalid("AVC redundant PPS POC flag differs from primary"));
            }
            if header.frame_num != first.frame_num || header.field_pic != first.field_pic
                || header.bottom_field != first.bottom_field || header.idr != first.idr
                || header.idr_pic_id != first.idr_pic_id || header.poc_lsb != first.poc_lsb
                || header.delta_poc_bottom != first.delta_poc_bottom || header.delta_poc != first.delta_poc
                || (header.nal_ref_idc == 0) != (first.nal_ref_idc == 0)
                || header.first_mb >= count
            {
                return Err(invalid("AVC redundant slice picture identity mismatch"));
            }
            // Parse and validate the bounded header, but do not retain RBSP,
            // apply reference marking, or count redundant areas as primary MBs.
            continue;
        }
        if header.pps_id != pps.id {
            return Err(invalid("AVC primary slice references another PPS"));
        }
        let plane_count = if sps.separate_colour_plane { 3 } else { 1 };
        if slices.len() >= (count as usize).saturating_mul(plane_count) {
            return Err(invalid("too many AVC slices for picture"));
        }
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
                || (!sps.separate_colour_plane && header.colour_plane_id != first.colour_plane_id)
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
    // slice_type5..9 promises a uniform type across the entire picture,
    // including slices received earlier or later in arbitrary NAL order.
    if let Some(uniform) = slices.iter().find(|slice| slice.header.all_same_type)
        && slices
            .iter()
            .any(|slice| slice.header.slice_type != uniform.header.slice_type)
    {
        return Err(invalid("AVC uniform slice type promise violated"));
    }
    if sps.separate_colour_plane
        && (0..3).any(|plane| !slices.iter().any(|slice| slice.header.colour_plane_id == plane))
    {
        return Err(invalid("AVC access unit is missing a colour plane"));
    }
    slices.sort_unstable_by_key(|slice| (slice.header.colour_plane_id, slice.header.first_mb));
    if !allow_partial && selected_rank == 0 && slices.first().is_some_and(|s| s.header.first_mb != 0) {
        return Err(invalid("AVC access unit must cover macroblock zero"));
    }
    for index in 0..slices.len().saturating_sub(1) {
        if slices[index].header.colour_plane_id != slices[index + 1].header.colour_plane_id {
            if slices[index + 1].header.first_mb != 0 {
                return Err(invalid("AVC colour plane must cover macroblock zero"));
            }
            continue;
        }
        let end = slices[index + 1].header.first_mb;
        if slices[index].header.first_mb == end {
            return Err(invalid("AVC slice macroblock addresses overlap"));
        }
        slices[index].macroblocks.end = end;
    }
    Ok(Some(slices))
}

#[cfg(test)]
mod tests {
    #[test]
    fn mixed_fields_reject_uniform_slice_type_promises() {
        use super::super::avc_decoder::AvcDecoder;
        let packets: &[&[u8]] = &[
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-plain-mb0-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-plain-mb0-aso-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-plain-mb1-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-plain-mb1-aso-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-primary-mb0-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-primary-mb0-aso-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-primary-mb1-packet.bin"
            ) as &[u8],
            include_bytes!(
                "../../tests/fixtures/playback-errors/avc-invalid-uniform-type-field-primary-mb1-aso-packet.bin"
            ) as &[u8],
        ];
        for packet in packets {
            let mut decoder = AvcDecoder::new(
                include_bytes!("../../tests/fixtures/playback-errors/avc-uniform-type-fields.avcc"),
                4 << 20,
            )
            .unwrap();
            assert!(
                decoder
                    .decode(include_bytes!(
                        "../../tests/fixtures/playback-errors/avc-uniform-type-fields-pcm0.bin"
                    ))
                    .unwrap()
                    .is_none()
            );
            assert!(
                decoder
                    .decode(include_bytes!(
                        "../../tests/fixtures/playback-errors/avc-uniform-type-fields-pcm1.bin"
                    ))
                    .unwrap()
                    .is_some()
            );
            let error = decoder.decode(packet).unwrap_err().to_string();
            assert!(
                error.contains("AVC uniform slice type promise violated"),
                "{error}"
            );
        }
    }
}
