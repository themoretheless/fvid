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
#[derive(Clone)]
pub struct LayeredDecoded {
    pub layer_id: u8,
    /// Exact immutable activation used for this coded picture. It outlives
    /// parameter replacement, EOS/EOB, reset and asynchronous output consumers.
    pub binding: Arc<super::LayeredBinding>,
    pub picture: Arc<Picture>,
    pub poc: i32,
    pub coded_output: bool,
    /// Eligibility after OLS/alternative selection; not presentation ordering.
    pub output: bool,
    pub nal: crate::codec::hevc_nal::NalHeader,
    pub access_unit: u64,
    /// Layer-local static HDR snapshot for this picture, preserved through output reordering.
    pub hdr: crate::color::hdr::HdrMetadata,
}
impl LayeredDecoded {
    /// Conservative retained ownership for asynchronous output consumers.
    /// Includes pixels and immutable parameter snapshots; shared owners may
    /// charge the same buffers again rather than underestimating their budget.
    pub fn retained_bytes(&self) -> Result<usize> {
        self.picture
            .storage_bytes()
            .and_then(|n| n.checked_add(self.binding.storage_bytes()))
            .and_then(|n| n.checked_add(self.binding.vps.storage_bytes()))
            .ok_or_else(|| invalid("HEVC output retained storage overflow"))
    }
    /// Membership in the signalled OLS output mask, independently of the coded
    /// picture-output flag. Alternative-output policy and ordering are separate.
    pub fn signalled_output_layer(&self) -> Result<bool> {
        if self.binding.vps.inferred_base_layer() {
            if self.layer_id != 0
                || self.binding.sps.layer_id != 0
                || self.binding.sps.output_layer_set != 0
            {
                return Err(invalid("HEVC inferred base output binding disagrees"));
            }
            return Ok(true);
        }
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
#[derive(Clone)]
pub(super) struct LayeredHdr {
    pub values: [crate::color::hdr::HdrMetadata; 63],
    started: u64,
    pending_mastering: u64,
    pending_clli: u64,
}
impl Default for LayeredHdr {
    fn default() -> Self {
        Self {
            values: [crate::color::hdr::HdrMetadata::default(); 63],
            started: 0,
            pending_mastering: 0,
            pending_clli: 0,
        }
    }
}
impl LayeredHdr {
    fn preview(&self, plan: &LayeredAccessUnit, access: &super::random_access::Preview) -> Self {
        let mut next = self.clone();
        for picture in plan.pictures() {
            let layer = picture.binding.sps.layer_id;
            let bit = 1u64 << layer;
            if access.ignored & bit != 0 {
                continue;
            }
            if access.starts_clvs & bit != 0 && next.started & bit != 0 {
                let pending = next.values[layer as usize];
                next.values[layer as usize] = Default::default();
                if next.pending_mastering & bit != 0 {
                    next.values[layer as usize].mastering = pending.mastering;
                }
                if next.pending_clli & bit != 0 {
                    next.values[layer as usize].light = pending.light;
                }
            }
            next.pending_mastering &= !bit;
            next.pending_clli &= !bit;
            next.started |= bit;
            let received = plan.hdr(layer);
            next.values[layer as usize].merge(received);
            if plan.hdr_clli_received(layer) {
                next.values[layer as usize].light = received.light;
            }
        }
        // Prefix guidance can precede its coded picture in a separate MP4
        // sample/block. Remember which halves arrived since the last picture,
        // so a following CLVS start clears old guidance but retains this seed.
        let present = plan
            .pictures()
            .iter()
            .fold(0, |mask, p| mask | (1u64 << p.binding.sps.layer_id));
        for layer in 0..63 {
            let bit = 1u64 << layer;
            if present & bit != 0 {
                continue;
            }
            let received = plan.hdr(layer);
            next.values[layer as usize].merge(received);
            if received.mastering.is_some() {
                next.pending_mastering |= bit;
            }
            if plan.hdr_clli_received(layer) {
                next.values[layer as usize].light = received.light;
                next.pending_clli |= bit;
            }
        }
        next
    }
    fn end(&mut self, affected: u64) {
        self.pending_mastering &= !affected;
        self.pending_clli &= !affected;
        for layer in 0..63 {
            if affected & (1u64 << layer) != 0 {
                self.values[layer] = Default::default();
            }
        }
    }
}
#[derive(Clone, Default)]
pub struct LayeredDpb {
    poc: LayeredPocState,
    references: Vec<(u8, Reference)>,
    access_unit: u64,
    random_access: super::random_access::RandomAccess,
    hdr: LayeredHdr,
    // Original header history survives seek; pixel random-access starts afresh.
    metadata_access: super::random_access::RandomAccess,
}
impl LayeredDpb {
    pub(super) fn seed_hdr(&mut self, hdr: &LayeredHdr) {
        self.hdr = hdr.clone();
    }
    /// Read skipped header/SEI history without entropy reconstruction or pixel references.
    pub(super) fn prime_metadata(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
    ) -> Result<LayeredParameters> {
        if self.access_unit != 0 || !self.references.is_empty() {
            return Err(invalid("HEVC metadata priming requires a reset decoder"));
        }
        if plan
            .retained_bytes()?
            .saturating_add(std::mem::size_of::<Self>())
            .saturating_add(std::mem::size_of::<LayeredHdr>())
            .saturating_add(std::mem::size_of::<super::random_access::Preview>())
            > budget
        {
            return Err(invalid("HEVC metadata priming exceeds memory budget"));
        }
        let access = self.metadata_access.preview(&plan)?;
        let mut hdr = self.hdr.preview(&plan, &access);
        let (mut parameters, transition, _, end) = plan.into_parts();
        let mut poc = transition.map_or_else(|| self.poc.clone(), |t| t.into_state());
        let affected = if end.bitstream {
            poc.end_bitstream();
            (1u64 << 63) - 1
        } else {
            poc.end_sequences(end.layers)?
        };
        for layer in 0..63 {
            if affected & (1u64 << layer) != 0 {
                parameters.end_layer_sequence(layer);
            }
        }
        hdr.end(affected);
        let mut next_access = access.next;
        next_access.end(end, affected);
        self.hdr = hdr;
        self.metadata_access = next_access;
        self.poc = poc;
        Ok(parameters)
    }
    pub(super) fn finish_metadata_priming(&mut self) {
        self.poc = LayeredPocState::default();
        self.random_access = Default::default();
    }
    pub(super) fn output_preview(
        &self,
        plan: &LayeredAccessUnit,
    ) -> Result<super::random_access::Preview> {
        self.random_access.preview(plan)
    }
    pub(super) fn retained_bytes(&self) -> Result<usize> {
        Ok(storage(&self.references)?.saturating_add(std::mem::size_of::<Self>()))
    }
    pub(super) fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(
            self.references
                .capacity()
                .saturating_mul(std::mem::size_of::<(u8, Reference)>()),
        )
    }
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
        self.decode_with_before(plan, budget, |_, _| Ok(()))
    }
    pub(super) fn decode_with_before<B>(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
        before: B,
    ) -> Result<(LayeredParameters, Vec<LayeredDecoded>, LayeredEndMarkers)>
    where
        B: FnMut(&LayeredAccessPicture, Option<&[(u8, Reference)]>) -> Result<()>,
    {
        use crate::codec::{hevc_picture::decode_layered_with_sources, hevc_pps::LayeredPpsContext};
        use std::mem::size_of;
        let sps_bytes = plan
            .pictures()
            .iter()
            .try_fold(
                size_of::<Vec<crate::codec::hevc_sps::LayeredSps>>(),
                |n, p| n.checked_add(p.binding.storage_bytes()),
            )
            .ok_or_else(|| invalid("HEVC prepared active-SPS storage overflow"))?;
        let components_needed=plan.pictures().iter().any(|p| !p.binding.sps.sps.depth_layer
            && p.binding.sps.sps.three_d.as_ref().is_some_and(|e|e.inter_view_disparity[0]||e.view_synthesis_prediction||e.inter_view_residual_prediction||e.depth_based_partition)
            && p.binding.vps.extension.as_ref().is_some_and(|e|e.graph.reference_list_layers(p.binding.sps.layer_id).is_ok_and(|n|n>0))
            && p.slices.iter().any(|h|h.slice_type!=crate::codec::hevc_cabac::SliceType::I));
        let binding_bytes=if components_needed {
            plan.pictures().len().checked_mul(size_of::<Arc<crate::codec::hevc_decoder::LayeredBinding>>())
                .and_then(|n|n.checked_add(size_of::<Vec<Arc<crate::codec::hevc_decoder::LayeredBinding>>>()))
                .ok_or_else(||invalid("HEVC component binding storage overflow"))?
        }else{0};
        if sps_bytes.checked_add(binding_bytes).is_none_or(|n|n>budget) {
            return Err(invalid("HEVC component bindings exceed memory budget"));
        }
        let mut component_bindings=Vec::new();
        if components_needed {
            component_bindings.try_reserve_exact(plan.pictures().len())
                .map_err(|_|invalid("cannot allocate HEVC component bindings"))?;
            component_bindings.extend(plan.pictures().iter().map(|p|p.binding.clone()));
        }
        use crate::codec::hevc_motion::inter_view_residual::{CurrentRps,BoundSources};
        let snapshot_count=if components_needed {plan.pictures().iter().filter(|p|!p.binding.sps.sps.depth_layer).count()}else{0};
        let mut snapshots=Vec::<CurrentRps>::new();
        snapshots.try_reserve_exact(snapshot_count).map_err(|_|invalid("cannot allocate HEVC AU RPS snapshots"))?;
        let snapshot_headers=if !components_needed {0}else{snapshots.capacity().checked_mul(size_of::<CurrentRps>())
            .and_then(|n|n.checked_add(size_of::<Vec<CurrentRps>>()))
            .ok_or_else(||invalid("HEVC AU RPS snapshot storage overflow"))?};
        let mut snapshot_entries=0usize;
        let available = budget
            .checked_sub(snapshot_headers)
            .and_then(|n|n.checked_sub(sps_bytes)).and_then(|n|n.checked_sub(binding_bytes))
            .ok_or_else(|| invalid("HEVC prepared active SPS exceeds memory budget"))?;
        let active_sps: Vec<_> = plan
            .pictures()
            .iter()
            .map(|p| p.binding.sps.clone())
            .collect();
        let access_unit = self.access_unit;
        self.reconstruct_with_before(
            plan,
            available,
            before,
            |picture, temporal, completed, budget| {
                let snapshot_source=components_needed && !picture.binding.sps.sps.depth_layer
                    && picture.binding.vps.extension.as_ref().map(|e|->Result<bool> {
                        let view=e.graph.view_index(picture.binding.sps.layer_id)?;
                        Ok(e.graph.view_component_for_index(view,false)?==Some(picture.binding.sps.layer_id))
                    }).transpose()?.unwrap_or(false);
                let pending=if snapshot_source {
                    Some(CurrentRps::bind(picture,0,access_unit,temporal,
                        budget.checked_sub(snapshot_entries).ok_or_else(||invalid("HEVC AU RPS entries exceed budget"))?)?)
                }else{None};
                let pending_bytes=pending.as_ref().map_or(0,|s|s.metadata_bytes()-size_of::<CurrentRps>());
                let budget=budget.checked_sub(snapshot_entries).and_then(|n|n.checked_sub(pending_bytes))
                    .ok_or_else(||invalid("HEVC AU RPS entries exceed reconstruction budget"))?;
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
                        available.checked_sub(held).ok_or_else(|| {
                            invalid("HEVC prepared references exceed memory budget")
                        })?,
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
                        let source = reference.picture.as_ref().ok_or_else(|| {
                            invalid("HEVC prepared reference has no source picture")
                        })?;
                        let remaining = available.checked_sub(held).ok_or_else(|| {
                            invalid("HEVC prepared references exceed memory budget")
                        })?;
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
                            .ok_or_else(|| {
                                invalid("HEVC prepared references exceed memory budget")
                            })?;
                        let prepared = Arc::new(prepared);
                        reference.picture = Some(prepared.clone());
                        cache.push((identity, prepared));
                    }
                    lists.push(selected);
                }
                let remaining = available
                    .checked_sub(held)
                    .ok_or_else(|| invalid("HEVC prepared references exceed memory budget"))?;
                // Component tools use the matching view picture in this AU,
                // not a retained picture with the same layer ID from the DPB.
                // CQT is disabled for an I slice, but a later independent P/B
                // slice in the same picture can still require TexturePic.
                let texture_header=picture.slices.iter().find(|h|
                    crate::codec::hevc_picture::cqt_enabled(&picture.binding.sps.sps,h)
                    || crate::codec::hevc_picture::texture_motion_enabled(&picture.binding.sps.sps,h))
                    .unwrap_or(&picture.slices[0]);
                let texture=crate::codec::hevc_picture::select_texture_picture(
                    &picture.binding.context(),texture_header,picture.poc,access_unit,completed)?;
                let first=&picture.slices[0];
                let current=crate::codec::hevc_inter_layer::PictureStatus {
                    identity:PictureIdentity{layer_id:picture.binding.sps.layer_id,poc:picture.poc},
                    access_unit,nal:first.nal,discardable:first.layered.as_ref().is_some_and(|h|h.discardable),
                };
                let needed=!picture.binding.sps.sps.depth_layer
                    && picture.binding.sps.sps.three_d.as_ref().is_some_and(|e|e.inter_view_disparity[0]||e.view_synthesis_prediction||e.inter_view_residual_prediction||e.depth_based_partition)
                    && picture.binding.vps.extension.as_ref().is_some_and(|e|e.graph.reference_list_layers(picture.binding.sps.layer_id).is_ok_and(|n|n>0))
                    && picture.slices.iter().any(|h|h.slice_type!=crate::codec::hevc_cabac::SliceType::I);
                let components=if needed {
                    Some(crate::codec::hevc_picture::ViewComponentPool::bind(
                        &picture.binding.context(),first,current,&component_bindings,false,completed,remaining)?)
                }else{None};
                let remaining=remaining.checked_sub(components.as_ref().map_or(0,|p|p.metadata_bytes()))
                    .ok_or_else(||invalid("HEVC component pool exceeds reconstruction budget"))?;
                let mut refinement_needed=false;
                if needed { for header in &picture.slices {
                    refinement_needed|=crate::codec::hevc_picture::texture_depth_refinement_enabled(&picture.binding.context(),header)?
                        || crate::codec::hevc_picture::texture_vsp_enabled(&picture.binding.context(),header)?
                        || crate::codec::hevc_picture::texture_dbbp_enabled(&picture.binding.context(),header)?;
                }}
                let refinement=if refinement_needed {
                    Some(crate::codec::hevc_picture::ViewComponentPool::bind(
                        &picture.binding.context(),first,current,&component_bindings,true,completed,remaining)?)
                }else{None};
                let remaining=remaining.checked_sub(refinement.as_ref().map_or(0,|p|p.metadata_bytes()))
                    .ok_or_else(||invalid("HEVC refinement pool exceeds reconstruction budget"))?;
                let source_context=picture.binding.context();
                let residual_sources=components.as_ref().map(|pool|BoundSources::bind(
                    pool,&source_context,first,current,&snapshots,temporal)).transpose()?;
                let decoded=Arc::new(decode_layered_with_sources(
                    &picture.binding.context(),
                    &picture.slices,
                    picture.poc,
                    &lists,
                    remaining,
                    texture.as_deref(),
                    components.as_ref().map(|p|(p,current)),
                    refinement.as_ref().map(|p|(p,current)),
                    residual_sources.as_ref(),
                )?);
                if let Some(snapshot)=pending {
                    snapshot_entries=snapshot_entries.checked_add(pending_bytes)
                        .ok_or_else(||invalid("HEVC AU RPS entries storage overflow"))?;
                    snapshots.push(snapshot);
                }
                Ok(decoded)
            },
        )
    }
    /// The callback prepares inter-layer samples/motion and reconstructs one
    /// complete picture. No source state is published unless all layers succeed.
    /// End markers are applied after reconstruction; returned outputs remain
    /// available to the sequence/output owner.
    pub fn reconstruct<F>(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
        decode: F,
    ) -> Result<(LayeredParameters, Vec<LayeredDecoded>, LayeredEndMarkers)>
    where
        F: FnMut(
            &LayeredAccessPicture,
            &[Reference],
            &[(PictureStatus, Arc<Picture>)],
            usize,
        ) -> Result<Arc<Picture>>,
    {
        self.reconstruct_with_before(plan, budget, |_, _| Ok(()), decode)
    }
    fn reconstruct_with_before<F, B>(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
        mut before: B,
        mut decode: F,
    ) -> Result<(LayeredParameters, Vec<LayeredDecoded>, LayeredEndMarkers)>
    where
        F: FnMut(
            &LayeredAccessPicture,
            &[Reference],
            &[(PictureStatus, Arc<Picture>)],
            usize,
        ) -> Result<Arc<Picture>>,
        B: FnMut(&LayeredAccessPicture, Option<&[(u8, Reference)]>) -> Result<()>,
    {
        let random_access = self.random_access.preview(&plan)?;
        let output_mask = plan.output_mask_after_suppression(random_access.suppressed)?;
        let next_access_unit = self
            .access_unit
            .checked_add(1)
            .ok_or_else(|| invalid("HEVC layered access-unit identity overflow"))?;
        let scratch = std::mem::size_of::<LayeredHdr>()
            + 2 * std::mem::size_of::<super::random_access::Preview>()
            + 63usize.saturating_mul(
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
        // NoClras releases every old reference before the first picture.
        // Do not translate these references against a possibly different VPS
        // graph: removed layers are legal here and will never be reconstructed.
        let release_all = plan.pictures().first().is_some_and(|picture| {
            random_access.release[picture.slices[0].nal.layer_id as usize]
                == (1u64 << 63) - 1
        });
        let mut staged = if release_all {
            Vec::new()
        } else if let Some(transition) = plan.transition() {
            transition.translate_references(&self.references, available)?
        } else {
            self.references.clone()
        };
        let metadata_access = self.metadata_access.preview(&plan)?;
        let mut next_hdr = self.hdr.preview(&plan, &metadata_access);
        let mut completed = Vec::new();
        let mut outputs = Vec::new();
        for picture in plan.pictures() {
            let layer = picture.slices[0].nal.layer_id;
            staged.retain(|(id, _)| random_access.release[layer as usize] & (1u64 << id) == 0);
            if random_access.ignored & (1u64 << layer) != 0 {
                before(picture, None)?;
                continue;
            }
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
            staged.retain(|(layer, r)| {
                *layer != first.nal.layer_id || retained.iter().any(|(p, _)| *p == r.poc)
            });
            for (layer, r) in &mut staged {
                if *layer == first.nal.layer_id {
                    r.long_term = retained.iter().find(|(p, _)| *p == r.poc).unwrap().1;
                }
            }
            before(picture, Some(&staged))?;
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
                coded_output: first.picture_output,
                output: output_mask & (1u64 << first.nal.layer_id) != 0,
                nal: first.nal,
                access_unit: self.access_unit,
                hdr: next_hdr.values[layer as usize],
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
        let mut next_random_access = random_access.next;
        next_random_access.end(end, affected);
        next_hdr.end(affected);
        let mut next_metadata_access = metadata_access.next;
        next_metadata_access.end(end, affected);
        self.metadata_access = next_metadata_access;
        self.hdr = next_hdr;
        self.random_access = next_random_access;
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
