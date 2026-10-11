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
    pub fn pictures(&self) -> &[LayeredAccessPicture] {
        &self.pictures
    }
    pub fn parameters(&self) -> &LayeredParameters {
        &self.parameters
    }
    pub fn transition(&self) -> Option<&PocTransition> {
        self.transition.as_ref()
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
        Ok(Self {
            parameters: staged,
            pictures,
            transition,
            end,
        })
    }
}
