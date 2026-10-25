//! Stateful native HEVC access-unit decoding and reference picture storage.
use super::{
    config::{HevcConfig, NalUnits},
    hevc_motion::Reference,
    hevc_nal::{NalHeader, NalRbsp},
    hevc_picture::{self, Picture},
    hevc_pps::Pps,
    hevc_sei,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
    hevc_vps::Vps,
};
use crate::color::hdr::HdrMetadata;
use crate::{Result, invalid};
use std::sync::Arc;
#[path = "hevc_parameters.rs"]
mod parameters;
pub use parameters::{LayeredBinding, LayeredParameters};
#[path = "hevc_access_unit.rs"]
mod access_unit;
pub use access_unit::{LayeredAccessPicture, LayeredAccessUnit, LayeredEndMarkers};
#[path = "hevc_random_access.rs"]
mod random_access;
#[path = "hevc_dpb_layered.rs"]
mod layered_dpb;
pub use layered_dpb::{LayeredDecoded, LayeredDpb};
#[path = "hevc_stream_layered.rs"]
mod layered_stream;
pub use layered_stream::LayeredDecoder;
#[path = "hevc_output_layered.rs"]
mod layered_output;
pub use layered_output::LayeredOutputQueue;
#[path = "hevc_substream.rs"]
mod substream;
pub use substream::{extract_layered_substream, LayeredSubstream, SubstreamScan};
pub struct Decoded {
    pub picture: Arc<Picture>,
    pub poc: i32,
    pub output: bool,
}
type RawParameter = (u8, u8, Arc<[u8]>);
#[derive(Clone)]
struct ParameterSets {
    vps: Vec<Vps>,
    sets: Vec<Sps>,
    pps_nals: Vec<(u8, Vec<u8>)>,
    raw_parameters: Vec<RawParameter>,
    raw_generation: u64,
    pairs: Vec<(Sps, Pps)>,
}
pub struct HevcDecoder {
    params: ParameterSets,
    initial_params: ParameterSets,
    decoded_sps: Option<Sps>,
    decoded_vps: Option<Vps>,
    references: Vec<Reference>,
    previous_poc: Option<i32>,
    reset_periods: super::hevc_poc::ResetPeriods,
    suppress_rasl: bool,
    sequence_ended: bool,
    active_pps: Option<u8>,
    hdr: HdrMetadata,
    cll_received: bool,
    pending_hdr_mastering: bool,
    pending_hdr_cll: bool,
    metadata_started: bool,
    metadata_ended: bool,
    primed: HdrMetadata,
    primed_cll_received: bool,
    primed_active_parameters: Option<hevc_sei::ActiveParameterSets>,
    pending_active_parameters: Option<hevc_sei::ActiveParameterSets>,
    active_parameters: Option<hevc_sei::ActiveParameterSets>,
    cvs_raw_generation: u64,
    cvs_no_update: bool,
    length: u8,
    budget: usize,
    failed: bool,
    allow_incomplete_parameters: bool,
}
// Select the declared SPS before parsing PPS tools. Trying every SPS and
// discarding errors hides a valid binding's actual malformed/unsupported tool.
fn bind_pps<'a>(
    nal: &[u8],
    sets: &'a [Sps],
    budget: usize,
    missing: &str,
) -> Result<(&'a Sps, Pps)> {
    let id = Pps::referenced_sps_id(nal, budget)?;
    let sps = sets.iter().find(|s| s.id == id).ok_or_else(|| invalid(missing))?;
    Ok((sps, Pps::parse(nal, sps, budget)?))
}
impl HevcDecoder {
    fn pps_storage(nals: &Vec<(u8, Vec<u8>)>) -> Result<usize> {
        nals.iter().try_fold(nals.capacity().saturating_mul(std::mem::size_of::<(u8, Vec<u8>)>()), |n, (_, raw)| {
            n.checked_add(raw.capacity()).ok_or_else(|| invalid("HEVC received PPS storage overflow"))
        })
    }
    // Current bank, reset snapshot and a staged update can own distinct PPS buffers.
    fn inband_pps_storage(&self, staged: Option<&ParameterSets>) -> Result<usize> {
        if !self.allow_incomplete_parameters { return Ok(0); }
        [&self.params, &self.initial_params, staged.unwrap_or(&self.params)].into_iter()
            .try_fold(0usize, |n, p| n.checked_add(Self::pps_storage(&p.pps_nals)?)
                .ok_or_else(|| invalid("HEVC received PPS storage overflow")))
    }
    fn raw_ownership(lists: &[&Vec<RawParameter>], extra: usize, budget: usize) -> Result<()> {
        let mut seen = [(std::ptr::null::<u8>(), 0usize); 96];
        let mut count = 0;
        let mut bytes = extra;
        for list in lists {
            bytes = bytes.checked_add(list.capacity().saturating_mul(std::mem::size_of::<RawParameter>()))
                .ok_or_else(||invalid("HEVC raw parameter storage overflow"))?;
            for (_,_,raw) in list.iter() {
                let ptr = Arc::as_ptr(raw) as *const u8;
                if seen[..count].iter().any(|(p,_)| *p == ptr) {continue;}
                if count == seen.len() {return Err(invalid("HEVC raw parameter identity domain exceeded"));}
                seen[count] = (ptr,raw.len()); count += 1;
                bytes = bytes.checked_add(raw.len()).and_then(|n|n.checked_add(2*std::mem::size_of::<usize>()))
                    .ok_or_else(||invalid("HEVC raw parameter storage overflow"))?;
            }
        }
        if bytes > budget {return Err(invalid("HEVC raw parameter storage exceeds decode budget"));}
        Ok(())
    }
    fn record_raw(
        &self, updates: &mut Vec<RawParameter>, staged: Option<&ParameterSets>,
        kind:u8, id:u8, nal:&[u8],
    ) -> Result<bool> {
        let queued = updates.iter().position(|(k,i,_)| *k==kind && *i==id);
        let previous = queued.map(|i| &updates[i].2).or_else(||self.params.raw_parameters.iter()
            .find(|(k,i,_)|*k==kind && *i==id).map(|p|&p.2));
        if previous.is_some_and(|raw|raw.as_ref()==nal) {return Ok(false);}
        let overwrite = previous.is_some();
        if let Some(index)=queued {updates.remove(index);}
        updates.try_reserve(1).map_err(|_|invalid("HEVC raw parameter allocation failed"))?;
        let cached = self.params.raw_parameters.iter().chain(&self.initial_params.raw_parameters)
            .find(|(k,i,raw)|*k==kind && *i==id && raw.as_ref()==nal).map(|p|Arc::clone(&p.2));
        let extra = if cached.is_some(){0}else{nal.len().checked_add(2*std::mem::size_of::<usize>())
            .ok_or_else(||invalid("HEVC raw parameter storage overflow"))?};
        let extra=extra.checked_add(self.inband_pps_storage(staged)?)
            .ok_or_else(||invalid("HEVC received parameter storage overflow"))?;
        let empty=Vec::new();
        Self::raw_ownership(&[&self.params.raw_parameters,&self.initial_params.raw_parameters,
            updates,staged.map_or(&empty,|p|&p.raw_parameters)],extra,self.budget)?;
        updates.push((kind,id,cached.unwrap_or_else(||Arc::from(nal))));
        Ok(overwrite)
    }
    pub fn from_configuration(data: &[u8], budget: usize) -> Result<Self> {
        Self::from_configuration_impl(data, budget, false)
    }
    /// Permit an initially incomplete hvcC bank and receive parameters in packets.
    /// Received PPS syntax is resolved when its SPS arrives. Metadata queries can
    /// use try_parameters() before a complete parameter pair is available.
    pub fn from_inband_configuration(data: &[u8], budget: usize) -> Result<Self> {
        Self::from_configuration_impl(data, budget, true)
    }
    fn from_configuration_impl(data: &[u8], budget: usize, allow_incomplete_parameters: bool) -> Result<Self> {
        let config = HevcConfig::parse(data)?;
        let mut vps = Vec::new();
        let mut raw_parameters: Vec<RawParameter> = Vec::new();
        for array in &config.arrays {
            if array.nal_type == 32 {
                for nal in &array.units {
                    let new = Vps::parse(nal, budget)?;
                    if vps.iter().any(|v: &Vps| v.id == new.id) {
                        return Err(invalid("duplicate HEVC VPS"));
                    }
                    raw_parameters.try_reserve(1).map_err(|_|invalid("HEVC raw parameter allocation failed"))?;
                    Self::raw_ownership(&[&raw_parameters,&raw_parameters],nal.len()+2*std::mem::size_of::<usize>(),budget)?;
                    raw_parameters.push((32,new.id,Arc::from(&nal[..])));
                    vps.push(new);
                }
            }
        }
        let mut sets = Vec::new();
        for array in &config.arrays {
            if array.nal_type == 33 {
                for nal in &array.units {
                    let sps = Sps::parse(nal, budget)?;
                    if sets.iter().any(|s: &Sps| s.id == sps.id) {
                        return Err(invalid("duplicate HEVC SPS"));
                    }
                    raw_parameters.try_reserve(1).map_err(|_|invalid("HEVC raw parameter allocation failed"))?;
                    Self::raw_ownership(&[&raw_parameters,&raw_parameters],nal.len()+2*std::mem::size_of::<usize>(),budget)?;
                    raw_parameters.push((33,sps.id,Arc::from(&nal[..])));
                    sets.push(sps);
                }
            }
        }
        let mut pairs = Vec::new();
        let mut pps_nals = Vec::new();
        for array in &config.arrays {
            if array.nal_type == 34 {
                for nal in &array.units {
                    let (id, sps_id) = Pps::identifiers(nal, budget)?;
                    let binding = if let Some(sps) = sets.iter().find(|s| s.id == sps_id) {
                        Some((sps.clone(), Pps::parse(nal, sps, budget)?))
                    } else if allow_incomplete_parameters { None }
                    else { return Err(invalid("HEVC PPS has no valid SPS")); };
                    if pps_nals.iter().any(|(old, _)| *old == id) {
                        return Err(invalid("duplicate HEVC PPS"));
                    }
                    if allow_incomplete_parameters {
                        pps_nals.try_reserve_exact(1).map_err(|_| invalid("cannot allocate received HEVC PPS bank"))?;
                        let bytes = Self::pps_storage(&pps_nals)?.checked_add(nal.len())
                            .and_then(|n| n.checked_mul(2)).ok_or_else(|| invalid("HEVC received PPS storage overflow"))?;
                        Self::raw_ownership(&[&raw_parameters, &raw_parameters], bytes, budget)?;
                    }
                    pps_nals.push((id, nal.to_vec()));
                    if let Some(pair) = binding { pairs.push(pair); }
                }
            }
        }
        if pairs.is_empty() && !allow_incomplete_parameters {
            return Err(invalid("HEVC configuration has no parameter-set pair"));
        }
        let pps_bytes = if allow_incomplete_parameters { Self::pps_storage(&pps_nals)?.checked_mul(2)
            .ok_or_else(|| invalid("HEVC received PPS storage overflow"))? } else { 0 };
        Self::raw_ownership(&[&raw_parameters,&raw_parameters],pps_bytes,budget)?;
        let mut primed = HdrMetadata::default();
        let mut primed_cll_received = false;
        let mut primed_active_parameters = None;
        for array in &config.arrays {
            // A muxer that wrote the encoder's HDR SEI messages into the
            // configuration record states the light before a single packet is
            // decoded, which is when a caller grading the first picture needs
            // it. A message this module cannot walk says nothing here rather
            // than failing an open.
            if !hevc_sei::is_sei_unit(array.nal_type) {
                continue;
            }
            for nal in &array.units {
                if let Ok(Some(active)) = hevc_sei::active_parameters_from_nal(nal, budget) {
                    primed_active_parameters = Some(active);
                }
                if let Ok(received) = hevc_sei::update_hdr_from_nal(&mut primed, nal, budget) {
                    primed_cll_received |= received;
                }
            }
        }
        let params = ParameterSets {
            vps,
            sets,
            pps_nals,
            raw_parameters,
            raw_generation: 0,
            pairs,
        };
        Ok(Self {
            initial_params: params.clone(),
            params,
            decoded_sps: None,
            decoded_vps: None,
            references: Vec::new(),
            previous_poc: None,
            reset_periods: Default::default(),
            suppress_rasl: false,
            sequence_ended: false,
            active_pps: None,
            hdr: primed,
            cll_received: primed_cll_received,
            pending_hdr_mastering: primed.mastering.is_some(),
            pending_hdr_cll: primed_cll_received,
            metadata_started: false,
            metadata_ended: false,
            primed,
            primed_cll_received,
            pending_active_parameters: primed_active_parameters.clone(),
            primed_active_parameters,
            active_parameters: None,
            cvs_raw_generation: 0,
            cvs_no_update: false,
            length: config.length_size,
            budget,
            failed: false,
            allow_incomplete_parameters,
        })
    }
    /// Current active or first available complete parameter pair.
    pub fn try_parameters(&self) -> Option<(&Sps, &Pps)> {
        self.active_pps.and_then(|id| self.params.pairs.iter().find(|(_, p)| p.id == id))
            .or_else(|| self.params.pairs.first()).map(|(s, p)| (s, p))
    }
    /// Complete parameters for a configured/decoded picture.
    ///
    /// # Panics
    /// If an in-band decoder has not received a complete parameter pair.
    pub fn parameters(&self) -> (&Sps, &Pps) {
        self.try_parameters().expect("HEVC parameter pair is not available yet")
    }
    pub fn reset(&mut self) {
        self.params.clone_from(&self.initial_params);
        self.decoded_sps = None;
        self.decoded_vps = None;
        self.references.clear();
        self.previous_poc = None;
        self.reset_periods = Default::default();
        self.suppress_rasl = false;
        self.sequence_ended = false;
        self.active_pps = None;
        self.hdr = self.primed;
        self.cll_received = self.primed_cll_received;
        self.pending_hdr_mastering = self.primed.mastering.is_some();
        self.pending_hdr_cll = self.primed_cll_received;
        self.metadata_started = false;
        self.metadata_ended = false;
        self.pending_active_parameters.clone_from(&self.primed_active_parameters);
        self.active_parameters = None;
        self.cvs_raw_generation = 0;
        self.cvs_no_update = false;
        self.failed = false;
    }
    /// The static HDR light the stream stated of itself: the SEI messages its
    /// configuration record carried, plus every one the decoder has walked since
    /// it was last flushed. A reader that grades a picture for a panel asks the
    /// coding as well as the file, because an encoder that wrote its mastering
    /// volume in-band often wrote it nowhere else — and a caller that grades at
    /// open, before a packet has been decoded, only ever sees the half the
    /// configuration record states.
    pub fn hdr(&self) -> HdrMetadata {
        self.hdr
    }
    /// Opening fallback fills absent metadata, but never overrides a received
    /// CLL statement merely because its explicitly transmitted values are zero.
    pub fn hdr_with_fallback(&self, fallback: HdrMetadata) -> HdrMetadata {
        let mut hdr = self.hdr.filled_with(fallback);
        if self.cll_received {
            hdr.light = self.hdr.light;
        }
        hdr
    }
    /// Last validated base-layer active-parameter declaration in this CVS.
    pub fn active_parameter_sets(&self) -> Option<&hevc_sei::ActiveParameterSets> {
        self.active_parameters.as_ref()
    }

    /// Inspect in-band SEI NAL units in a packet to update HDR metadata without full picture decoding.
    pub fn observe_packet(&mut self, packet: &[u8]) {
        let mut first_vcl = None;
        let mut end_after_picture = false;
        if let Ok(units) = NalUnits::new(packet, self.length) {
            for nal in units {
                let Ok(nal) = nal else { break };
                let Ok(header) = NalHeader::parse(nal) else { continue };
                if header.layer_id != 0 { continue; }
                if header.is_vcl() {
                    first_vcl.get_or_insert(header);
                } else if matches!(header.unit_type, 36 | 37) {
                    if first_vcl.is_some() { end_after_picture = true; }
                    else { self.metadata_ended = true; }
                }
                if hevc_sei::is_sei_unit(header.unit_type) {
                    let mut received_hdr = HdrMetadata::default();
                    if let Ok(received) = hevc_sei::update_hdr_from_nal(&mut received_hdr, nal, self.budget) {
                        self.hdr.merge(received_hdr);
                        if received { self.hdr.light = received_hdr.light; }
                        self.cll_received |= received;
                        self.pending_hdr_cll |= received;
                        self.pending_hdr_mastering |= received_hdr.mastering.is_some();
                    }
                }
            }
        }
        if let Some(header) = first_vcl {
            // Header-only seek priming follows the same CLVS lifetime as pixel
            // decoding. Prefix guidance may arrive in an earlier empty sample.
            if self.metadata_started && (self.metadata_ended || header.is_idr()
                || matches!(header.unit_type, 16..=18)) {
                if !self.pending_hdr_mastering { self.hdr.mastering = None; }
                if !self.pending_hdr_cll {
                    self.hdr.light = Default::default();
                    self.cll_received = false;
                }
            }
            self.metadata_started = true;
            self.metadata_ended = end_after_picture;
            self.pending_hdr_mastering = false;
            self.pending_hdr_cll = false;
        }
    }
    // Validate parameter updates before committing them to decoder state.
    fn updated_pairs(&self, packet: &[u8]) -> Result<Option<ParameterSets>> {
        if packet.len() > self.budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        let mut updated: Option<ParameterSets> = None;
        let mut seen_slice = false;
        let mut first_vcl = None;
        let mut prefix_end = false;
        let mut raw_updates = Vec::new();
        let mut raw_overwrite = false;
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let header = NalHeader::parse(nal)?;
            header.require_base_layer()?;
            if header.is_vcl() && first_vcl.is_none() {first_vcl=Some(header.unit_type);}
            if !seen_slice && matches!(header.unit_type,36|37) {prefix_end=true;}
            seen_slice |= header.is_vcl();
            let params = updated.as_ref().unwrap_or(&self.params);
            match header.unit_type {
                32 => {
                    let new = Vps::parse(nal, self.budget)?;
                    raw_overwrite |= self.record_raw(&mut raw_updates,updated.as_ref(),32,new.id,nal)?;
                    let previous = params.vps.iter().find(|v| v.id == new.id);
                    if previous.is_some_and(|v| *v == new && v.same_reset_semantics(&new)) {
                        continue;
                    }
                    // Preserve latest deferred Annex F result even when Annex A ignores
                    // the tail: a later PPS may activate reset semantics. Inert
                    // reserved/future bits do not make a semantic parameter change.
                    let semantic_change = previous.is_none_or(|v| *v != new)
                        || params.pairs.iter().any(|(s, p)| s.vps_id == new.id && (p.poc_reset_info_present
                            || p.slice_header_extension && (p.multilayer_extension || new.has_poc_extension()
                                || previous.is_some_and(Vps::has_poc_extension))));
                    if seen_slice && semantic_change {
                        return Err(invalid("HEVC changed VPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.vps.iter_mut().find(|v| v.id == new.id) {
                        *old = new;
                    } else {
                        params.vps.push(new);
                    }
                }
                33 => {
                    let new = Sps::parse(nal, self.budget)?;
                    raw_overwrite |= self.record_raw(&mut raw_updates,updated.as_ref(),33,new.id,nal)?;
                    if params.sets.iter().any(|s| *s == new) {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("HEVC changed SPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.sets.iter_mut().find(|s| s.id == new.id) {
                        *old = new;
                    } else {
                        params.sets.push(new);
                    }
                }
                34 => {
                    let (id, _) = Pps::identifiers(nal, self.budget)?;
                    // A replacement SPS with the same ID may follow this PPS
                    // in the prefix. Validate contextual PPS syntax against
                    // the completed staged bank below, before any picture is
                    // decoded or parameter state is committed.
                    if params.pps_nals.iter().any(|(old, bytes)| *old == id && bytes == nal) { continue; }
                    raw_overwrite |= params.pps_nals.iter().any(|(old,_)| *old == id);
                    if self.allow_incomplete_parameters {
                        let candidate = updated.as_ref().unwrap_or(&self.params);
                        let extra = self.inband_pps_storage(updated.as_ref())?.checked_add(nal.len())
                            .and_then(|n| n.checked_add(std::mem::size_of::<(u8, Vec<u8>)>()))
                            .ok_or_else(|| invalid("HEVC received PPS storage overflow"))?;
                        Self::raw_ownership(&[&self.params.raw_parameters,&self.initial_params.raw_parameters,
                            &candidate.raw_parameters,&raw_updates],extra,self.budget)?;
                    }
                    if seen_slice {
                        return Err(invalid("HEVC changed PPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.pps_nals.iter_mut().find(|(old, _)| *old == id) {
                        old.1 = nal.to_vec();
                    } else {
                        params.pps_nals.push((id, nal.to_vec()));
                    }
                }
                _ => {}
            }
        }
        let begins_cvs = self.sequence_ended || self.previous_poc.is_none() || prefix_end
            || first_vcl.is_some_and(|kind|matches!(kind,16..=20));
        if raw_overwrite && !begins_cvs
            && self.cvs_no_update {
            return Err(invalid("HEVC active parameter SEI forbids parameter updates"));
        }
        if raw_overwrite {
            let params=updated.get_or_insert_with(||self.params.clone());
            params.raw_generation=params.raw_generation.checked_add(1)
                .ok_or_else(||invalid("HEVC raw parameter generation overflow"))?;
        }
        if !raw_updates.is_empty() {
            let params=updated.get_or_insert_with(||self.params.clone());
            for new in raw_updates {
                if let Some(old)=params.raw_parameters.iter_mut().find(|p|p.0==new.0 && p.1==new.1){*old=new;}else{params.raw_parameters.push(new);}
            }
        }
        if let Some(params)=&updated {
            Self::raw_ownership(&[&self.params.raw_parameters,&self.initial_params.raw_parameters,
                &params.raw_parameters],self.inband_pps_storage(Some(params))?,self.budget)?;
        }
        if let Some(params) = &mut updated {
            let mut pairs = Vec::new();
            for (_, nal) in &params.pps_nals {
                let sps_id = Pps::referenced_sps_id(nal, self.budget)?;
                if self.allow_incomplete_parameters && !params.sets.iter().any(|s| s.id == sps_id) { continue; }
                let (sps, pps) = bind_pps(nal, &params.sets, self.budget,
                    "HEVC updated PPS has no valid SPS")?;
                pairs.push((sps.clone(), pps));
            }
            params.pairs = pairs;
        }
        Ok(updated)
    }
    /// Restore in-band VPS/SPS/PPS and received static HDR before a seek entry point. Call
    /// only after reset, before any picture; previous picture data is not decoded.
    pub fn prime_parameters(&mut self, packet: &[u8]) -> Result<()> {
        if self.failed { return Err(invalid("HEVC decoder requires reset after error")); }
        if self.decoded_sps.is_some() || self.previous_poc.is_some() {
            return Err(invalid("HEVC parameter priming requires a reset decoder"));
        }
        match self.updated_pairs(packet) {
            Ok(params) => {
                if let Some(params) = params { self.params = params; }
                // Reset discarded received SEI too. Recover it only after the
                // complete packet's parameter/header validation succeeded, and
                // without decoding or retaining any preceding picture.
                self.observe_packet(packet);
                Ok(())
            }
            Err(error) => { self.failed = true; Err(error) }
        }
    }
    /// Parse all slices, including prefix PPS updates, without changing state.
    pub fn slice_headers(&self, packet: &[u8]) -> Result<Vec<SliceHeader>> {
        if packet.len() > self.budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        let updated = self.updated_pairs(packet)?;
        let pairs = &updated.as_ref().unwrap_or(&self.params).pairs;
        let mut headers: Vec<SliceHeader> = Vec::new();
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let kind = NalHeader::parse(nal)?;
            kind.require_base_layer()?;
            if !kind.is_vcl() {
                continue;
            }
            let id = SliceHeader::parameter_set_id(nal)?;
            let (sps, pps) = pairs
                .iter()
                .find(|(_, p)| p.id == id)
                .ok_or_else(|| invalid("HEVC slice references unknown PPS"))?;
            let header =
                SliceHeader::parse_with_previous(nal, sps, pps, self.budget, headers.last())?;
            if header.nal.temporal_id as usize >= sps.ordering.len() {
                return Err(invalid("HEVC slice exceeds SPS temporal layers"));
            }
            if let Some(first) = headers.first() {
                // Independent colour planes have independent slice-address
                // spaces. A first slice in another plane is part of this AU.
                let previous = if sps.separate_colour_plane {
                    headers
                        .iter()
                        .rev()
                        .find(|h| h.colour_plane == header.colour_plane)
                } else {
                    headers.last()
                };
                let order = |address| {
                    if let Some(tiles) = &pps.tiles {
                        let side = 1u32 << sps.coding_block_log2[1];
                        super::hevc_tiles::tile_scan_address(
                            tiles,
                            sps.dimensions.map(|v| v.div_ceil(side)),
                            address,
                        )
                    } else {
                        Ok(address)
                    }
                };
                if let Some(previous) = previous {
                    if header.first || order(header.address)? <= order(previous.address)? {
                        return Err(invalid(
                            "HEVC slice addresses must increase within one picture",
                        ));
                    }
                } else if !header.first || header.address != 0 {
                    return Err(invalid("HEVC colour plane must begin with the first slice"));
                }
                if header.pps_id != first.pps_id
                    || header.poc_lsb != first.poc_lsb
                    || header.nal.unit_type != first.nal.unit_type
                    || header.nal.temporal_id != first.nal.temporal_id
                    || header.picture_output != first.picture_output
                {
                    return Err(invalid("HEVC slices disagree on picture identity"));
                }
            } else if !header.first || header.address != 0 {
                return Err(invalid("HEVC access unit must begin with the first slice"));
            }
            headers.push(header);
        }
        if let Some(first) = headers.first() {
            let (sps, _) = pairs.iter().find(|(_, p)| p.id == first.pps_id).unwrap();
            if sps.separate_colour_plane
                && (0..3).any(|plane| !headers.iter().any(|h| h.colour_plane == plane))
            {
                return Err(invalid("HEVC access unit is missing a colour plane"));
            }
        }
        Ok(headers)
    }
    pub fn decode_packet(&mut self, packet: &[u8]) -> Result<Option<Decoded>> {
        if self.failed {
            return Err(invalid("HEVC decoder requires reset after error"));
        }
        let result = self.decode(packet);
        self.failed = result.is_err();
        result
    }
    fn decode(&mut self, packet: &[u8]) -> Result<Option<Decoded>> {
        if let Some(params) = self.updated_pairs(packet)? {
            self.params = params;
        }
        let headers = self.slice_headers(packet)?;
        self.observe_packet(packet);
        let mut slice = None;
        let mut end_after_picture = false;
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let header = NalHeader::parse(nal)?;
            header.require_base_layer()?;
            if header.is_vcl() {
                if slice.is_none() {
                    slice = Some(nal);
                }
            } else if matches!(header.unit_type, 36 | 37) {
                if header.temporal_id != 0 {
                    return Err(invalid("HEVC EOS/EOB requires temporal layer zero"));
                }
                if NalRbsp::parse(nal, self.budget)?.bytes.as_slice() != [128] {
                    return Err(invalid("invalid HEVC EOS/EOB trailing bits"));
                }
                if slice.is_some() {
                    // A suffix closes the current picture's sequence only
                    // after that picture has used its existing DPB/POC state.
                    end_after_picture = true;
                } else {
                    self.sequence_ended = true;
                }
            } else if matches!(header.unit_type, 32..=34) {
                // Parameter sets were validated before parsing slices.
            } else if hevc_sei::is_sei_unit(header.unit_type) {
                if let Ok(Some(mut active)) = hevc_sei::active_parameters_from_nal(nal, self.budget) {
                    if slice.is_some() {
                        return Err(invalid("HEVC active parameter SEI follows picture slices"));
                    }
                    // Multiple declarations can precede the same picture,
                    // including in parameter-only packets. A later zero does
                    // not revoke an earlier no-update promise.
                    active.no_parameter_set_update |= self.pending_active_parameters
                        .as_ref().is_some_and(|p| p.no_parameter_set_update);
                    self.pending_active_parameters = Some(active);
                }
            } else {
                // AUD/filler and reserved/unspecified non-VCL units
                // do not contribute slices or change the decoded base picture.
                // Header validity and base-layer admission were checked above.
            }
        }
        let Some(nal) = slice else { return Ok(None) };
        let id = SliceHeader::parameter_set_id(nal)?;
        let (sps, pps) = self
            .params
            .pairs
            .iter()
            .find(|(_, p)| p.id == id)
            .ok_or_else(|| invalid("HEVC slice references unknown PPS"))?;
        let header = &headers[0];
        let vps = self.params.vps.iter().find(|v| v.id == sps.vps_id);
        if vps.is_none() {
            // F.10.2/F.11.1 independent-layer output deliberately has no VPS.
            // An existing mismatched VPS is still an error; omission is not a
            // fallback for a received, malformed or otherwise unrelated VPS.
            if !self.params.vps.is_empty() {
                return Err(invalid("HEVC SPS references unavailable VPS"));
            }
            let profile = sps.profile.sub_layers.get(header.nal.temporal_id as usize)
                .and_then(|p|p.profile).or(sps.profile.profile);
            if !profile.is_some_and(|p|p.is_annex_a_compatible()) {
                return Err(invalid("VPS-less HEVC requires a self-contained Annex A SPS"));
            }
        }
        if let Some(vps) = vps {
            if sps.ordering.len() > vps.ordering.len() {
                return Err(invalid("HEVC SPS exceeds VPS temporal sublayers"));
            }
            if vps.temporal_id_nesting && !sps.temporal_id_nesting {
                return Err(invalid("HEVC SPS contradicts VPS temporal nesting"));
            }
        }
        let poc_extension_active = pps.poc_reset_info_present
            || pps.slice_header_extension && (pps.multilayer_extension || vps.is_some_and(Vps::has_poc_extension));
        let reset_syntax = if poc_extension_active {
            let lsb_aligned = vps.ok_or_else(|| invalid("HEVC POC-reset syntax requires an active VPS"))?.poc_lsb_aligned()?;
            let parse = |h: &SliceHeader| {
                if pps.slice_header_extension {
                    super::hevc_poc::ResetSyntax::parse_base(&h.extension, pps.poc_reset_info_present, sps.poc_bits, h.nal, lsb_aligned)
                } else { Ok(super::hevc_poc::ResetSyntax::default()) }
            };
            let first = parse(header)?;
            for other in headers.iter().skip(1) {
                if parse(other)? != first {
                    return Err(invalid("HEVC slices disagree on POC reset syntax"));
                }
            }
            first
        } else { super::hevc_poc::ResetSyntax::default() };
        let new_sequence = self.sequence_ended;
        let begins_cvs = new_sequence || self.previous_poc.is_none()
            || header.nal.is_idr() || matches!(header.nal.unit_type, 16..=18);
        self.sequence_ended = end_after_picture;
        if begins_cvs {
            self.cvs_raw_generation = self.params.raw_generation;
            self.cvs_no_update = false;
        }
        if let Some(active) = self.pending_active_parameters.take() {
            if active.no_parameter_set_update && self.params.raw_generation != self.cvs_raw_generation {
                return Err(invalid("HEVC active parameter SEI forbids parameter updates"));
            }
            self.cvs_no_update |= active.no_parameter_set_update;
            if active.vps_id != sps.vps_id || active.sps_ids[0] != sps.id {
                return Err(invalid("HEVC active parameter SEI disagrees with picture binding"));
            }
            if !begins_cvs && (self.decoded_vps.as_ref() != vps
                || self.decoded_sps.as_ref() != Some(sps)) {
                return Err(invalid("HEVC active parameter SEI requires new sequence"));
            }
            self.active_parameters = Some(active);
        } else if begins_cvs {
            self.active_parameters = None;
        }
        if new_sequence {
            self.references.clear();
            self.previous_poc = None;
            self.reset_periods = Default::default();
            self.suppress_rasl = false;
        }
        if header.nal.temporal_id as usize >= sps.ordering.len() {
            return Err(invalid("HEVC slice exceeds SPS temporal layers"));
        }
        // No-update guidance remains about parameter identity, including
        // known metadata unused by this base layer, not reconstruction equality.
        if !begins_cvs
            && self.cvs_no_update
            && self.decoded_sps.as_ref().is_some_and(|previous| previous != sps)
        {
            return Err(invalid("HEVC active parameter SEI forbids parameter updates"));
        }
        if self
            .decoded_sps
            .as_ref()
            .is_some_and(|previous| !previous.same_base_decoding_parameters(sps))
        {
            if !header.nal.is_irap() {
                return Err(invalid("HEVC SPS change requires a random-access picture"));
            }
            self.references.clear();
            self.previous_poc = None;
            self.reset_periods = Default::default();
            self.suppress_rasl = false;
        }
        let reset_alignment_changed = if poc_extension_active
            || pps.slice_header_extension && self.decoded_vps.as_ref().is_some_and(Vps::has_poc_extension)
        {
            match (self.decoded_vps.as_ref(), vps) {
                // A new CVS can activate Annex F after an opaque Annex A tail.
                // Only the newly active VPS needs strict interpretation then.
                (Some(previous), Some(vps)) if begins_cvs => !previous.same_reset_semantics(vps),
                (Some(previous), Some(vps)) => previous.poc_lsb_aligned()? != vps.poc_lsb_aligned()?,
                (Some(_), None) => true,
                _ => false,
            }
        } else { false };
        if reset_alignment_changed || self.decoded_sps.is_some() && self.decoded_vps.as_ref() != vps {
            if !begins_cvs {
                return Err(invalid("HEVC active VPS changed within sequence"));
            }
            self.references.clear();
            self.previous_poc = None;
            self.reset_periods = Default::default();
            self.suppress_rasl = false;
        }
        self.decoded_vps = vps.cloned();
        if self.decoded_sps.as_ref() != Some(sps) {
            self.decoded_sps = Some(sps.clone());
        }
        if header.nal.is_irap() {
            self.suppress_rasl = self.previous_poc.is_none()
                || header.nal.is_idr()
                || matches!(header.nal.unit_type, 16..=18);
        }
        if self.suppress_rasl && matches!(header.nal.unit_type, 8 | 9) {
            return Ok(None);
        }
        let (next_periods, resetting) = self.reset_periods.advance(reset_syntax, header.nal, header.poc_lsb)?;
        let (poc, decrement) = if resetting {
            super::hevc_poc::derive_reset(sps.poc_bits, header.poc_lsb, self.previous_poc,
                reset_syntax.reset.ok_or_else(|| invalid("missing HEVC reset operation"))?, reset_syntax.msb_cycle)?
        } else if let Some(cycle) = reset_syntax.msb_cycle {
            let poc = i64::from(cycle) * (1i64 << sps.poc_bits) + i64::from(header.poc_lsb);
            (i32::try_from(poc).map_err(|_| invalid("HEVC explicit cycle POC overflow"))?, 0)
        } else {
            (super::hevc_poc::derive(sps, header.nal, header.poc_lsb, self.previous_poc)?, 0)
        };
        if header.nal.is_irap() && self.suppress_rasl {
            // An IRAP starting a CVS marks every prior DPB reference unused.
            // BLA carries RPS syntax, but that syntax cannot retain the old CVS.
            self.references.clear();
        }
        if decrement != 0 && !self.references.is_empty() {
            let metadata = self.references.iter().try_fold(
                self.references.len().checked_mul(std::mem::size_of::<Reference>())
                    .ok_or_else(|| invalid("HEVC POC translation size overflow"))?,
                |sum, r| sum.checked_add(r.picture.as_ref()?.poc_translation_bytes()?),
            ).ok_or_else(|| invalid("HEVC POC translation size overflow"))?;
            let retained = self.references.iter().try_fold(0usize,
                |sum, r| sum.checked_add(r.picture.as_ref()?.storage_bytes()?),
            ).ok_or_else(|| invalid("HEVC retained POC translation size overflow"))?;
            if retained.checked_add(metadata).is_none_or(|n| n > self.budget) {
                return Err(invalid("HEVC POC translation exceeds decode budget"));
            }
            let mut translated = Vec::new();
            translated.try_reserve_exact(self.references.len())
                .map_err(|_| invalid("cannot allocate HEVC translated references"))?;
            for reference in &self.references {
                let poc = i64::from(reference.poc).checked_sub(decrement)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| invalid("HEVC translated DPB POC overflow"))?;
                let source = reference.picture.as_ref().ok_or_else(|| invalid("HEVC retained reference has no picture"))?;
                translated.push(Reference { layer_id: reference.layer_id, poc, long_term: reference.long_term,
                    picture: Some(Arc::new(source.translated_pocs(decrement, metadata)?)) });
            }
            self.references = translated;
        }
        let (retained, lists) = reference_lists(&header, poc, &self.references, sps.poc_bits)?;
        let mut slice_lists = vec![lists];
        for other in headers.iter().skip(1) {
            let (other_retained, other_lists) =
                reference_lists(other, poc, &self.references, sps.poc_bits)?;
            if other_retained != retained {
                return Err(invalid("HEVC slices disagree on reference picture set"));
            }
            slice_lists.push(other_lists);
        }
        self.references
            .retain(|r| retained.iter().any(|&(poc, _)| poc == r.poc));
        for reference in &mut self.references {
            reference.long_term = retained
                .iter()
                .find(|&&(poc, _)| poc == reference.poc)
                .unwrap()
                .1;
        }
        let retained_bytes = self
            .references
            .iter()
            .try_fold(0usize, |total, r| {
                total.checked_add(r.picture.as_deref()?.storage_bytes()?)
            })
            .ok_or_else(|| invalid("HEVC reference storage size overflow"))?;
        let budget = self
            .budget
            .checked_sub(retained_bytes)
            .ok_or_else(|| invalid("HEVC references exceed decode budget"))?;
        let picture = Arc::new(hevc_picture::decode_slices(
            sps,
            pps,
            &headers,
            poc,
            &slice_lists,
            budget,
        )?);
        if super::hevc_poc::updates_previous(header.nal) {
            self.previous_poc = Some(poc);
        } else if resetting {
            if let Some(super::hevc_poc::Reset::Signalled { full, lsb }) = reset_syntax.reset {
                self.previous_poc = Some(if full { 0 } else { lsb as i32 });
            }
        }
        self.reset_periods = next_periods;
        if header.nal.unit_type >= 16 || header.nal.unit_type & 1 != 0 {
            self.references.push(Reference { layer_id: header.nal.layer_id,
                long_term: false,
                poc,
                picture: Some(Arc::clone(&picture)),
            });
            let capacity = sps
                .ordering
                .last()
                .ok_or_else(|| invalid("HEVC SPS has no DPB ordering"))?
                .max_decoded_pictures as usize;
            if self.references.len() > capacity {
                return Err(invalid("HEVC decoded reference count exceeds SPS"));
            }
        }
        self.active_pps = Some(header.pps_id);
        Ok(Some(Decoded {
            picture,
            poc,
            output: header.picture_output,
        }))
    }
}

// Resolve RPS before list selection so layered callers can insert same-AU
// inter-layer candidates even when both temporal lists are empty.
pub(super) fn temporal_reference_groups(
    header: &SliceHeader,
    poc: i32,
    references: &[Reference],
    poc_bits: u8,
) -> Result<(Vec<(i32, bool)>, [Vec<Reference>; 3])> {
    let mut retained = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for r in &header.short_term {
        let target = poc
            .checked_add(r.delta_poc)
            .ok_or_else(|| invalid("HEVC reference POC overflow"))?;
        if references.iter().any(|r| r.layer_id == header.nal.layer_id && r.poc == target && r.long_term) {
            return Err(invalid("HEVC RPS uses a long-term picture as short-term"));
        }
        retained.push((target, false));
        if r.used {
            let mut reference = references
                .iter()
                .find(|r| r.layer_id == header.nal.layer_id && r.poc == target)
                .ok_or_else(|| {
                    invalid(&format!(
                        "HEVC reference POC {target} is missing for POC {poc} (NAL {})",
                        header.nal.unit_type
                    ))
                })?
                .clone();
            reference.long_term = false;
            if r.delta_poc < 0 {
                before.push(reference);
            } else {
                after.push(reference);
            }
        }
    }
    let dpb_pocs: Vec<_> = references.iter().filter(|r|r.layer_id==header.nal.layer_id).map(|r| r.poc).collect();
    let mut long = Vec::new();
    for entry in &header.long_term {
        if let Some(target) = entry.resolve(poc, poc_bits, &dpb_pocs)? {
            if target == poc || retained.iter().any(|&(poc, _)| poc == target) {
                return Err(invalid("HEVC long-term RPS repeats a reference POC"));
            }
            retained.push((target, true));
            if entry.used {
                let mut reference = references
                    .iter()
                    .find(|r| r.layer_id == header.nal.layer_id && r.poc == target)
                    .ok_or_else(|| invalid("HEVC long-term current reference is missing"))?
                    .clone();
                reference.long_term = true;
                long.push(reference);
            }
        }
    }
    Ok((retained, [before, after, long]))
}

fn reference_lists(
    header: &SliceHeader,
    poc: i32,
    references: &[Reference],
    poc_bits: u8,
) -> Result<(Vec<(i32, bool)>, [Vec<Reference>; 2])> {
    let (retained, [before, after, long]) = temporal_reference_groups(header, poc, references, poc_bits)?;
    let mut lists: [Vec<Reference>; 2] = [Vec::new(), Vec::new()];
    for list in 0..2 {
        let base: Vec<_> = if list == 0 {
            before.iter().chain(&after).chain(&long)
        } else {
            after.iter().chain(&before).chain(&long)
        }
        .cloned()
        .collect();
        let selected = super::hevc_reference_list::select(
            base.len(),
            header.current_picture_reference,
            header.references[list] as usize,
            header.list_modification[list].as_deref(),
            list,
        )?;
        for entry in selected {
            match entry {
                super::hevc_reference_list::Source::Decoded(index) => {
                    lists[list].push(base[index].clone())
                }
                super::hevc_reference_list::Source::Current => {
                    lists[list].push(Reference { layer_id: header.nal.layer_id,
                        poc,
                        long_term: true,
                        picture: None,
                    });
                }
            }
        }
    }

    Ok((retained, lists))
}

#[cfg(test)]
mod long_term_list_tests {
    #[test]
    fn seek_parameter_priming_keeps_picture_state_empty_and_restores_late_idr() {
        use super::*;
        let data = include_bytes!("../../tests/fixtures/playback-errors/hevc-poc-vps-alignment-update-annex-a-to-reset-at-idr-synthetic.mp4");
        let mut input = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
        let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        for index in 0..33 {
            input.read_packet(0, index, &mut packet).unwrap();
            decoder.prime_parameters(&packet).unwrap();
            assert!(decoder.references.is_empty());
            assert!(decoder.previous_poc.is_none() && decoder.decoded_sps.is_none() && decoder.decoded_vps.is_none());
        }
        assert!(!decoder.params.vps[0].poc_lsb_aligned().unwrap());
        assert!(decoder.params.pairs[0].1.poc_reset_info_present);
        input.read_packet(0, 33, &mut packet).unwrap();
        let frame = decoder.decode_packet(&packet).unwrap().unwrap();
        assert_eq!(frame.poc, 0);
        assert_eq!(decoder.prime_parameters(&packet).unwrap_err().to_string(), "HEVC parameter priming requires a reset decoder");
        assert!(!decoder.failed);
        decoder.reset();
        assert!(!decoder.params.pairs[0].1.poc_reset_info_present);
        assert!(decoder.params.vps[0].poc_lsb_aligned().is_err());
        let invalid = include_bytes!("../../tests/fixtures/playback-errors/hevc-future-extension-vps-bad-stop-synthetic.mp4");
        let bad_movie = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(invalid), Default::default()).unwrap();
        let config = HevcConfig::parse(&bad_movie.tracks()[0].configuration).unwrap();
        let nal = config.arrays.iter().find(|a| a.nal_type == 32).unwrap().units[0];
        let mut bad_packet = (nal.len() as u32).to_be_bytes().to_vec();
        bad_packet.extend_from_slice(nal);
        assert!(decoder.prime_parameters(&bad_packet).is_err());
        assert!(decoder.failed);
        assert!(decoder.prime_parameters(&[]).is_err());
        decoder.reset();
        assert!(!decoder.failed);
    }
    #[test]
    fn annex_a_ignored_vps_tail_is_retained_for_later_annex_f_activation() {
        use super::*;
        let data = include_bytes!("../../tests/fixtures/playback-errors/hevc-poc-vps-alignment-update-annex-a-after-slices-synthetic.mp4");
        let mut input = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default()).unwrap();
        let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20).unwrap();
        assert!(decoder.params.vps[0].poc_lsb_aligned().unwrap());
        let mut packet = Vec::new();
        for index in 0..35 {
            input.read_packet(0, index, &mut packet).unwrap();
            assert!(decoder.decode_packet(&packet).unwrap().is_some());
            assert_eq!(decoder.params.vps[0].poc_lsb_aligned().unwrap(), index < 16);
        }
        decoder.reset();
        assert!(decoder.params.vps[0].poc_lsb_aligned().unwrap());
    }
    use super::super::{hevc_long_term::LongTermReference, hevc_rps::ShortTermReference};
    use super::*;
    #[test]
    fn bla_starts_with_no_prior_reference_pictures() {
        let file = include_bytes!("../../tests/fixtures/playback-errors/hevc-bla-w-lp-synthetic.mp4");
        let mut input = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(file), Default::default()).unwrap();
        let mut decoder = HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
        let mut packet = vec![];
        for index in 0..=5 {
            input.read_packet(0, index, &mut packet).unwrap();
            assert!(decoder.decode_packet(&packet).unwrap().is_some());
        }
        assert!(!decoder.references.is_empty());
        let bla_poc = decoder.previous_poc.unwrap();
        assert!(decoder.references.iter().all(|r| r.poc >= bla_poc),
            "BLA retained prior-sequence POCs: {:?}", decoder.references.iter().map(|r| r.poc).collect::<Vec<_>>());
    }
    #[test]
    fn mixed_reference_lists_append_long_term_pictures_and_apply_modifications() {
        let data = include_bytes!("../../tests/fixtures/playback-errors/shared-hevc-main.mp4");
        let mut input =
            crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(data), Default::default())
                .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let picture = decoder.decode_packet(&packet).unwrap().unwrap().picture;
        let (sps, pps) = decoder.parameters();
        let nal = NalUnits::new(&packet, decoder.length)
            .unwrap()
            .map(|n| n.unwrap())
            .find(|n| NalHeader::parse(n).unwrap().is_vcl())
            .unwrap();
        let mut header = SliceHeader::parse(nal, sps, pps, 16 << 20).unwrap();
        header.short_term = vec![
            ShortTermReference {
                delta_poc: -1,
                used: true,
            },
            ShortTermReference {
                delta_poc: 1,
                used: true,
            },
        ];
        header.long_term = vec![LongTermReference {
            poc_lsb: 3,
            used: true,
            msb_cycles: Some(1),
        }];
        header.references = [3, 3];
        let references: Vec<_> = [34, 36, 19]
            .into_iter()
            .map(|poc| Reference { layer_id: 0,
                poc,
                long_term: false,
                picture: Some(Arc::clone(&picture)),
            })
            .collect();
        let (retained, lists) = reference_lists(&header, 35, &references, 4).unwrap();
        assert_eq!(retained, [(34, false), (36, false), (19, true)]);
        assert_eq!(
            lists[0].iter().map(|r| r.poc).collect::<Vec<_>>(),
            [34, 36, 19]
        );
        assert_eq!(
            lists[1].iter().map(|r| r.poc).collect::<Vec<_>>(),
            [36, 34, 19]
        );
        assert_eq!(
            lists[0].iter().map(|r| r.long_term).collect::<Vec<_>>(),
            [false, false, true]
        );
        header.list_modification[0] = Some(vec![2, 0, 1]);
        let (_, lists) = reference_lists(&header, 35, &references, 4).unwrap();
        assert_eq!(
            lists[0].iter().map(|r| r.poc).collect::<Vec<_>>(),
            [19, 34, 36]
        );
        assert!(reference_lists(&header, 35, &references[..2], 4).is_err());
        header.long_term[0] = LongTermReference {
            poc_lsb: 2,
            used: true,
            msb_cycles: Some(0),
        };
        assert!(
            reference_lists(&header, 35, &references, 4)
                .err()
                .unwrap()
                .to_string()
                .contains("repeats")
        );
        // The same retained POC with a different classification is a different RPS.
        header.short_term = vec![ShortTermReference {
            delta_poc: -1,
            used: true,
        }];
        header.long_term.clear();
        header.references = [1, 1];
        header.list_modification = [None, None];
        let (short_set, _) = reference_lists(&header, 35, &references, 4).unwrap();
        header.short_term.clear();
        header.long_term = vec![LongTermReference {
            poc_lsb: 2,
            used: true,
            msb_cycles: Some(0),
        }];
        let (long_set, _) = reference_lists(&header, 35, &references, 4).unwrap();
        assert_ne!(
            short_set, long_set,
            "slices cannot change a retained picture's classification"
        );
    }
}
