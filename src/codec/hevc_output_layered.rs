//! Annex F output ordering. The queued native decoder invokes per-picture
//! RPS/fullness/epoch transitions before reconstruction and AU bumping after it.
use super::LayeredDecoded;
use crate::{Result, invalid};

#[derive(Clone)]
struct Pending {
    pictures: Vec<LayeredDecoded>,
    latency: u64,
}
/// Queue complete decoded AUs, after suppression and alternative-layer policy.
/// Standalone push implements F.13.5.2.3 additional bumping. Use the queued
/// native decoder for pre-reconstruction DPB fullness and output epochs.
/// Before a POC reset, drain the old epoch; for seek/discard-prior, reset it.
/// Cropping remains represented by each picture's immutable SPS binding.
#[derive(Clone, Default)]
pub struct LayeredOutputQueue {
    pending: Vec<Pending>,
    last_poc: Option<i32>,
    last_access_unit: Option<u64>,
    highest_tid: Option<u8>,
}
impl LayeredOutputQueue {
    pub(crate) fn retained_bytes(&self) -> Result<usize> {
        let mut bytes = std::mem::size_of::<Self>().saturating_add(
            self.pending
                .capacity()
                .saturating_mul(std::mem::size_of::<Pending>()),
        );
        for au in &self.pending {
            bytes = bytes.saturating_add(
                au.pictures
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LayeredDecoded>()),
            );
            for p in &au.pictures {
                bytes = bytes
                    .checked_add(
                        p.picture
                            .storage_bytes()
                            .ok_or_else(|| invalid("HEVC output storage overflow"))?,
                    )
                    .and_then(|n| n.checked_add(p.binding.storage_bytes()))
                    .and_then(|n| n.checked_add(p.binding.vps.storage_bytes()))
                    .ok_or_else(|| invalid("HEVC output storage overflow"))?;
            }
        }
        Ok(bytes)
    }
    pub(super) fn prepare_picture(
        &mut self,
        picture: &super::LayeredAccessPicture,
        preview: &super::random_access::Preview,
    ) -> Vec<LayeredDecoded> {
        let mut ready = Vec::new();
        let p = picture;
        let (mut scope, discard) = preview.output_prior[p.binding.sps.layer_id as usize];
        if scope != 0
            && self
                .pending
                .iter()
                .any(|au| au.pictures[0].binding.vps != p.binding.vps)
        {
            scope = (1u64 << 63) - 1;
        }
        if scope == 0 {
            if p.resetting { ready.extend(self.drain()); }
            return ready;
        }
        if discard {
            for au in &mut self.pending {
                au.pictures.retain(|p| scope & (1u64 << p.layer_id) == 0);
            }
            self.pending.retain(|au| !au.pictures.is_empty());
        } else {
            self.pending.sort_by_key(|au| au.pictures[0].poc);
            // A bump outputs every layer in the earliest AU, including
            // other sub-DPBs, until all targeted sub-DPBs are empty.
            while self.pending.iter().any(|au| {
                au.pictures
                    .iter()
                    .any(|p| scope & (1u64 << p.layer_id) != 0)
            }) {
                ready.extend(self.pending.remove(0).pictures);
            }
            if let Some(last) = ready.last() {
                self.last_poc = Some(last.poc);
            }
        }
        if scope == (1u64 << 63) - 1 {
            self.last_poc = None;
            self.last_access_unit = None;
            self.highest_tid = None;
        }
        // Prior-output discard must happen before the reset drains remaining
        // pictures. Reset ordering last: flushing old output must not restore
        // its POC as the lower bound for the new resetting period.
        if p.resetting { ready.extend(self.drain()); }
        ready
    }
    /// F.13.5.2.2: RPS has already marked unused references. Count the union of
    /// remaining references and pictures needed for output in this sub-DPB.
    pub(super) fn bump_before(
        &mut self,
        picture: &super::LayeredAccessPicture,
        references: &[(u8, crate::codec::hevc_motion::Reference)],
        highest_tid: u8,
    ) -> Result<Vec<LayeredDecoded>> {
        let layer = picture.binding.sps.layer_id;
        let sps = &picture.binding.sps.sps;
        let pps = &picture.binding.pps.pps;
        let two_versions = pps.current_picture_reference
            && (sps.sao || !pps.deblocking.disabled || pps.deblocking.override_enabled);
        let ordering = picture
            .binding
            .sps
            .output_ordering
            .get(usize::from(highest_tid))
            .ok_or_else(|| invalid("HEVC output temporal limit is absent"))?;
        let capacity = usize::from(ordering.max_decoded_pictures)
            .checked_sub(usize::from(two_versions))
            .filter(|&n| n != 0)
            .ok_or_else(|| invalid("HEVC current picture versions exceed DPB capacity"))?;
        let latency = (ordering.max_latency_increase_plus1 != 0).then(|| {
            u64::from(ordering.max_reorder_pictures)
                + u64::from(ordering.max_latency_increase_plus1)
                - 1
        });
        let mut ready = Vec::new();
        if std::env::var_os("FVID_HEVC_QUEUE_TRACE").is_some() {eprintln!("before current layer{} {:?} capacity{} refs{:?} pending{:?}",layer,picture.slices[0].poc_lsb,capacity,references.iter().map(|(l,r)|(*l,r.poc)).collect::<Vec<_>>(),self.pending.iter().map(|p|p.pictures.iter().map(|p|p.poc).collect::<Vec<_>>()).collect::<Vec<_>>());}
        loop {
            let reference_count = references.iter().filter(|(id, _)| *id == layer).count();
            let pending_only = self
                .pending
                .iter()
                .flat_map(|au| &au.pictures)
                .filter(|p| {
                    p.layer_id == layer
                        && !references
                            .iter()
                            .any(|(id, r)| *id == layer && r.poc == p.poc)
                })
                .count();
            if reference_count.saturating_add(pending_only) < capacity
                && self.pending.len() <= ordering.max_reorder_pictures as usize
                && !latency.is_some_and(|max| self.pending.iter().any(|au| au.latency >= max))
            {
                break;
            }
            if self.pending.is_empty() {
                return Err(invalid(
                    "HEVC retained references leave no current DPB slot",
                ));
            }
            self.pending.sort_by_key(|au| au.pictures[0].poc);
            ready.extend(self.pending.remove(0).pictures);
        }
        if let Some(last) = ready.last() {
            self.last_poc = Some(last.poc);
        }
        Ok(ready)
    }
    pub fn pending_access_units(&self) -> usize {
        self.pending.len()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// Flush a completed output epoch in POC order, then ascending layer ID.
    pub fn drain(&mut self) -> Vec<LayeredDecoded> {
        self.pending.sort_by_key(|p| p.pictures[0].poc);
        let out = self.pending.drain(..).flat_map(|p| p.pictures).collect();
        self.last_poc = None;
        self.last_access_unit = None;
        self.highest_tid = None;
        out
    }
    /// Atomically enqueue one complete AU using HighestTid's active OLS limits.
    /// Budget conservatively includes retained pixels/bindings (even when shared
    /// with the decoder) and both old and staged queue metadata. Returned frames
    /// transfer ownership to the caller, which must budget its own output queue.
    pub fn push(
        &mut self,
        pictures: Vec<LayeredDecoded>,
        highest_tid: u8,
        budget: usize,
    ) -> Result<Vec<LayeredDecoded>> {
        let Some(first) = pictures.first() else {
            return Ok(Vec::new());
        };
        if self.highest_tid.is_some_and(|tid| tid != highest_tid) {
            return Err(invalid("HEVC output queue requires an epoch boundary"));
        }
        if self
            .last_access_unit
            .is_some_and(|au| first.access_unit <= au)
        {
            return Err(invalid("HEVC output queue requires an epoch boundary"));
        }
        let ordering = first
            .binding
            .sps
            .output_ordering
            .get(usize::from(highest_tid))
            .ok_or_else(|| invalid("HEVC output temporal limit is absent"))?;
        let reorder = ordering.max_reorder_pictures as usize;
        let latency = (ordering.max_latency_increase_plus1 != 0).then(|| {
            u64::from(ordering.max_reorder_pictures)
                + u64::from(ordering.max_latency_increase_plus1)
                - 1
        });
        let mut layers = 0u64;
        for p in &pictures {
            if p.layer_id > 62
                || layers & (1u64 << p.layer_id) != 0
                || p.layer_id != p.binding.sps.layer_id
                || p.access_unit != first.access_unit
                || p.poc != first.poc
                || p.binding.sps.output_layer_set != first.binding.sps.output_layer_set
                || p.binding.vps != first.binding.vps
                || p.nal.temporal_id > highest_tid
            {
                return Err(invalid(
                    "HEVC output queue requires one complete consistent AU",
                ));
            }
            layers |= 1u64 << p.layer_id;
        }
        for old in &self.pending {
            let p = &old.pictures[0];
            if p.access_unit >= first.access_unit
                || p.poc == first.poc
                || p.binding.vps != first.binding.vps
                || p.binding.sps.output_layer_set != first.binding.sps.output_layer_set
            {
                return Err(invalid("HEVC output queue requires an epoch boundary"));
            }
        }
        if pictures.iter().any(|p| p.output) && self.last_poc.is_some_and(|last| first.poc <= last)
        {
            return Err(invalid(
                "HEVC output POC precedes an already emitted picture",
            ));
        }
        // Charge maximum live metadata before cloning/staging. Each pending AU
        // owns at least one picture; bounds also cover staging/result vectors.
        let count = self
            .pending
            .iter()
            .map(|p| p.pictures.len())
            .sum::<usize>()
            .checked_add(pictures.len())
            .ok_or_else(|| invalid("HEVC output storage overflow"))?;
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(
                self.pending
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Pending>()),
            )
            .saturating_add(
                pictures
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LayeredDecoded>()),
            )
            .saturating_add(count.saturating_mul(
                4 * (std::mem::size_of::<Pending>() + std::mem::size_of::<LayeredDecoded>()),
            ));
        for pending in &self.pending {
            bytes = bytes.saturating_add(
                pending
                    .pictures
                    .capacity()
                    .saturating_mul(std::mem::size_of::<LayeredDecoded>()),
            );
        }
        for p in self
            .pending
            .iter()
            .flat_map(|a| &a.pictures)
            .chain(&pictures)
        {
            bytes = bytes
                .checked_add(
                    p.picture
                        .storage_bytes()
                        .ok_or_else(|| invalid("HEVC output picture storage overflow"))?,
                )
                .and_then(|b| b.checked_add(p.binding.storage_bytes()))
                .and_then(|b| b.checked_add(p.binding.vps.storage_bytes()))
                .ok_or_else(|| invalid("HEVC output storage overflow"))?;
        }
        if bytes > budget {
            return Err(invalid("HEVC output queue exceeds memory budget"));
        }
        let poc = first.poc;
        let access_unit = first.access_unit;
        let mut current: Vec<_> = pictures.into_iter().filter(|p| p.output).collect();
        let mut staged = self.pending.clone();
        if !current.is_empty() {
            for old in &mut staged {
                if old.pictures[0].poc > poc {
                    old.latency = old.latency.saturating_add(1);
                }
            }
            current.sort_by_key(|p| p.layer_id);
            staged.push(Pending {
                pictures: current,
                latency: 0,
            });
        }
        staged.sort_by_key(|p| p.pictures[0].poc);
        let mut ready = Vec::new();
        while staged.len() > reorder
            || latency.is_some_and(|max| staged.iter().any(|p| p.latency >= max))
        {
            if staged.is_empty() {
                break;
            }
            ready.extend(staged.remove(0).pictures);
        }
        if let Some(last) = ready.last() {
            self.last_poc = Some(last.poc);
        }
        self.pending = staged;
        self.last_access_unit = Some(access_unit);
        self.highest_tid = Some(highest_tid);
        Ok(ready)
    }
}
