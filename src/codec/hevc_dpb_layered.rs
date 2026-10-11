//! Transactional reconstruction/reference storage for already extracted Annex F AUs.
use super::{
    Decoded, LayeredAccessPicture, LayeredAccessUnit, LayeredEndMarkers, LayeredParameters,
};
use crate::codec::{
    hevc_inter_layer::PictureStatus,
    hevc_motion::Reference,
    hevc_picture::Picture,
    hevc_poc::{LayeredPocState, PocPicture},
    hevc_reference_list::PictureIdentity,
};
use crate::{Result, invalid};
use std::sync::Arc;

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
    /// The callback prepares inter-layer samples/motion and reconstructs one
    /// complete picture. No source state is published unless all layers succeed.
    /// End markers are applied after reconstruction; returned outputs remain
    /// available to the sequence/output owner.
    pub fn reconstruct<F>(
        &mut self,
        plan: LayeredAccessUnit,
        budget: usize,
        mut decode: F,
    ) -> Result<(LayeredParameters, Vec<Decoded>, LayeredEndMarkers)>
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
                + std::mem::size_of::<Decoded>()
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
            outputs.push(Decoded {
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
