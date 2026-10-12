//! Transactional reconstruction/reference storage for already extracted Annex F AUs.
use super::{LayeredAccessPicture, LayeredAccessUnit, LayeredEndMarkers, LayeredParameters};
use crate::codec::{
    hevc_inter_layer::PictureStatus,
    hevc_motion::Reference,
    hevc_picture::Picture,
    hevc_poc::{LayeredPocState, PocPicture},
    hevc_reference_list::PictureIdentity,
};
use crate::{Result, invalid};
use std::sync::Arc;

/// Coded-layer identity remains explicit when an AU produces several pictures.
pub struct LayeredDecoded {
    pub layer_id: u8,
    /// Exact immutable activation used for this coded picture. It outlives
    /// parameter replacement, EOS/EOB, reset and asynchronous output consumers.
    pub binding: Arc<super::LayeredBinding>,
    pub picture: Arc<Picture>,
    pub poc: i32,
    pub output: bool,
}
impl LayeredDecoded {
    /// Membership in the signalled OLS output mask, independently of the coded
    /// picture-output flag. Alternative-output policy and ordering are separate.
    pub fn signalled_output_layer(&self) -> Result<bool> {
        let extension = self
            .binding
            .vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("HEVC output binding has no VPS extension"))?;
        let output = extension
            .output
            .sets
            .get(self.binding.sps.output_layer_set)
            .ok_or_else(|| invalid("HEVC output binding has no selected OLS"))?;
        if self.layer_id != self.binding.sps.layer_id || self.layer_id > 62 {
            return Err(invalid("HEVC output picture layer disagrees with binding"));
        }
        Ok(output.output & (1u64 << self.layer_id) != 0)
    }
}
#[derive(Default)]
pub struct LayeredDpb {
    poc: LayeredPocState,
    references: Vec<(u8, Reference)>,
    access_unit: u64,
}
impl LayeredDpb {
    pub fn next_access_unit(&self) -> u64 {
        self.access_unit
    }
    pub fn poc(&self) -> &LayeredPocState {
        &self.poc
    }
    pub fn references(&self) -> impl Iterator<Item = &Reference> {
        self.references.iter().map(|(_, r)| r)
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Reconstruct with owned sample/motion preparation and reference-list
    /// binding. Preparation is cached per source identity across slices/lists.
    /// Budget charges active-SPS copies, retained prepared references, lists and
    /// reconstruction scratch simultaneously; original state stays transactional.
    pub fn decode(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
    ) -> Result<(LayeredParameters, Vec<LayeredDecoded>, LayeredEndMarkers)> {
        use crate::codec::{hevc_picture::decode_layered, hevc_pps::LayeredPpsContext};
        use std::mem::size_of;
        let sps_bytes = plan
            .pictures()
            .iter()
            .try_fold(
                size_of::<Vec<crate::codec::hevc_sps::LayeredSps>>(),
                |n, p| n.checked_add(p.binding.storage_bytes()),
            )
            .ok_or_else(|| invalid("HEVC prepared active-SPS storage overflow"))?;
        let available = budget
            .checked_sub(sps_bytes)
            .ok_or_else(|| invalid("HEVC prepared active SPS exceeds memory budget"))?;
        let active_sps: Vec<_> = plan
            .pictures()
            .iter()
            .map(|p| p.binding.sps.clone())
            .collect();
        let access_unit = self.access_unit;
        self.reconstruct(plan, available, |picture, temporal, completed, budget| {
            let context = LayeredPpsContext {
                vps: &picture.binding.vps,
                sps: &picture.binding.sps,
                active_sps: &active_sps,
                active_pps: &[],
            };
            let list_bytes = picture
                .slices
                .len()
                .checked_mul(size_of::<[Vec<Reference>; 2]>() + 32 * size_of::<Reference>())
                .and_then(|n| {
                    n.checked_add(
                        64 * (size_of::<PictureIdentity>() + size_of::<Arc<Picture>>())
                            + 2 * size_of::<Vec<Reference>>(),
                    )
                })
                .ok_or_else(|| invalid("HEVC prepared list storage overflow"))?;
            let available = budget
                .checked_sub(list_bytes)
                .ok_or_else(|| invalid("HEVC prepared lists exceed memory budget"))?;
            let mut held = 0usize;
            let mut cache: Vec<(PictureIdentity, Arc<Picture>)> = Vec::new();
            let mut lists = Vec::new();
            lists
                .try_reserve_exact(picture.slices.len())
                .map_err(|_| invalid("cannot allocate HEVC prepared slice lists"))?;
            for slice in 0..picture.slices.len() {
                let (_, mut selected) = picture.reference_lists(
                    slice,
                    access_unit,
                    temporal,
                    completed,
                    available
                        .checked_sub(held)
                        .ok_or_else(|| invalid("HEVC prepared references exceed memory budget"))?,
                )?;
                for reference in selected.iter_mut().flat_map(|list| list.iter_mut()) {
                    if reference.layer_id == picture.binding.sps.layer_id {
                        continue;
                    }
                    let identity = reference.identity();
                    if let Some((_, prepared)) = cache.iter().find(|(id, _)| *id == identity) {
                        reference.picture = Some(prepared.clone());
                        continue;
                    }
                    let source = reference
                        .picture
                        .as_ref()
                        .ok_or_else(|| invalid("HEVC prepared reference has no source picture"))?;
                    let remaining = available
                        .checked_sub(held)
                        .ok_or_else(|| invalid("HEVC prepared references exceed memory budget"))?;
                    let prepared = picture.binding.pps.reference_picture(
                        &context,
                        reference.layer_id,
                        source,
                        remaining,
                    )?;
                    let bytes = prepared
                        .additional_reference_storage(source)
                        .ok_or_else(|| invalid("HEVC prepared reference storage overflow"))?;
                    held = held
                        .checked_add(bytes)
                        .filter(|&n| n <= available)
                        .ok_or_else(|| invalid("HEVC prepared references exceed memory budget"))?;
                    let prepared = Arc::new(prepared);
                    reference.picture = Some(prepared.clone());
                    cache.push((identity, prepared));
                }
                lists.push(selected);
            }
            let remaining = available
                .checked_sub(held)
                .ok_or_else(|| invalid("HEVC prepared references exceed memory budget"))?;
            Ok(Arc::new(decode_layered(
                &picture.binding.context(),
                &picture.slices,
                picture.poc,
                &lists,
                remaining,
            )?))
        })
    }
    /// The callback prepares inter-layer samples/motion and reconstructs one
    /// complete picture. No source state is published unless all layers succeed.
    /// End markers are applied after reconstruction; returned outputs remain
    /// available to the sequence/output owner.
    pub fn reconstruct<F>(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
        mut decode: F,
    ) -> Result<(LayeredParameters, Vec<LayeredDecoded>, LayeredEndMarkers)>
    where
        F: FnMut(
            &LayeredAccessPicture,
            &[Reference],
            &[(PictureStatus, Arc<Picture>)],
            usize,
        ) -> Result<Arc<Picture>>,
    {
        let next_access_unit = self
            .access_unit
            .checked_add(1)
            .ok_or_else(|| invalid("HEVC layered access-unit identity overflow"))?;
        let scratch = 63usize.saturating_mul(
            std::mem::size_of::<PictureStatus>()
                + std::mem::size_of::<LayeredDecoded>()
                + 3 * std::mem::size_of::<Reference>()
                + 2 * std::mem::size_of::<Arc<Picture>>(),
        );
        let source_bytes = storage(&self.references)?
            .saturating_add(scratch)
            .saturating_add(std::mem::size_of::<Self>());
        let plan_bytes = plan.retained_bytes()?;
        let mut available = budget
            .checked_sub(source_bytes.saturating_add(plan_bytes))
            .ok_or_else(|| invalid("HEVC layered reconstruction exceeds memory budget"))?;
        // Re-derive from this store, so a plan made against another/stale POC
        // state cannot silently translate or publish this store's references.
        if !plan.pictures().is_empty() {
            let contexts: Vec<_> = plan
                .pictures()
                .iter()
                .map(|p| p.binding.context())
                .collect();
            let pictures: Vec<_> = plan
                .pictures()
                .iter()
                .zip(&contexts)
                .map(|(p, c)| PocPicture {
                    header: &p.slices[0],
                    context: c,
                })
                .collect();
            let fresh = self.poc.preview(&contexts[0].vps, &pictures)?;
            let transition = plan
                .transition()
                .ok_or_else(|| invalid("HEVC layered reconstruction has no POC transition"))?;
            if fresh.clone().into_state() != transition.clone().into_state()
                || (0..63).any(|id| fresh.decrement(id).ok() != transition.decrement(id).ok())
            {
                return Err(invalid(
                    "HEVC layered reconstruction plan has stale POC state",
                ));
            }
        }
        let mut staged = if let Some(transition) = plan.transition() {
            transition.translate_references(&self.references, available)?
        } else {
            self.references.clone()
        };
        let mut completed = Vec::new();
        let mut outputs = Vec::new();
        for picture in plan.pictures() {
            let temporal: Vec<_> = staged.iter().map(|(_, r)| r.clone()).collect();
            let first = &picture.slices[0];
            let (retained, _) = super::temporal_reference_groups(
                first,
                picture.poc,
                &temporal,
                picture.binding.sps.sps.poc_bits,
            )?;
            for header in &picture.slices[1..] {
                let (other, _) = super::temporal_reference_groups(
                    header,
                    picture.poc,
                    &temporal,
                    picture.binding.sps.sps.poc_bits,
                )?;
                if other != retained {
                    return Err(invalid(
                        "HEVC layered slices disagree on retained references",
                    ));
                }
            }
            let held = storage(&staged)?.saturating_add(
                completed
                    .iter()
                    .try_fold(0usize, |n, (_, p): &(PictureStatus, Arc<Picture>)| {
                        n.checked_add(p.storage_bytes()?)
                    })
                    .ok_or_else(|| invalid("HEVC layered completed storage overflow"))?,
            );
            available = budget
                .checked_sub(source_bytes.saturating_add(plan_bytes).saturating_add(held))
                .ok_or_else(|| invalid("HEVC layered reconstruction exceeds memory budget"))?;
            let decoded = decode(picture, &temporal, &completed, available)?;
            let bytes = decoded
                .storage_bytes()
                .ok_or_else(|| invalid("HEVC layered decoded storage overflow"))?;
            if bytes > available {
                return Err(invalid(
                    "HEVC layered decoded picture exceeds memory budget",
                ));
            }
            if decoded.dimensions != picture.binding.sps.sps.dimensions
                || decoded.depth != picture.binding.sps.sps.depth
            {
                return Err(invalid(
                    "HEVC layered reconstructed picture disagrees with SPS",
                ));
            }
            staged.retain(|(layer, r)| {
                *layer != first.nal.layer_id || retained.iter().any(|(p, _)| *p == r.poc)
            });
            for (layer, r) in &mut staged {
                if *layer == first.nal.layer_id {
                    r.long_term = retained.iter().find(|(p, _)| *p == r.poc).unwrap().1;
                }
            }
            if first.nal.unit_type >= 16 || first.nal.unit_type & 1 != 0 {
                if staged
                    .iter()
                    .any(|(_, r)| r.layer_id == first.nal.layer_id && r.poc == picture.poc)
                {
                    return Err(invalid("HEVC layered DPB repeats a picture identity"));
                }
                staged.push((
                    first.nal.layer_id,
                    Reference {
                        layer_id: first.nal.layer_id,
                        poc: picture.poc,
                        long_term: false,
                        picture: Some(decoded.clone()),
                    },
                ));
                let capacity = picture
                    .binding
                    .sps
                    .output_ordering
                    .last()
                    .ok_or_else(|| invalid("HEVC layered SPS has no DPB ordering"))?
                    .max_decoded_pictures as usize;
                if staged
                    .iter()
                    .filter(|(layer, _)| *layer == first.nal.layer_id)
                    .count()
                    > capacity
                {
                    return Err(invalid("HEVC layered decoded reference count exceeds SPS"));
                }
            }
            completed.push((
                PictureStatus {
                    identity: PictureIdentity {
                        layer_id: first.nal.layer_id,
                        poc: picture.poc,
                    },
                    access_unit: self.access_unit,
                    nal: first.nal,
                    discardable: first.layered.as_ref().unwrap().discardable,
                },
                decoded.clone(),
            ));
            outputs.push(LayeredDecoded {
                layer_id: first.nal.layer_id,
                binding: picture.binding.clone(),
                picture: decoded,
                poc: picture.poc,
                output: first.picture_output,
            });
        }
        let (mut parameters, transition, _, end) = plan.into_parts();
        let mut next_poc = transition.map_or_else(|| self.poc.clone(), |t| t.into_state());
        let affected = if end.bitstream {
            next_poc.end_bitstream();
            (1u64 << 63) - 1
        } else {
            next_poc.end_sequences(end.layers)?
        };
        staged.retain(|(layer, _)| affected & (1u64 << layer) == 0);
        for layer in 0..63 {
            if affected & (1u64 << layer) != 0 {
                parameters.end_layer_sequence(layer);
            }
        }
        self.poc = next_poc;
        self.references = staged;
        self.access_unit = next_access_unit;
        Ok((parameters, outputs, end))
    }
}
fn storage(references: &Vec<(u8, Reference)>) -> Result<usize> {
    references
        .iter()
        .try_fold(
            references
                .capacity()
                .saturating_mul(std::mem::size_of::<(u8, Reference)>()),
            |n, (_, r)| n.checked_add(r.picture.as_ref()?.storage_bytes()?),
        )
        .ok_or_else(|| invalid("HEVC layered reference storage overflow"))
}
