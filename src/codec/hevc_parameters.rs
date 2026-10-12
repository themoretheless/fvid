//! Annex F global parameter IDs and separate, transactional layer activations.
use super::super::{
    bits::BitReader,
    config::HevcConfig,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_pps::{LayeredPps, LayeredPpsContext},
    hevc_profile::ProfileTierLevel,
    hevc_slice::{LayeredSliceContext, SliceHeader},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
use crate::{Result, invalid};
use std::{mem::size_of, sync::Arc};

fn bytes<T>(v: &Vec<T>) -> usize {
    v.capacity().saturating_mul(size_of::<T>())
}
fn sps_heap(s: &LayeredSps) -> usize {
    let p = &s.sps;
    let mut n = bytes(&p.profile.sub_layers)
        .saturating_add(bytes(&s.effective_profile.sub_layers))
        .saturating_add(bytes(&p.ordering))
        .saturating_add(bytes(&s.output_ordering))
        .saturating_add(bytes(&p.short_term))
        .saturating_add(bytes(&p.long_term));
    for set in &p.short_term {
        n = n.saturating_add(bytes(set));
    }
    if let Some(palette) = &p.palette {
        n = n.saturating_add(bytes(&palette.initial));
    }
    if let Some(hrd) = p.vui.as_ref().and_then(|v| v.hrd.as_ref()) {
        n = n.saturating_add(bytes(&hrd.layers));
        for layer in &hrd.layers {
            n = n
                .saturating_add(bytes(&layer.nal))
                .saturating_add(bytes(&layer.vcl));
        }
    }
    n
}
fn pps_heap(p: &LayeredPps) -> usize {
    let mut n = bytes(&p.reference_locations);
    if let Some(t) = &p.pps.tiles {
        n = n
            .saturating_add(bytes(&t.column_widths))
            .saturating_add(bytes(&t.row_heights));
    }
    if let Some(q) = &p.pps.chroma_qp_offset_list {
        n = n.saturating_add(bytes(&q.entries));
    }
    if let Some(palette) = &p.pps.palette_initial {
        n = n.saturating_add(bytes(palette));
    }
    if let Some(table) = &p.colour_mapping {
        n = n.saturating_add(table.heap_bytes());
    }
    n
}

/// Immutable active binding. Receiving a later parameter NAL never mutates a
/// binding already used by a picture or another layer.
#[derive(Clone, Debug)]
pub struct LayeredBinding {
    pub vps: Arc<LayeredVps>,
    pub sps: LayeredSps,
    pub pps: LayeredPps,
    sps_nal: Arc<[u8]>,
    pps_nal: Arc<[u8]>,
    vps_nal: Arc<[u8]>,
}
impl LayeredBinding {
    pub fn context(&self) -> LayeredSliceContext<'_> {
        LayeredSliceContext {
            vps: &self.vps,
            sps: &self.sps,
            pps: &self.pps,
        }
    }
    pub(crate) fn same_picture_binding(&self, other: &Self) -> bool {
        self.vps_nal[2..] == other.vps_nal[2..]
            && self.sps_nal[2..] == other.sps_nal[2..]
            && self.pps_nal[2..] == other.pps_nal[2..]
            && self.sps.sps == other.sps.sps
            && self.sps.effective_profile == other.sps.effective_profile
            && self.sps.output_ordering == other.sps.output_ordering
            && self.sps.layer_id == other.sps.layer_id
            && self.sps.output_layer_set == other.sps.output_layer_set
            && self.pps.pps == other.pps.pps
            && self.pps.effective_scaling_lists == other.pps.effective_scaling_lists
            && self.pps.reference_locations == other.pps.reference_locations
            && self.pps.colour_mapping == other.pps.colour_mapping
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(sps_heap(&self.sps))
            .saturating_add(pps_heap(&self.pps))
            // Conservatively charge raw snapshots even if also present in the
            // received table. Old snapshots can outlive replacement of an ID.
            .saturating_add(self.sps_nal.len())
            .saturating_add(self.pps_nal.len())
            .saturating_add(self.vps_nal.len())
            .saturating_add(6 * size_of::<usize>())
    }
}

/// Received VPS/SPS/PPS IDs have one namespace across all NAL layers (F.7.4.2.4.2).
/// Syntax needing active reference layers is resolved only at activation.
/// Clone to stage a complete access unit; publish the clone only after its
/// pictures, POC and DPB transitions have all succeeded.
#[derive(Clone)]
pub struct LayeredParameters {
    vps: [Option<Arc<[u8]>>; 16],
    sps: [Option<Arc<[u8]>>; 16],
    pps: [Option<Arc<[u8]>>; 64],
    active: Vec<Arc<LayeredBinding>>,
    budget: usize,
}
impl LayeredParameters {
    pub fn new(budget: usize) -> Result<Self> {
        let result = Self {
            vps: Default::default(),
            sps: Default::default(),
            pps: std::array::from_fn(|_| None),
            active: Vec::new(),
            budget,
        };
        result.remaining()?;
        Ok(result)
    }
    pub fn from_configuration(data: &[u8], budget: usize) -> Result<Self> {
        let config = HevcConfig::parse(data)?;
        let mut result = Self::new(budget)?;
        for array in config.arrays {
            if matches!(array.nal_type, 32..=34) {
                for nal in array.units {
                    result.receive(nal)?;
                }
            }
        }
        Ok(result)
    }
    /// Conservative retained bound; shared raw snapshots may be counted twice.
    /// Excludes caller-owned configuration/packet bytes and older cloned banks.
    pub fn storage_bytes(&self) -> usize {
        let mut n = size_of::<Self>().saturating_add(bytes(&self.active));
        for nal in self.vps.iter().chain(&self.sps).chain(&self.pps).flatten() {
            n = n
                .saturating_add(nal.len())
                .saturating_add(2 * size_of::<usize>());
        }
        for binding in &self.active {
            n = n.saturating_add(binding.storage_bytes());
        }
        // All activations in one CVS share a single immutable parsed VPS.
        if let Some(binding) = self.active.first() {
            n = n
                .saturating_add(binding.vps.storage_bytes())
                .saturating_add(2 * size_of::<usize>());
        }
        n
    }
    fn remaining(&self) -> Result<usize> {
        self.budget
            .checked_sub(self.storage_bytes())
            .ok_or_else(|| invalid("HEVC layered parameters exceed memory budget"))
    }
    pub(crate) fn with_available<T>(
        &mut self,
        available: usize,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let original = self.budget;
        self.budget = original.min(self.storage_bytes().saturating_add(available));
        let result = operation(self);
        self.budget = original;
        result
    }
    pub fn active(&self, layer: u8) -> Option<&Arc<LayeredBinding>> {
        self.active.iter().find(|b| b.sps.layer_id == layer)
    }
    /// End a layer's CLVS after the caller has drained its pictures/references.
    /// Clearing the last layer also permits activation of a different VPS.
    pub fn end_layer_sequence(&mut self, layer: u8) {
        self.active.retain(|b| b.sps.layer_id != layer);
    }
    /// Store a received parameter NAL by global ID. Full context-dependent
    /// validation occurs at activation. Invalid IDs/headers or budget failures
    /// leave the table unchanged; a later NAL with the same ID replaces it.
    pub fn receive(&mut self, nal: &[u8]) -> Result<()> {
        let kind = NalHeader::parse(nal)?.unit_type;
        let table = match kind {
            32 => &self.vps[..],
            33 => &self.sps[..],
            34 => &self.pps[..],
            _ => return Err(invalid("expected HEVC parameter-set NAL")),
        };
        if table.iter().flatten().any(|old| old.as_ref() == nal) {
            return Ok(());
        }
        let left = self.remaining()?;
        let rbsp = NalRbsp::parse(nal, left)?;
        let mut bits = BitReader::new(&rbsp.bytes);
        let kind = rbsp.header.unit_type;
        let id = match kind {
            32 => {
                if rbsp.header.layer_id != 0 || rbsp.header.temporal_id != 0 {
                    return Err(invalid("invalid HEVC layered VPS header"));
                }
                bits.read(4)?
            }
            33 => {
                if rbsp.header.temporal_id != 0 {
                    return Err(invalid("invalid HEVC layered SPS temporal ID"));
                }
                bits.read(4)?;
                let sub = bits.read(3)? as u8;
                if !(rbsp.header.layer_id != 0 && sub == 7) {
                    bits.bit()?;
                    ProfileTierLevel::read(&mut bits, true, sub)?;
                }
                bits.unsigned_golomb()?
            }
            34 => {
                let id = bits.unsigned_golomb()?;
                if bits.unsigned_golomb()? > 15 {
                    return Err(invalid("HEVC PPS SPS ID exceeds range"));
                }
                id
            }
            _ => return Err(invalid("expected HEVC parameter-set NAL")),
        } as usize;
        let limit = if kind == 34 { 64 } else { 16 };
        if id >= limit {
            return Err(invalid("HEVC parameter-set ID exceeds range"));
        }
        // Account for allocation while the previous value and RBSP still live.
        if nal
            .len()
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(rbsp.bytes.capacity())
            > left
        {
            return Err(invalid(
                "HEVC layered parameter update exceeds memory budget",
            ));
        }
        let slot = match kind {
            32 => &mut self.vps[id],
            33 => &mut self.sps[id],
            _ => &mut self.pps[id],
        };
        if slot.as_deref() != Some(nal) {
            *slot = Some(Arc::from(nal));
        }
        Ok(())
    }
    /// Resolve the PPS ID signalled by a coded slice, without selecting raw
    /// parameter NALs by physical layer or guessing their IDs.
    pub fn activate_slice(
        &mut self,
        nal: &[u8],
        output_layer_set: usize,
    ) -> Result<Arc<LayeredBinding>> {
        if nal.len() > self.remaining()? {
            return Err(invalid("HEVC slice parameter lookup exceeds memory budget"));
        }
        let header = NalHeader::parse(nal)?;
        let id = SliceHeader::parameter_set_id(nal)?;
        self.activate(header.layer_id, id, output_layer_set)
    }
    /// Find the received VPS referenced by this slice's PPS/SPS chain.
    /// This does not activate parameters or release existing sequence pins.
    /// Source inventories can inspect actual references without decoding pixels.
    pub fn referenced_vps(&self, nal: &[u8]) -> Result<&[u8]> {
        let left = self.remaining()?;
        if nal.len() > left {
            return Err(invalid("HEVC slice parameter lookup exceeds memory budget"));
        }
        let id = SliceHeader::parameter_set_id(nal)?;
        let pps = self
            .pps
            .get(usize::from(id))
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("HEVC active PPS is absent"))?;
        let rbsp = NalRbsp::parse(pps, left)?;
        let mut bits = BitReader::new(&rbsp.bytes);
        bits.unsigned_golomb()?;
        let sps_id = bits.unsigned_golomb()? as usize;
        let sps = self
            .sps
            .get(sps_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("HEVC active SPS is absent"))?;
        let sps_rbsp = NalRbsp::parse(sps, left.saturating_sub(rbsp.bytes.capacity()))?;
        let vps_id = BitReader::new(&sps_rbsp.bytes).read(4)? as usize;
        self.vps[vps_id]
            .as_deref()
            .ok_or_else(|| invalid("HEVC active VPS is absent"))
    }
    /// Activate the globally received PPS and its SPS/VPS for one layer. Active
    /// SPS/VPS contents cannot change until the corresponding sequences end.
    /// Failed resolution leaves every prior active binding intact.
    pub fn activate(
        &mut self,
        layer: u8,
        pps_id: u8,
        output_layer_set: usize,
    ) -> Result<Arc<LayeredBinding>> {
        if layer > 62 {
            return Err(invalid("invalid HEVC active layer ID"));
        }
        let left = self.remaining()?;
        let pps_nal = self
            .pps
            .get(usize::from(pps_id))
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("HEVC active PPS is absent"))?;
        let rbsp = NalRbsp::parse(pps_nal, left)?;
        let mut bits = BitReader::new(&rbsp.bytes);
        bits.unsigned_golomb()?;
        let sps_id = bits.unsigned_golomb()? as usize;
        let sps_nal = self
            .sps
            .get(sps_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid("HEVC active SPS is absent"))?;
        let sps_rbsp = NalRbsp::parse(sps_nal, left.saturating_sub(rbsp.bytes.capacity()))?;
        let vps_id = BitReader::new(&sps_rbsp.bytes).read(4)? as usize;
        let vps_nal = self.vps[vps_id]
            .as_ref()
            .ok_or_else(|| invalid("HEVC active VPS is absent"))?;
        if self
            .active
            .iter()
            .any(|b| b.vps_nal.as_ref() != vps_nal.as_ref())
        {
            return Err(invalid("HEVC active VPS changed before sequence end"));
        }
        if self
            .active
            .iter()
            .any(|b| usize::from(b.sps.sps.id) == sps_id && b.sps_nal[2..] != sps_nal[2..])
        {
            return Err(invalid("HEVC active SPS changed before layer sequence end"));
        }
        if self.active(layer).is_some_and(|b| {
            b.sps_nal[2..] != sps_nal[2..] || b.sps.output_layer_set != output_layer_set
        }) {
            return Err(invalid("HEVC active SPS changed before layer sequence end"));
        }
        // Re-resolve inference against currently active reference-layer
        // parameters even when this layer's received SPS/PPS bytes are unchanged.
        let mut scratch = rbsp
            .bytes
            .capacity()
            .saturating_add(sps_rbsp.bytes.capacity());
        for b in &self.active {
            scratch = scratch
                .saturating_add(size_of::<LayeredSps>())
                .saturating_add(sps_heap(&b.sps))
                .saturating_add(size_of::<LayeredPps>())
                .saturating_add(pps_heap(&b.pps));
        }
        let available = left
            .checked_sub(scratch)
            .ok_or_else(|| invalid("HEVC layered activation scratch exceeds memory budget"))?;
        let vps = match self.active.first() {
            Some(b) => Arc::clone(&b.vps),
            None => Arc::new(LayeredVps::parse(vps_nal, available)?),
        };
        let available = available
            .checked_sub(if self.active.is_empty() {
                vps.storage_bytes() + 2 * size_of::<usize>()
            } else {
                0
            })
            .ok_or_else(|| invalid("HEVC layered VPS activation exceeds memory budget"))?;
        let active_sps: Vec<_> = self.active.iter().map(|b| b.sps.clone()).collect();
        let active_pps: Vec<_> = self.active.iter().map(|b| b.pps.clone()).collect();
        let sps = LayeredSps::parse(
            sps_nal,
            &LayeredSpsContext {
                vps: &vps,
                layer_id: layer,
                output_layer_set,
                active_sps: &active_sps,
            },
            available,
        )?;
        let available = available
            .checked_sub(size_of::<LayeredSps>().saturating_add(sps_heap(&sps)))
            .ok_or_else(|| invalid("HEVC layered SPS activation exceeds memory budget"))?;
        let pps = LayeredPps::parse(
            pps_nal,
            &LayeredPpsContext {
                vps: &vps,
                sps: &sps,
                active_sps: &active_sps,
                active_pps: &active_pps,
            },
            available,
        )?;
        // Reserve overlap with the old binding and the new vector slot before
        // publishing anything. No partial activation on a late failure.
        let binding = LayeredBinding {
            vps,
            sps,
            pps,
            sps_nal: Arc::clone(sps_nal),
            pps_nal: Arc::clone(pps_nal),
            vps_nal: Arc::clone(vps_nal),
        };
        if binding
            .storage_bytes()
            .saturating_add(size_of::<Arc<LayeredBinding>>())
            > available
        {
            return Err(invalid("HEVC layered binding exceeds memory budget"));
        }
        let binding = Arc::new(binding);
        if let Some(old) = self.active.iter_mut().find(|b| b.sps.layer_id == layer) {
            *old = Arc::clone(&binding);
        } else {
            let mut active = Vec::with_capacity(self.active.len() + 1);
            if bytes(&active).saturating_add(binding.storage_bytes()) > available {
                return Err(invalid(
                    "HEVC layered activation table exceeds memory budget",
                ));
            }
            active.extend(self.active.iter().cloned());
            active.push(Arc::clone(&binding));
            self.active = active;
        }
        Ok(binding)
    }
}
