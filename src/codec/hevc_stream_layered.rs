//! Owned packet entry for an already extracted layered HEVC operation point.
use super::{LayeredAccessUnit, LayeredDecoded, LayeredDpb, LayeredEndMarkers, LayeredParameters};
use crate::codec::{
    config::{HevcConfig, NalUnits},
    hevc_nal::NalHeader,
    hevc_poc::LayeredPocState,
};
use crate::{Result, invalid};

pub struct LayeredDecoder {
    initial: LayeredParameters,
    parameters: LayeredParameters,
    dpb: LayeredDpb,
    length_size: u8,
    output_layer_set: usize,
    budget: usize,
}
impl LayeredDecoder {
    pub fn from_configuration(data: &[u8], output_layer_set: usize, budget: usize) -> Result<Self> {
        let length_size = HevcConfig::parse(data)?.length_size;
        let initial = LayeredParameters::from_configuration(data, budget)?;
        if initial
            .storage_bytes()
            .saturating_mul(2)
            .saturating_add(std::mem::size_of::<Self>())
            > budget
        {
            return Err(invalid("HEVC layered stream state exceeds memory budget"));
        }
        Ok(Self {
            parameters: initial.clone(),
            initial,
            dpb: LayeredDpb::default(),
            length_size,
            output_layer_set,
            budget,
        })
    }
    fn available(&self) -> Result<usize> {
        self.budget
            .checked_sub(
                self.initial
                    .storage_bytes()
                    .saturating_add(self.parameters.storage_bytes())
                    .saturating_add(std::mem::size_of::<Self>()),
            )
            .ok_or_else(|| invalid("HEVC layered stream state exceeds memory budget"))
    }
    pub fn parameters(&self) -> &LayeredParameters {
        &self.parameters
    }
    pub fn poc(&self) -> &LayeredPocState {
        self.dpb.poc()
    }
    pub fn next_access_unit(&self) -> u64 {
        self.dpb.next_access_unit()
    }
    /// Restore the original hvcC bank and discard every decoded reference.
    pub fn reset(&mut self) {
        self.parameters = self.initial.clone();
        self.dpb.reset();
    }
    /// Returns every decoded layer with explicit identity. Output queuing and
    /// operation-point extraction belong to the caller. Errors publish no state.
    pub fn decode_packet(
        &mut self,
        packet: &[u8],
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        let budget = self.available()?;
        let plan = LayeredAccessUnit::parse(
            &self.parameters,
            self.dpb.poc(),
            packet,
            self.length_size,
            self.output_layer_set,
            budget,
        )?;
        let (parameters, pictures, end) = self.dpb.decode(plan, budget)?;
        self.parameters = parameters;
        Ok((pictures, end))
    }
    /// Decode and schedule a complete AU atomically. Use one queue exclusively
    /// with this decoder; seek must reset both. End-of-input requires queue.drain().
    /// Implements output epochs, pre-reconstruction sub-DPB fullness bumping
    /// and additional bumping after the complete AU is reconstructed.
    pub fn decode_queued_packet(
        &mut self,
        queue: &mut super::LayeredOutputQueue,
        packet: &[u8],
        highest_tid: u8,
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        // Conservative combined bound: source queue, its transaction, original
        // DPB metadata and the native decoder's own staged working set coexist.
        let held = queue
            .retained_bytes()?
            .saturating_mul(2)
            .saturating_add(self.dpb.metadata_bytes());
        let budget = self
            .available()?
            .checked_sub(held)
            .ok_or_else(|| invalid("HEVC queued decode exceeds memory budget"))?;
        let plan = LayeredAccessUnit::parse(
            &self.parameters,
            self.dpb.poc(),
            packet,
            self.length_size,
            self.output_layer_set,
            budget,
        )?;
        // Reject invalid temporal limits even if this AU reconstructs no pixels.
        if highest_tid > 6
            || plan.pictures().iter().any(|p| {
                p.slices[0].nal.temporal_id > highest_tid
                    || p.binding
                        .sps
                        .output_ordering
                        .get(usize::from(highest_tid))
                        .is_none()
            })
        {
            return Err(invalid("HEVC queued decode temporal limit is invalid"));
        }
        let preview = self.dpb.output_preview(&plan)?;
        let mut staged_queue = queue.clone();
        let mut ready = Vec::new();
        let mut staged_dpb = self.dpb.clone();
        let (parameters, pictures, end) =
            staged_dpb.decode_with_before(plan, budget, |picture, refs| {
                ready.extend(staged_queue.prepare_picture(picture, &preview));
                if let Some(refs) = refs {
                    ready.extend(staged_queue.bump_before(picture, refs, highest_tid)?);
                }
                Ok(())
            })?;
        let output_budget = budget
            .checked_sub(
                self.dpb
                    .retained_bytes()?
                    .saturating_add(staged_dpb.retained_bytes()?)
                    .saturating_add(parameters.storage_bytes()),
            )
            .ok_or_else(|| invalid("HEVC queued output exceeds memory budget"))?;
        ready.extend(staged_queue.push(pictures, highest_tid, output_budget)?);
        if end.bitstream {
            ready.extend(staged_queue.drain());
        }
        self.parameters = parameters;
        self.dpb = staged_dpb;
        *queue = staged_queue;
        Ok((ready, end))
    }
    /// Seek/reset discards pending output and references as one operation.
    pub fn reset_queued(&mut self, queue: &mut super::LayeredOutputQueue) {
        self.reset();
        queue.reset();
    }
    /// Extract one packet with a globally scanned VPS/OLS selection, then decode
    /// atomically. A changed VPS is refused before reconstruction/publication.
    pub fn decode_selected_packet(
        &mut self,
        selection: &super::LayeredSubstream,
        packet: &[u8],
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        if selection.output_layer_set() != self.output_layer_set {
            return Err(invalid("HEVC extraction OLS disagrees with decoder"));
        }
        let available = self.available()?;
        let extracted = selection.extract_packet(packet, self.length_size, available)?;
        let budget = available
            .checked_sub(
                extracted
                    .capacity()
                    .saturating_add(std::mem::size_of_val(&extracted)),
            )
            .ok_or_else(|| invalid("HEVC extracted packet exceeds decode budget"))?;
        for nal in NalUnits::new(&extracted, self.length_size)? {
            let nal = nal?;
            if NalHeader::parse(nal)?.unit_type == 32
                && crate::codec::hevc_vps::LayeredVps::parse(nal, budget)? != *selection.vps
            {
                return Err(invalid("HEVC extraction VPS disagrees with active picture"));
            }
        }
        let plan = LayeredAccessUnit::parse(
            &self.parameters,
            self.dpb.poc(),
            &extracted,
            self.length_size,
            self.output_layer_set,
            budget,
        )?;
        if plan
            .pictures()
            .iter()
            .any(|p| p.binding.vps.as_ref() != selection.vps.as_ref())
        {
            return Err(invalid("HEVC extraction VPS disagrees with active picture"));
        }
        let (parameters, pictures, end) = self.dpb.decode(plan, budget)?;
        self.parameters = parameters;
        Ok((pictures, end))
    }
    /// Seek priming receives only parameter NALs; it never decodes skipped
    /// pictures or updates POC/DPB state. Full activation remains deferred.
    pub fn prime_parameters(&mut self, packet: &[u8]) -> Result<()> {
        if self.dpb.next_access_unit() != 0 || self.dpb.references().next().is_some() {
            return Err(invalid(
                "HEVC layered parameter priming requires a reset decoder",
            ));
        }
        let budget = self.available()?;
        if packet.len() > budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        let mut parameters = self.parameters.clone();
        for unit in NalUnits::new(packet, self.length_size)? {
            let nal = unit?;
            let header = NalHeader::parse(nal)?;
            if header.layer_id != 63 && matches!(header.unit_type, 32..=34) {
                let remaining =
                    budget
                        .checked_sub(parameters.storage_bytes())
                        .ok_or_else(|| {
                            invalid("HEVC layered primed parameters exceed memory budget")
                        })?;
                parameters.with_available(remaining, |p| p.receive(nal))?;
            }
        }
        if parameters.storage_bytes() > budget {
            return Err(invalid(
                "HEVC layered primed parameters exceed memory budget",
            ));
        }
        self.parameters = parameters;
        Ok(())
    }
}
