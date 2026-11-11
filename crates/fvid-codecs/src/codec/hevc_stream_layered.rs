//! Owned packet entry for an already extracted layered HEVC operation point.
use super::{LayeredAccessUnit, LayeredDecoded, LayeredDpb, LayeredEndMarkers, LayeredParameters};
use crate::codec::{
    config::{HevcConfig, NalUnits},
    hevc_nal::NalHeader,
    hevc_poc::LayeredPocState,
};
use crate::{Result, invalid};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Priming {
    None,
    Parameters,
    Metadata,
}

pub struct LayeredDecoder {
    initial: LayeredParameters,
    parameters: LayeredParameters,
    dpb: LayeredDpb,
    initial_hdr: super::layered_dpb::LayeredHdr,
    length_size: u8,
    output_layer_set: usize,
    initial_output_layer_set: usize,
    budget: usize,
    priming: Priming,
}
impl LayeredDecoder {
    pub fn from_configuration(data: &[u8], output_layer_set: usize, budget: usize) -> Result<Self> {
        let configuration = HevcConfig::parse(data)?;
        let length_size = configuration.length_size;
        let mut initial_hdr = super::layered_dpb::LayeredHdr::default();
        for array in &configuration.arrays {
            if matches!(array.nal_type, 39 | 40) {
                for nal in &array.units {
                    let h = NalHeader::parse(nal)?;
                    if h.layer_id != 63 {
                        let rbsp = crate::codec::hevc_nal::NalRbsp::parse(nal, budget)?;
                        crate::codec::hevc_sei::update_hdr_from_rbsp(
                            &mut initial_hdr.values[h.layer_id as usize], &rbsp.bytes)?;
                    }
                }
            }
        }
        let mut dpb = LayeredDpb::default();
        dpb.seed_hdr(&initial_hdr);
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
            dpb,
            initial_hdr,
            length_size,
            output_layer_set,
            initial_output_layer_set:output_layer_set,
            budget,
            priming: Priming::None,
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
        self.priming = Priming::None;
        self.parameters = self.initial.clone();
        self.output_layer_set=self.initial_output_layer_set;
        self.dpb.reset();
        self.dpb.seed_hdr(&self.initial_hdr);
    }
    /// Returns every decoded layer with explicit identity. Output queuing and
    /// operation-point extraction belong to the caller. Errors publish no state.
    pub fn decode_packet(
        &mut self,
        packet: &[u8],
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        self.require_decode_ready()?;
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
        let budget = self.queued_budget(queue)?;
        let plan = LayeredAccessUnit::parse(
            &self.parameters,
            self.dpb.poc(),
            packet,
            self.length_size,
            self.output_layer_set,
            budget,
        )?;
        self.decode_queued_plan(queue, plan, highest_tid, budget)
    }
    fn queued_budget(&self, queue: &super::LayeredOutputQueue) -> Result<usize> {
        self.require_decode_ready()?;
        // Conservative combined bound: source queue, its transaction, original
        // DPB metadata and the native decoder's own staged working set coexist.
        let held = queue
            .retained_bytes()?
            .saturating_mul(2)
            .saturating_add(self.dpb.metadata_bytes());
        self.available()?
            .checked_sub(held)
            .ok_or_else(|| invalid("HEVC queued decode exceeds memory budget"))
    }
    fn decode_queued_plan(
        &mut self,
        queue: &mut super::LayeredOutputQueue,
        plan: LayeredAccessUnit,
        highest_tid: u8,
        budget: usize,
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
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
        let access_unit = self.dpb.next_access_unit();
        let mut staged_queue = queue.clone();
        if preview.complete_target_idr {
            staged_queue.begin_idr_epoch(access_unit);
        }
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
    /// Extract the globally scanned operation point and schedule it atomically.
    /// An activated VPS mismatch is refused before queue/reference publication.
    pub fn decode_selected_queued_packet(
        &mut self,
        queue: &mut super::LayeredOutputQueue,
        selection: &super::LayeredSubstream,
        packet: &[u8],
        highest_tid: u8,
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        if selection.output_layer_set() != self.output_layer_set {
            return Err(invalid("HEVC extraction OLS disagrees with decoder"));
        }
        if highest_tid != selection.highest_temporal_id() {
            return Err(invalid(
                "HEVC extraction temporal target disagrees with decoder",
            ));
        }
        let available = self.queued_budget(queue)?;
        let extracted = selection.extract_for_decode(packet, self.length_size, available)?;
        let budget = available
            .checked_sub(
                extracted
                    .capacity()
                    .saturating_add(std::mem::size_of_val(&extracted)),
            )
            .ok_or_else(|| invalid("HEVC extracted packet exceeds decode budget"))?;
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
        self.decode_queued_plan(queue, plan, highest_tid, budget)
    }
    /// Decode a source-selected OLS, committing the selection only with the AU.
    /// Ordinary parameter activation still rejects switches inside an active CVS.
    /// reset restores the constructor's OLS. The strict selected entry is unchanged.
    pub fn decode_reselected_queued_packet(
        &mut self,
        queue: &mut super::LayeredOutputQueue,
        selection: &super::LayeredSubstream,
        packet: &[u8],
        highest_tid: u8,
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        let previous = self.output_layer_set;
        self.output_layer_set = selection.output_layer_set();
        let result = self.decode_selected_queued_packet(queue, selection, packet, highest_tid);
        if result.is_err() {
            self.output_layer_set = previous;
        }
        result
    }
    pub fn output_layer_set(&self) -> usize {
        self.output_layer_set
    }
    /// Seek/reset discards pending output and references as one operation.
    pub fn reset_queued(&mut self, queue: &mut super::LayeredOutputQueue) {
        self.reset();
        queue.reset();
    }
    /// Extract one packet with a globally scanned VPS/OLS selection, then decode
    /// atomically. Activated picture bindings must match the selected VPS.
    pub fn decode_selected_packet(
        &mut self,
        selection: &super::LayeredSubstream,
        packet: &[u8],
    ) -> Result<(Vec<LayeredDecoded>, LayeredEndMarkers)> {
        if selection.output_layer_set() != self.output_layer_set {
            return Err(invalid("HEVC extraction OLS disagrees with decoder"));
        }
        self.require_decode_ready()?;
        let available = self.available()?;
        let extracted = selection.extract_for_decode(packet, self.length_size, available)?;
        let budget = available
            .checked_sub(
                extracted
                    .capacity()
                    .saturating_add(std::mem::size_of_val(&extracted)),
            )
            .ok_or_else(|| invalid("HEVC extracted packet exceeds decode budget"))?;
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
    fn require_decode_ready(&self) -> Result<()> {
        if self.priming == Priming::Metadata {
            return Err(invalid("HEVC metadata priming must finish before decoding"));
        }
        Ok(())
    }
    /// Accumulate original skipped parameter, POC, layer initialization and SEI
    /// header history without entropy decoding or allocating reconstructed pictures.
    /// Call finish_metadata_priming before decoding the selected sync sample.
    /// Each packet publishes atomically; this is separate from parameter-only priming.
    pub fn prime_metadata(&mut self, packet: &[u8]) -> Result<()> {
        self.prime_metadata_inner(packet, self.available()?, None)
    }
    /// Prime a scanned operation point in the original parameter namespace.
    /// Selection, extraction storage, metadata and activation commit together;
    /// F.10.3 exported VPS rewriting must not leak into seek's source bindings.
    pub fn prime_selected_metadata(
        &mut self,
        selection: &super::LayeredSubstream,
        packet: &[u8],
    ) -> Result<()> {
        let previous = self.output_layer_set;
        self.output_layer_set = selection.output_layer_set();
        let result = (|| {
            let available = self.available()?;
            let extracted = selection.extract_for_decode(packet, self.length_size, available)?;
            let budget = available
                .checked_sub(extracted.capacity().saturating_add(std::mem::size_of_val(&extracted)))
                .ok_or_else(|| invalid("HEVC extracted priming packet exceeds decode budget"))?;
            self.prime_metadata_inner(&extracted, budget, Some(selection.vps.as_ref()))
        })();
        if result.is_err() {
            self.output_layer_set = previous;
        }
        result
    }
    fn prime_metadata_inner(
        &mut self,
        packet: &[u8],
        budget: usize,
        selected: Option<&crate::codec::hevc_vps::LayeredVps>,
    ) -> Result<()> {
        if self.priming == Priming::Parameters {
            return Err(invalid(
                "HEVC parameter-only and metadata priming cannot be mixed",
            ));
        }
        if self.dpb.next_access_unit() != 0 || self.dpb.references().next().is_some() {
            return Err(invalid("HEVC metadata priming requires a reset decoder"));
        }
        let plan = LayeredAccessUnit::parse(
            &self.parameters,
            self.dpb.poc(),
            packet,
            self.length_size,
            self.output_layer_set,
            budget,
        )?;
        if selected.is_some_and(|v| plan.pictures().iter().any(|p| p.binding.vps.as_ref() != v)) {
            return Err(invalid("HEVC extraction VPS disagrees with active picture"));
        }
        let parameters = self.dpb.prime_metadata(plan, budget)?;
        self.parameters = parameters;
        self.priming = Priming::Metadata;
        Ok(())
    }
    /// Prime skipped AUs under their source epoch's OLS, with the same atomic
    /// parameter-activation checks as prime_metadata.
    pub fn prime_reselected_metadata(
        &mut self,
        packet: &[u8],
        output_layer_set: usize,
    ) -> Result<()> {
        let previous = self.output_layer_set;
        self.output_layer_set = output_layer_set;
        let result = self.prime_metadata(packet);
        if result.is_err() {
            self.output_layer_set = previous;
        }
        result
    }
    /// Discard skipped POC history for pixel reconstruction but preserve original
    /// metadata CLVS history, so seeking to an ordinary CRA cannot reset its HDR.
    pub fn finish_metadata_priming(&mut self) {
        if self.priming == Priming::Metadata {
            self.dpb.finish_metadata_priming();
            self.priming = Priming::None;
        }
    }
    /// Seek priming receives only parameter NALs; it never decodes skipped
    /// pictures or updates POC/DPB state. Full activation remains deferred.
    pub fn prime_parameters(&mut self, packet: &[u8]) -> Result<()> {
        self.require_decode_ready()?;
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
        self.priming = Priming::Parameters;
        Ok(())
    }
}
