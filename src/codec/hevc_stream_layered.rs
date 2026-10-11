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
