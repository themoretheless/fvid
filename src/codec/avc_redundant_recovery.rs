//! Loss recovery chooses complete coded coverage before any reference mutation.
use super::super::{
    avc::{Pps, Sps},
    avc_slice::{SliceHeader, SliceType},
    config::NalUnits,
};
use super::{PreparedPicture, Slice, prepare_nals};
use crate::{Result, invalid, unsupported};
struct Candidate<'a> {
    rank: u32,
    pps_id: u32,
    nals: Vec<&'a [u8]>,
}
#[derive(PartialEq, Eq)]
struct Identity {
    frame_num: u32,
    field: bool,
    bottom: bool,
    poc: Option<u32>,
    delta_bottom: i32,
    delta: [i32; 2],
    reference: bool,
}
impl Identity {
    fn of(h: &SliceHeader) -> Self {
        Self {
            frame_num: h.frame_num,
            field: h.field_pic,
            bottom: h.bottom_field,
            poc: h.poc_lsb,
            delta_bottom: h.delta_poc_bottom,
            delta: h.delta_poc,
            reference: h.nal_ref_idc != 0,
        }
    }
}
fn parameters<'p>(
    id: u32,
    sps: &'p Sps,
    pps: &'p Pps,
    bank: &'p [(Sps, Pps)],
) -> Result<(&'p Sps, &'p Pps)> {
    if id == pps.id {
        Ok((sps, pps))
    } else {
        bank.iter()
            .find(|(_, p)| p.id == id)
            .map(|(s, p)| (s, p))
            .ok_or_else(|| invalid("unknown AVC PPS in redundant recovery"))
    }
}
pub(super) fn prepare<'a>(
    packet: &'a [u8],
    length: u8,
    sps: &Sps,
    pps: &Pps,
    bank: &[(Sps, Pps)],
    budget: usize,
) -> Result<PreparedPicture<'a>> {
    let mut candidates: Vec<Candidate<'a>> = Vec::new();
    let mut identity = None;
    let mut first_parameters: Option<(&Sps, &Pps)> = None;
    let mut first_idr_id = None;
    for nal in NalUnits::new(packet, length)? {
        let nal = nal?;
        match nal[0] & 31 {
            1 | 2 | 5 => {
                let id = SliceHeader::parameter_set_id(nal)?;
                let (ss, pp) = parameters(id, sps, pps, bank)?;
                let h = SliceHeader::parse(nal, ss, pp)?;
                let key = Identity::of(&h);
                if let Some(first) = &identity {
                    let (fs, fp) = first_parameters.unwrap();
                    if *first != key
                        || fs.coded_dimensions() != ss.coded_dimensions()
                        || fs.crop != ss.crop
                        || fs.frame_num_bits != ss.frame_num_bits
                        || fs.picture_order != ss.picture_order
                        || fs.chroma_format != ss.chroma_format
                        || fs.separate_colour_plane != ss.separate_colour_plane
                        || fs.bit_depth_luma != ss.bit_depth_luma
                        || fs.bit_depth_chroma != ss.bit_depth_chroma
                        || fs.frame_mbs_only != ss.frame_mbs_only
                        || fs.mb_adaptive_frame_field != ss.mb_adaptive_frame_field
                        || fp.bottom_field_pic_order_present != pp.bottom_field_pic_order_present
                        || first_idr_id.zip(h.idr_pic_id).is_some_and(|(a, b)| a != b)
                    {
                        return Err(invalid("AVC redundant recovery picture identity mismatch"));
                    }
                } else {
                    identity = Some(key);
                    first_parameters = Some((ss, pp));
                    first_idr_id = h.idr_pic_id;
                }
                let rank = h.redundant_pic_cnt;
                if candidates.last().is_some_and(|c| c.rank > rank) {
                    return Err(invalid(
                        "AVC coded pictures are out of redundant counter order",
                    ));
                }
                if candidates.last().is_none_or(|c| c.rank != rank) {
                    candidates
                        .try_reserve(1)
                        .map_err(|_| invalid("cannot allocate AVC recovery candidates"))?;
                    candidates.push(Candidate {
                        rank,
                        pps_id: id,
                        nals: Vec::new(),
                    });
                }
            }
            3 | 4 => {}
            6 | 7 | 8 | 9 | 10 | 11 | 12 => continue,
            _ => return Err(invalid("unsupported in-band AVC NAL")),
        }
        let candidate = candidates
            .last_mut()
            .ok_or_else(|| invalid("AVC residual partition precedes partition A"))?;
        candidate
            .nals
            .try_reserve(1)
            .map_err(|_| invalid("cannot allocate AVC recovery NAL index"))?;
        candidate.nals.push(nal);
    }
    let mut last_error = None;
    let mut regions: Vec<Slice<'_>> = Vec::new();
    let mut region_pps = None;
    let mut first_rank = 0;
    for candidate in candidates {
        let (ss, pp) = parameters(candidate.pps_id, sps, pps, bank)?;
        let prepared = prepare_nals(
            candidate.nals.iter().copied().map(Ok),
            ss,
            pp,
            Some(bank),
            candidate.rank,
            true,
            &mut false,
        )
        .and_then(|s| s.ok_or_else(|| invalid("missing selected AVC redundant picture")))
        .and_then(|slices| usable_areas(slices, ss, pp, budget));
        match prepared {
            Ok(mut slices) => match coverage(&slices, ss, pp, budget) {
                Ok(map) => {
                    let whole = map.iter().all(|&v| v != 0);
                    if regions.is_empty() {
                        region_pps = Some(candidate.pps_id);
                        first_rank = candidate.rank;
                    }
                    let compatible = region_pps == Some(candidate.pps_id)
                        && regions.first().is_none_or(|first| {
                            compatible_marking(&first.header, &slices[0].header)
                        });
                    if compatible {
                        let mut occupied = if regions.is_empty() {
                            crate::buffer(map.len())?
                        } else {
                            coverage(&regions, ss, pp, budget)?
                        };
                        for slice in slices.drain(..) {
                            let area = coverage(std::slice::from_ref(&slice), ss, pp, budget)?;
                            if area.iter().zip(&occupied).any(|(&a, &b)| a != 0 && b != 0) {
                                continue;
                            }
                            for (dst, src) in occupied.iter_mut().zip(area) {
                                *dst |= src;
                            }
                            regions
                                .try_reserve(1)
                                .map_err(|_| invalid("cannot allocate AVC recovery regions"))?;
                            regions.push(slice);
                        }
                        if occupied.iter().all(|&v| v != 0) {
                            regions.sort_unstable_by_key(|s| s.header.first_mb);
                            for i in 0..regions.len() {
                                regions[i].macroblocks.end = regions
                                    .get(i + 1)
                                    .map_or(map.len() as u32, |s| s.header.first_mb);
                                // Uniform-type promises apply to a wire representation,
                                // not the virtual picture assembled from several ranks.
                                regions[i].header.all_same_type = false;
                            }
                            return Ok(PreparedPicture {
                                slices: regions,
                                pps_id: candidate.pps_id,
                                source_redundant_pic_cnt: first_rank,
                            });
                        }
                    }
                    if whole {
                        // An overlapping slice cannot be cut at an arbitrary MB:
                        // its entropy/prediction contexts belong to the original slice.
                        // Retain the existing complete-representation fallback.
                        let slices = prepare_nals(
                            candidate.nals.iter().copied().map(Ok),
                            ss,
                            pp,
                            Some(bank),
                            candidate.rank,
                            true,
                            &mut false,
                        )?
                        .ok_or_else(|| invalid("missing complete AVC recovery candidate"))?;
                        let slices = usable_areas(slices, ss, pp, budget)?;
                        return Ok(PreparedPicture {
                            slices,
                            pps_id: candidate.pps_id,
                            source_redundant_pic_cnt: candidate.rank,
                        });
                    }
                }
                Err(e) => last_error = Some(e),
            },
            Err(e) => last_error = Some(e),
        }
    }
    if let Some(error) = last_error {
        Err(invalid(&format!(
            "AVC redundant recovery found no complete coded picture: {error}"
        )))
    } else {
        Err(unsupported(
            "AVC redundant recovery found no complete coded picture",
        ))
    }
}

fn coverage(slices: &[Slice<'_>], sps: &Sps, pps: &Pps, budget: usize) -> Result<Vec<u8>> {
    if !can_probe(sps, pps) {
        return Err(unsupported(
            "AVC redundant recovery requires admitted CAVLC geometry",
        ));
    }
    let first = &slices
        .first()
        .ok_or_else(|| invalid("missing AVC recovery slice"))?
        .header;
    let count = (sps.width_mbs as usize)
        .checked_mul(sps.height_map_units as usize)
        .and_then(|n| {
            n.checked_mul(if !sps.frame_mbs_only && !first.field_pic {
                2
            } else {
                1
            })
        })
        .ok_or_else(|| invalid("AVC recovery macroblock count overflow"))?;
    // Candidate, accumulated and per-slice masks may coexist during merging.
    let remaining = budget
        .checked_sub(
            count
                .checked_mul(3)
                .ok_or_else(|| invalid("AVC recovery mask budget overflow"))?,
        )
        .ok_or_else(|| invalid("AVC recovery coverage exceeds memory budget"))?;
    let mut coverage = crate::buffer(count)?;
    let mut seen = 0;
    let mut mark = |address: usize| -> Result<()> {
        let cell = coverage
            .get_mut(address)
            .ok_or_else(|| invalid("AVC recovery macroblock outside picture"))?;
        if *cell != 0 {
            return Err(invalid("AVC recovery macroblock overlap"));
        }
        *cell = 1;
        seen += 1;
        Ok(())
    };
    for slice in slices {
        let h = &slice.header;
        if matches!(h.slice_type, SliceType::I | SliceType::Si) {
            // 48 count/mode bytes plus FMO/pair storage for admitted mono/420/422.
            let mut reader = super::super::avc_macroblock::IntraCavlcReader::new_fmo(
                h,
                sps,
                pps,
                remaining / 52,
            )?;
            while let Some(block) = reader.read_macroblock()? {
                mark(block.address as usize)?;
            }
        } else {
            let mut reader =
                super::super::avc_inter_slice::InterCavlcSlice::new_fmo(h, sps, pps, remaining)?;
            while let Some(block) = reader.read_macroblock()? {
                use super::super::avc_inter_slice::InterMacroblock;
                let at = match block {
                    InterMacroblock::Intra(b) => b.address as usize,
                    InterMacroblock::Skip { address, .. }
                    | InterMacroblock::Coded { address, .. } => address,
                };
                mark(at)?;
            }
        }
    }
    if seen == 0 {
        return Err(invalid("empty AVC recovery coded area"));
    }
    Ok(coverage)
}

fn compatible_marking(a: &SliceHeader, b: &SliceHeader) -> bool {
    a.idr == b.idr
        && a.idr_pic_id == b.idr_pic_id
        && a.no_output_of_prior_pics == b.no_output_of_prior_pics
        && a.long_term_reference == b.long_term_reference
        && a.adaptive_reference_marking == b.adaptive_reference_marking
        && a.memory_operations == b.memory_operations
        && a.slice_group_change_cycle == b.slice_group_change_cycle
}

pub(super) fn complete(slices: &[Slice<'_>], sps: &Sps, pps: &Pps, budget: usize) -> Result<bool> {
    Ok(coverage(slices, sps, pps, budget)?.iter().all(|&v| v != 0))
}

/// A damaged entropy payload must not erase independently decodable slices.
/// Header association and partition ownership were validated by prepare_nals.
fn usable_areas<'a>(
    slices: Vec<Slice<'a>>,
    sps: &Sps,
    pps: &Pps,
    budget: usize,
) -> Result<Vec<Slice<'a>>> {
    let mut usable = Vec::new();
    let mut failure = None;
    for slice in slices {
        match coverage(std::slice::from_ref(&slice), sps, pps, budget) {
            Ok(_) => {
                usable
                    .try_reserve(1)
                    .map_err(|_| invalid("cannot allocate usable AVC recovery areas"))?;
                usable.push(slice);
            }
            Err(error) => failure = Some(error),
        }
    }
    if usable.is_empty() {
        return Err(failure.unwrap_or_else(|| invalid("missing usable AVC recovery areas")));
    }
    Ok(usable)
}

// Preserve the existing primary reconstruction path for tools whose recovery
// entropy probe is not implemented yet, rather than refusing a healthy primary.
pub(super) fn can_probe(sps: &Sps, pps: &Pps) -> bool {
    !pps.cabac && !sps.separate_colour_plane && sps.chroma_format <= 2
}
