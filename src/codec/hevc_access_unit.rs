//! Complete Annex F packet/header/POC planning without publishing decoder state.
use super::parameters::{LayeredBinding, LayeredParameters};
use crate::codec::{
    bits::BitReader,
    config::NalUnits,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_poc::{LayeredPocState, PocPicture, PocTransition},
    hevc_slice::{LayeredSliceContext, SliceHeader},
};
use crate::{Result, invalid};
use std::{mem::size_of, sync::Arc};

pub struct LayeredAccessPicture {
    pub binding: Arc<LayeredBinding>,
    pub slices: Vec<SliceHeader>,
    pub poc: i32,
    pub resetting: bool,
}
impl LayeredAccessPicture {
    /// Resolve temporal RPS and same-AU inter-layer candidates before selecting
    /// lists. Retained temporal identities are returned for the DPB owner.
    /// Reference pictures must already have undergone any required sample/motion
    /// preparation; this method does not substitute unprepared layer pixels.
    pub fn reference_lists(
        &self,
        slice: usize,
        access_unit: u64,
        temporal: &[crate::codec::hevc_motion::Reference],
        completed: &[(
            crate::codec::hevc_inter_layer::PictureStatus,
            Arc<crate::codec::hevc_picture::Picture>,
        )],
        budget: usize,
    ) -> Result<(
        Vec<(i32, bool)>,
        [Vec<crate::codec::hevc_motion::Reference>; 2],
    )> {
        use crate::codec::{hevc_motion::Reference, hevc_reference_list::ReferenceKind};
        let header = self
            .slices
            .get(slice)
            .ok_or_else(|| invalid("HEVC layered slice index out of range"))?;
        let entries = header
            .short_term
            .len()
            .saturating_add(header.long_term.len());
        let needed = entries
            .saturating_mul(4)
            .saturating_add(60)
            .saturating_mul(
                size_of::<Reference>()
                    + size_of::<crate::codec::hevc_reference_list::PictureIdentity>()
                    + size_of::<(i32, bool)>(),
            )
            .saturating_add(temporal.len().saturating_mul(size_of::<i32>()));
        let available = budget
            .checked_sub(needed)
            .ok_or_else(|| invalid("HEVC layered temporal lists exceed memory budget"))?;
        for (index, reference) in temporal.iter().enumerate() {
            if reference.layer_id > 62
                || temporal[..index]
                    .iter()
                    .any(|old| old.identity() == reference.identity())
            {
                return Err(invalid(
                    "invalid or duplicate HEVC temporal picture identity",
                ));
            }
        }
        let (retained, groups) = super::temporal_reference_groups(
            header,
            self.poc,
            temporal,
            self.binding.sps.sps.poc_bits,
        )?;
        let identities = groups
            .each_ref()
            .map(|g| g.iter().map(Reference::identity).collect::<Vec<_>>());
        let bound = header.bind_inter_layer_references(
            &self.binding.context(),
            access_unit,
            self.poc,
            completed,
            available,
        )?;
        let mut lists: [Vec<Reference>; 2] = [Vec::new(), Vec::new()];
        for list in 0..2 {
            // I slices have no inter reference list; P slices have no L1.
            // Do not impose nonbase IRAP inter-list constraints on an absent list.
            if header.references[list] == 0 {
                continue;
            }
            for selected in bound.select(
                [&identities[0], &identities[1], &identities[2]],
                header.current_picture_reference,
                header.references[list] as usize,
                header.list_modification[list].as_deref(),
                list,
                available,
            )? {
                let picture = match selected.kind {
                    ReferenceKind::Current => None,
                    ReferenceKind::InterLayer => Some(bound.resolve(selected)?.clone()),
                    _ => temporal
                        .iter()
                        .find(|r| r.identity() == selected.picture)
                        .ok_or_else(|| invalid("HEVC layered temporal reference is missing"))?
                        .picture
                        .clone(),
                };
                if selected.kind != ReferenceKind::Current && picture.is_none() {
                    return Err(invalid(
                        "HEVC layered reference has no reconstructed picture",
                    ));
                }
                lists[list].push(Reference {
                    layer_id: selected.picture.layer_id,
                    poc: selected.picture.poc,
                    long_term: selected.long_term(),
                    picture,
                });
            }
        }
        Ok((retained, lists))
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayeredEndMarkers {
    pub layers: u64,
    pub bitstream: bool,
}
/// Staged bindings, complete pictures and a common POC transition. The caller
/// reconstructs every picture before publishing these changes or applying the
/// end markers. Input belongs to the caller's selected/extracted operation point.
pub struct LayeredAccessUnit {
    parameters: LayeredParameters,
    pictures: Vec<LayeredAccessPicture>,
    transition: Option<PocTransition>,
    end: LayeredEndMarkers,
    hdr: [crate::color::hdr::HdrMetadata; 63],
}
fn remaining(
    budget: usize,
    original: &LayeredParameters,
    staged: &LayeredParameters,
    pictures: &Vec<LayeredAccessPicture>,
    nal_count: usize,
) -> Result<usize> {
    // Conservative overlap accounting: shared parameter snapshots may be
    // charged in both banks/pictures. Borrowed packet bytes are not allocated.
    let mut held = size_of::<LayeredAccessUnit>()
        .saturating_add(size_of::<PocTransition>())
        .saturating_add(original.storage_bytes())
        .saturating_add(staged.storage_bytes())
        .saturating_add(nal_count.saturating_mul(size_of::<&[u8]>()))
        .saturating_add(
            pictures
                .capacity()
                .saturating_mul(size_of::<LayeredAccessPicture>()),
        );
    for picture in pictures {
        held = held.saturating_add(picture.binding.storage_bytes());
        held = held.saturating_add(
            picture
                .slices
                .capacity()
                .saturating_mul(size_of::<SliceHeader>()),
        );
        for header in &picture.slices {
            held = held.saturating_add(
                header
                    .storage_bytes()?
                    .saturating_sub(size_of::<SliceHeader>()),
            );
        }
    }
    budget
        .checked_sub(held)
        .ok_or_else(|| invalid("HEVC layered access-unit metadata exceeds memory budget"))
}
fn prefix(kind: u8) -> bool {
    matches!(kind, 32..=34 | 39 | 41..=44 | 48..=55)
}
impl LayeredAccessUnit {
    pub(crate) fn retained_bytes(&self) -> Result<usize> {
        let mut n = self
            .parameters
            .storage_bytes()
            .saturating_add(size_of::<Self>())
            .saturating_add(
                self.pictures
                    .capacity()
                    .saturating_mul(size_of::<LayeredAccessPicture>()),
            );
        for picture in &self.pictures {
            n = n
                .saturating_add(picture.binding.storage_bytes())
                .saturating_add(
                    picture
                        .slices
                        .capacity()
                        .saturating_mul(size_of::<SliceHeader>()),
                );
            for header in &picture.slices {
                n = n.saturating_add(header.storage_bytes()?);
            }
        }
        Ok(n)
    }

    /// Final per-AU output eligibility after F.8.1.2 OLS/alternative selection.
    /// Presentation order and random-access RASL suppression are separate.
    pub fn output_mask(&self) -> Result<u64> {
        self.output_mask_after_suppression(0)
    }
    pub(crate) fn output_mask_after_suppression(&self, suppressed: u64) -> Result<u64> {
        let Some(first) = self.pictures.first() else {
            return Ok(0);
        };
        let extension = first
            .binding
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC output policy has no VPS extension"))?;
        let ols = extension
            .output
            .sets
            .get(first.binding.sps.output_layer_set)
            .ok_or_else(|| invalid("HEVC output policy has no selected OLS"))?;
        let coded = self
            .pictures
            .iter()
            .filter(|p| {
                p.slices[0].picture_output && suppressed & (1u64 << p.binding.sps.layer_id) == 0
            })
            .fold(0u64, |mask, p| mask | (1u64 << p.binding.sps.layer_id));
        if ols.alternative_output {
            if ols.output.count_ones() != 1 {
                return Err(invalid("HEVC alternative output requires one output layer"));
            }
            let target = ols.output.trailing_zeros() as u8;
            let position = extension
                .graph
                .layer_ids
                .binary_search(&target)
                .map_err(|_| invalid("HEVC output layer is absent from VPS"))?;
            let references = extension.graph.transitive[position];
            let mut flag = None;
            for p in &self.pictures {
                if ((1u64 << target) | references) & (1u64 << p.binding.sps.layer_id) != 0 {
                    let current = p.slices[0].picture_output;
                    if flag.is_some_and(|old| old != current) {
                        return Err(invalid(
                            "HEVC alternative output pic_output_flag disagrees across layers",
                        ));
                    }
                    flag = Some(current);
                }
            }
            if coded & ols.output == 0 {
                let candidates = coded & references;
                return Ok(if candidates == 0 {
                    0
                } else {
                    1u64 << (63 - candidates.leading_zeros())
                });
            }
        }
        Ok(coded & ols.output)
    }
    pub fn pictures(&self) -> &[LayeredAccessPicture] {
        &self.pictures
    }
    pub fn parameters(&self) -> &LayeredParameters {
        &self.parameters
    }
    pub fn transition(&self) -> Option<&PocTransition> {
        self.transition.as_ref()
    }
    pub(super) fn hdr(&self, layer: u8) -> crate::color::hdr::HdrMetadata {
        self.hdr[layer as usize]
    }
    pub fn end_markers(&self) -> LayeredEndMarkers {
        self.end
    }
    /// Publication remains the reconstruction owner's responsibility. EOS/EOB
    /// are not silently applied before this packet's pictures use their DPB.
    pub fn into_parts(
        self,
    ) -> (
        LayeredParameters,
        Option<PocTransition>,
        Vec<LayeredAccessPicture>,
        LayeredEndMarkers,
    ) {
        (self.parameters, self.transition, self.pictures, self.end)
    }
    pub fn parse(
        parameters: &LayeredParameters,
        poc: &LayeredPocState,
        packet: &[u8],
        length_size: u8,
        output_layer_set: usize,
        budget: usize,
    ) -> Result<Self> {
        if packet.len() > budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        // Two passes avoid allocating a packet-sized guess for one large NAL.
        let mut count = 0usize;
        let mut vcl_count = 0usize;
        let mut last_vcl = None;
        for unit in NalUnits::new(packet, length_size)? {
            let h = NalHeader::parse(unit?)?;
            if h.layer_id != 63 && h.is_vcl() {
                vcl_count += 1;
                last_vcl = Some(count);
            }
            count += 1;
        }
        let mut staged = parameters.clone();
        let mut pictures = Vec::new();
        let capacity = vcl_count.min(63);
        if capacity.saturating_mul(size_of::<LayeredAccessPicture>())
            > remaining(budget, parameters, &staged, &pictures, count)?
        {
            return Err(invalid("HEVC layered picture headers exceed memory budget"));
        }
        pictures
            .try_reserve_exact(capacity)
            .map_err(|_| invalid("cannot allocate HEVC layered pictures"))?;
        let mut end = LayeredEndMarkers::default();
        let mut hdr = [crate::color::hdr::HdrMetadata::default(); 63];
        let mut aud = None;
        let mut seen_vcl = false;
        let mut decoded_units = 0usize;
        for (index, unit) in NalUnits::new(packet, length_size)?.enumerate() {
            let nal = unit?;
            let h = NalHeader::parse(nal)?;
            // 7.4.2.2 explicitly reserves 63 and requires payload ignorance.
            // Do not de-escape or interpret even an invalid future payload.
            if h.layer_id == 63 {
                continue;
            }
            let decoded_index = decoded_units;
            decoded_units += 1;
            if end.bitstream {
                return Err(invalid("HEVC NAL follows end of bitstream"));
            }
            if end.layers & (1u64 << h.layer_id) != 0 && h.unit_type != 37 {
                return Err(invalid("HEVC NAL follows end of layer sequence"));
            }
            let available = remaining(budget, parameters, &staged, &pictures, count)?;
            if prefix(h.unit_type) && last_vcl.is_some_and(|last| index > last) {
                return Err(invalid("HEVC prefix NAL follows last access-unit VCL"));
            }
            if matches!(h.unit_type, 38 | 40 | 45..=47 | 56..=63) && !seen_vcl {
                return Err(invalid("HEVC suffix NAL precedes first access-unit VCL"));
            }
            match h.unit_type {
                32..=34 => staged.with_available(available, |p| p.receive(nal))?,
                35 => {
                    if decoded_index != 0 || aud.is_some() || h.temporal_id != 0 {
                        return Err(invalid(
                            "HEVC access-unit delimiter must be first and unique",
                        ));
                    }
                    let rbsp = NalRbsp::parse(nal, available)?;
                    let mut bits = BitReader::new(&rbsp.bytes);
                    let kind = bits.read(3)?;
                    if kind > 2 {
                        return Err(invalid("invalid HEVC access-unit delimiter picture type"));
                    }
                    bits.finish_rbsp()?;
                    aud = Some((h.layer_id, kind));
                }
                39 | 40 => {
                    let rbsp = NalRbsp::parse(nal, available)?;
                    hdr[h.layer_id as usize]
                        .merge(crate::codec::hevc_sei::hdr_from_rbsp(&rbsp.bytes)?);
                }
                36 | 37 => {
                    if h.temporal_id != 0 || h.unit_type == 37 && h.layer_id != 0 {
                        return Err(invalid("invalid HEVC EOS/EOB header"));
                    }
                    if NalRbsp::parse(nal, available)?.bytes.as_slice() != [128] {
                        return Err(invalid("invalid HEVC EOS/EOB trailing bits"));
                    }
                    if h.unit_type == 36 {
                        end.layers |= 1u64 << h.layer_id;
                    } else {
                        end.bitstream = true;
                    }
                }
                _ if h.is_vcl() => {
                    if pictures
                        .last()
                        .is_some_and(|p: &LayeredAccessPicture| p.binding.sps.layer_id > h.layer_id)
                    {
                        return Err(invalid(
                            "HEVC layered pictures must increase in layer order",
                        ));
                    }
                    let new_picture = pictures
                        .last()
                        .is_none_or(|p| p.binding.sps.layer_id != h.layer_id);
                    if new_picture && nal.get(2).is_none_or(|b| b & 128 == 0) {
                        return Err(invalid(
                            "HEVC layered picture must begin with the first slice",
                        ));
                    }
                    let binding = staged
                        .with_available(available, |p| p.activate_slice(nal, output_layer_set))?;
                    if !new_picture
                        && !pictures
                            .last()
                            .unwrap()
                            .binding
                            .same_picture_binding(&binding)
                    {
                        return Err(invalid("HEVC parameter binding changes within a picture"));
                    }
                    let previous = if new_picture {
                        None
                    } else {
                        pictures.last().unwrap().slices.last()
                    };
                    let context = binding.context();
                    let available = remaining(budget, parameters, &staged, &pictures, count)?;
                    let header = SliceHeader::parse_layered_with_previous(
                        nal, &context, available, previous,
                    )?;
                    if new_picture {
                        if !header.first || header.address != 0 {
                            return Err(invalid(
                                "HEVC layered picture must begin with the first slice",
                            ));
                        }
                        pictures.push(LayeredAccessPicture {
                            binding,
                            slices: Vec::new(),
                            poc: 0,
                            resetting: false,
                        });
                    } else {
                        let picture = pictures.last().unwrap();
                        let first = &picture.slices[0];
                        if header.nal != first.nal
                            || header.pps_id != first.pps_id
                            || header.poc_lsb != first.poc_lsb
                            || header.picture_output != first.picture_output
                            || header.no_output_of_prior_pictures
                                != first.no_output_of_prior_pictures
                            || header.layered != first.layered
                        {
                            return Err(invalid(
                                "HEVC layered slices disagree on picture identity",
                            ));
                        }
                        let same_plane = picture
                            .slices
                            .iter()
                            .rev()
                            .find(|old| old.colour_plane == header.colour_plane);
                        if let Some(previous) = same_plane {
                            let order = |address| {
                                if let Some(tiles) = &picture.binding.pps.pps.tiles {
                                    let sps = &picture.binding.sps.sps;
                                    let side = 1u32 << sps.coding_block_log2[1];
                                    crate::codec::hevc_tiles::tile_scan_address(
                                        tiles,
                                        sps.dimensions.map(|v| v.div_ceil(side)),
                                        address,
                                    )
                                } else {
                                    Ok(address)
                                }
                            };
                            if header.first || order(header.address)? <= order(previous.address)? {
                                return Err(invalid(
                                    "HEVC slice addresses must increase within one picture",
                                ));
                            }
                        } else if !picture.binding.sps.sps.separate_colour_plane
                            || !header.first
                            || header.address != 0
                        {
                            return Err(invalid(
                                "HEVC colour plane must begin with the first slice",
                            ));
                        }
                    }
                    let available = remaining(budget, parameters, &staged, &pictures, count)?;
                    let picture = pictures.last_mut().unwrap();
                    let growth =
                        (picture.slices.len() + 1).saturating_mul(size_of::<SliceHeader>());
                    if header.storage_bytes()?.saturating_add(growth) > available {
                        return Err(invalid("HEVC layered slices exceed memory budget"));
                    }
                    picture
                        .slices
                        .try_reserve_exact(1)
                        .map_err(|_| invalid("cannot allocate HEVC layered slices"))?;
                    picture.slices.push(header);
                    seen_vcl = true;
                }
                _ => { /* SEI/opaque non-VCL payloads are observed by the reconstruction owner. */ }
            }
        }
        for picture in &pictures {
            if picture.binding.sps.sps.separate_colour_plane
                && (0..3).any(|plane| !picture.slices.iter().any(|h| h.colour_plane == plane))
            {
                return Err(invalid(
                    "HEVC layered access unit is missing a colour plane",
                ));
            }
        }
        if let Some((layer, kind)) = aud {
            if pictures
                .first()
                .is_some_and(|p| p.binding.sps.layer_id != layer)
            {
                return Err(invalid(
                    "HEVC access-unit delimiter layer is not the smallest picture layer",
                ));
            }
            for picture in &pictures {
                if picture.slices.iter().any(|h| match h.slice_type {
                    crate::codec::hevc_cabac::SliceType::I => false,
                    crate::codec::hevc_cabac::SliceType::P => kind == 0,
                    crate::codec::hevc_cabac::SliceType::B => kind < 2,
                }) {
                    return Err(invalid(
                        "HEVC access-unit delimiter disagrees with slice types",
                    ));
                }
            }
        }
        let scratch = pictures
            .len()
            .saturating_mul(size_of::<LayeredSliceContext<'_>>() + size_of::<PocPicture<'_>>());
        if scratch > remaining(budget, parameters, &staged, &pictures, count)? {
            return Err(invalid("HEVC layered POC scratch exceeds memory budget"));
        }
        let transition = if pictures.is_empty() {
            None
        } else {
            let contexts: Vec<_> = pictures.iter().map(|p| p.binding.context()).collect();
            let inputs: Vec<_> = pictures
                .iter()
                .zip(&contexts)
                .map(|(p, c)| PocPicture {
                    header: &p.slices[0],
                    context: c,
                })
                .collect();
            Some(poc.preview(&pictures[0].binding.vps, &inputs)?)
        };
        if let Some(plan) = &transition {
            for (picture, derived) in pictures.iter_mut().zip(plan.pictures()) {
                picture.poc = derived.poc;
                picture.resetting = derived.resetting;
            }
        }
        remaining(budget, parameters, &staged, &pictures, count)?;
        let result = Self {
            parameters: staged,
            pictures,
            transition,
            end,
            hdr,
        };
        result.output_mask()?;
        Ok(result)
    }
}
