//! Stateful native HEVC access-unit decoding and short-term reference storage.
use super::{
    config::{HevcConfig, NalUnits},
    hevc_motion::Reference,
    hevc_nal::NalHeader,
    hevc_picture::{self, Picture},
    hevc_pps::Pps,
    hevc_sei,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
};
use crate::color::hdr::HdrMetadata;
use crate::{Result, invalid};
use std::sync::Arc;
pub struct Decoded {
    pub picture: Arc<Picture>,
    pub poc: i32,
    pub output: bool,
}
pub struct HevcDecoder {
    pairs: Vec<(Sps, Pps)>,
    references: Vec<Reference>,
    previous_poc: Option<i32>,
    suppress_rasl: bool,
    active_pps: Option<u8>,
    hdr: HdrMetadata,
    primed: HdrMetadata,
    length: u8,
    budget: usize,
    failed: bool,
}
impl HevcDecoder {
    pub fn from_configuration(data: &[u8], budget: usize) -> Result<Self> {
        let config = HevcConfig::parse(data)?;
        let mut sets = Vec::new();
        for array in &config.arrays {
            if array.nal_type == 33 {
                for nal in &array.units {
                    let sps = Sps::parse(nal, budget)?;
                    if sets.iter().any(|s: &Sps| s.id == sps.id) {
                        return Err(invalid("duplicate HEVC SPS"));
                    }
                    sets.push(sps);
                }
            }
        }
        let mut pairs = Vec::new();
        for array in &config.arrays {
            if array.nal_type == 34 {
                for nal in &array.units {
                    let pair = sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s, budget).ok().map(|p| (s.clone(), p)))
                        .ok_or_else(|| invalid("HEVC PPS has no valid SPS"))?;
                    if pairs.iter().any(|(_, p): &(Sps, Pps)| p.id == pair.1.id) {
                        return Err(invalid("duplicate HEVC PPS"));
                    }
                    pairs.push(pair);
                }
            }
        }
        if pairs.is_empty() {
            return Err(invalid("HEVC configuration has no parameter-set pair"));
        }
        let mut primed = HdrMetadata::default();
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
                if let Ok(Some(hdr)) = hevc_sei::hdr_from_nal(nal, budget) {
                    primed.merge(hdr);
                }
            }
        }
        Ok(Self {
            pairs,
            references: Vec::new(),
            previous_poc: None,
            suppress_rasl: false,
            active_pps: None,
            hdr: primed,
            primed,
            length: config.length_size,
            budget,
            failed: false,
        })
    }
    pub fn parameters(&self) -> (&Sps, &Pps) {
        let (s, p) = self
            .active_pps
            .and_then(|id| self.pairs.iter().find(|(_, p)| p.id == id))
            .unwrap_or(&self.pairs[0]);
        (s, p)
    }
    pub fn reset(&mut self) {
        self.references.clear();
        self.previous_poc = None;
        self.suppress_rasl = false;
        self.active_pps = None;
        self.hdr = self.primed;
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
    /// Parse all independent slices in one picture without changing reference
    /// state. Reconstruction can use these ranges without reparsing headers.
    pub fn slice_headers(&self, packet: &[u8]) -> Result<Vec<SliceHeader>> {
        if packet.len() > self.budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        let mut headers: Vec<SliceHeader> = Vec::new();
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let kind = NalHeader::parse(nal)?;
            kind.require_base_layer()?;
            if !kind.is_vcl() {
                continue;
            }
            let id = SliceHeader::parameter_set_id(nal)?;
            let (sps, pps) = self
                .pairs
                .iter()
                .find(|(_, p)| p.id == id)
                .ok_or_else(|| invalid("HEVC slice references unknown PPS"))?;
            let header = SliceHeader::parse(nal, sps, pps, self.budget)?;
            if header.nal.temporal_id as usize >= sps.ordering.len() {
                return Err(invalid("HEVC slice exceeds SPS temporal layers"));
            }
            if let Some(first) = headers.first() {
                let previous = headers.last().unwrap();
                if header.first || header.address <= previous.address {
                    return Err(invalid(
                        "HEVC slice addresses must increase within one picture",
                    ));
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
        let headers = self.slice_headers(packet)?;
        let mut slice = None;
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let header = NalHeader::parse(nal)?;
            header.require_base_layer()?;
            if header.is_vcl() {
                if slice.is_none() {
                    slice = Some(nal);
                }
            } else if matches!(header.unit_type, 32..=34) {
                // Configuration changes must never reuse stale reference geometry.
                if header.unit_type == 33
                    && !self
                        .pairs
                        .iter()
                        .any(|(s, _)| Sps::parse(nal, self.budget).is_ok_and(|new| new == *s))
                {
                    return Err(invalid("HEVC in-band SPS change is not implemented"));
                }
                if header.unit_type == 34
                    && !self
                        .pairs
                        .iter()
                        .any(|(s, p)| Pps::parse(nal, s, self.budget).is_ok_and(|new| new == *p))
                {
                    return Err(invalid("HEVC in-band PPS change is not implemented"));
                }
            } else if hevc_sei::is_sei_unit(header.unit_type) {
                // An SEI this module cannot walk costs the guidance, never the
                // picture it travels with.
                if let Ok(Some(hdr)) = hevc_sei::hdr_from_nal(nal, self.budget) {
                    self.hdr.merge(hdr);
                }
            } else if !matches!(header.unit_type, 35..=40) {
                return Err(invalid(&format!(
                    "unsupported HEVC NAL type {}",
                    header.unit_type
                )));
            }
        }
        let Some(nal) = slice else { return Ok(None) };
        let id = SliceHeader::parameter_set_id(nal)?;
        let (sps, pps) = self
            .pairs
            .iter()
            .find(|(_, p)| p.id == id)
            .ok_or_else(|| invalid("HEVC slice references unknown PPS"))?;
        let header = &headers[0];
        if header.nal.temporal_id as usize >= sps.ordering.len() {
            return Err(invalid("HEVC slice exceeds SPS temporal layers"));
        }
        if header.nal.is_irap() {
            self.suppress_rasl = self.previous_poc.is_none()
                || header.nal.is_idr()
                || matches!(header.nal.unit_type, 16..=18);
        }
        if self.suppress_rasl && matches!(header.nal.unit_type, 8 | 9) {
            return Ok(None);
        }
        let modulus = 1i32 << sps.poc_bits;
        let lsb = header.poc_lsb as i32;
        let poc = if header.nal.is_idr() {
            0
        } else if matches!(header.nal.unit_type, 16..=18)
            || self.previous_poc.is_none() && header.nal.is_irap()
        {
            lsb
        } else {
            let previous = self
                .previous_poc
                .ok_or_else(|| invalid("HEVC stream must start at a random-access picture"))?;
            let old = previous.rem_euclid(modulus);
            let mut msb = previous - old;
            if lsb < old && old - lsb >= modulus / 2 {
                msb = msb
                    .checked_add(modulus)
                    .ok_or_else(|| invalid("HEVC POC overflow"))?;
            } else if lsb > old && lsb - old > modulus / 2 {
                msb = msb
                    .checked_sub(modulus)
                    .ok_or_else(|| invalid("HEVC POC overflow"))?;
            }
            msb.checked_add(lsb)
                .ok_or_else(|| invalid("HEVC POC overflow"))?
        };
        if header.nal.is_idr() {
            self.references.clear();
        }
        let (retained, lists) = reference_lists(&header, poc, &self.references)?;
        let mut slice_lists = vec![lists];
        for other in headers.iter().skip(1) {
            let (other_retained, other_lists) = reference_lists(other, poc, &self.references)?;
            if other_retained != retained {
                return Err(invalid("HEVC slices disagree on reference picture set"));
            }
            slice_lists.push(other_lists);
        }
        self.references.retain(|r| retained.contains(&r.poc));
        let retained_bytes = self
            .references
            .iter()
            .try_fold(0usize, |total, r| {
                let count = (r.picture.dimensions[0] as usize)
                    .checked_mul(r.picture.dimensions[1] as usize)?;
                total.checked_add(count.checked_mul(5)?)
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
        if header.nal.temporal_id == 0 && !matches!(header.nal.unit_type, 0 | 2 | 4 | 6..=9) {
            self.previous_poc = Some(poc);
        }
        if header.nal.unit_type >= 16 || header.nal.unit_type & 1 != 0 {
            self.references.push(Reference {
                poc,
                picture: Arc::clone(&picture),
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

fn reference_lists(
    header: &SliceHeader,
    poc: i32,
    references: &[Reference],
) -> Result<(Vec<i32>, [Vec<Reference>; 2])> {
    let mut retained = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for r in &header.short_term {
        let target = poc
            .checked_add(r.delta_poc)
            .ok_or_else(|| invalid("HEVC reference POC overflow"))?;
        retained.push(target);
        if r.used {
            let reference = references
                .iter()
                .find(|r| r.poc == target)
                .ok_or_else(|| {
                    invalid(&format!(
                        "HEVC reference POC {target} is missing for POC {poc} (NAL {})",
                        header.nal.unit_type
                    ))
                })?
                .clone();
            if r.delta_poc < 0 {
                before.push(reference);
            } else {
                after.push(reference);
            }
        }
    }
    let mut lists: [Vec<Reference>; 2] = [Vec::new(), Vec::new()];
    for list in 0..2 {
        let base: Vec<_> = if list == 0 {
            before.iter().chain(&after)
        } else {
            after.iter().chain(&before)
        }
        .cloned()
        .collect();
        for i in 0..header.references[list] as usize {
            if base.is_empty() {
                return Err(invalid("HEVC active reference list is empty"));
            }
            let index = header.list_modification[list]
                .as_ref()
                .map_or(i % base.len(), |m| m[i] as usize);
            lists[list].push(
                base.get(index)
                    .ok_or_else(|| invalid("HEVC reference-list index out of range"))?
                    .clone(),
            );
        }
    }

    Ok((retained, lists))
}
