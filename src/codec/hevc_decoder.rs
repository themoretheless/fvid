//! Stateful native HEVC access-unit decoding and reference picture storage.
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
#[derive(Clone)]
struct ParameterSets {
    sets: Vec<Sps>,
    pps_nals: Vec<(u8, Vec<u8>)>,
    pairs: Vec<(Sps, Pps)>,
}
pub struct HevcDecoder {
    params: ParameterSets,
    initial_params: ParameterSets,
    decoded_sps: Option<Sps>,
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
        let mut pps_nals = Vec::new();
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
                    pps_nals.push((pair.1.id, nal.to_vec()));
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
        let params = ParameterSets {
            sets,
            pps_nals,
            pairs,
        };
        Ok(Self {
            initial_params: params.clone(),
            params,
            decoded_sps: None,
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
            .and_then(|id| self.params.pairs.iter().find(|(_, p)| p.id == id))
            .unwrap_or(&self.params.pairs[0]);
        (s, p)
    }
    pub fn reset(&mut self) {
        self.params.clone_from(&self.initial_params);
        self.decoded_sps = None;
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
    // Validate parameter updates before committing them to decoder state.
    fn updated_pairs(&self, packet: &[u8]) -> Result<Option<ParameterSets>> {
        if packet.len() > self.budget {
            return Err(invalid("HEVC access unit exceeds decode budget"));
        }
        let mut updated: Option<ParameterSets> = None;
        let mut seen_slice = false;
        for nal in NalUnits::new(packet, self.length)? {
            let nal = nal?;
            let header = NalHeader::parse(nal)?;
            header.require_base_layer()?;
            seen_slice |= header.is_vcl();
            let params = updated.as_ref().unwrap_or(&self.params);
            match header.unit_type {
                33 => {
                    let new = Sps::parse(nal, self.budget)?;
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
                    let pps = params
                        .sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s, self.budget).ok())
                        .ok_or_else(|| invalid("HEVC in-band PPS has no valid SPS"))?;
                    if params
                        .pps_nals
                        .iter()
                        .any(|(id, bytes)| *id == pps.id && bytes == nal)
                    {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("HEVC changed PPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.pps_nals.iter_mut().find(|(id, _)| *id == pps.id) {
                        old.1 = nal.to_vec();
                    } else {
                        params.pps_nals.push((pps.id, nal.to_vec()));
                    }
                }
                _ => {}
            }
        }
        if let Some(params) = &mut updated {
            params.pairs = params
                .pps_nals
                .iter()
                .map(|(_, nal)| {
                    params
                        .sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s, self.budget).ok().map(|p| (s.clone(), p)))
                        .ok_or_else(|| invalid("HEVC updated PPS has no valid SPS"))
                })
                .collect::<Result<_>>()?;
        }
        Ok(updated)
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
                let previous = headers.last().unwrap();
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
                if header.first || order(header.address)? <= order(previous.address)? {
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
        if let Some(params) = self.updated_pairs(packet)? {
            self.params = params;
        }
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
                // Parameter sets were validated before parsing slices.
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
            .params
            .pairs
            .iter()
            .find(|(_, p)| p.id == id)
            .ok_or_else(|| invalid("HEVC slice references unknown PPS"))?;
        let header = &headers[0];
        if header.nal.temporal_id as usize >= sps.ordering.len() {
            return Err(invalid("HEVC slice exceeds SPS temporal layers"));
        }
        if self
            .decoded_sps
            .as_ref()
            .is_some_and(|previous| previous != sps)
        {
            if !header.nal.is_irap() {
                return Err(invalid("HEVC SPS change requires a random-access picture"));
            }
            self.references.clear();
            self.previous_poc = None;
            self.suppress_rasl = false;
        }
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
        let poc = super::hevc_poc::derive(sps, header.nal, header.poc_lsb, self.previous_poc)?;
        if header.nal.is_idr() {
            self.references.clear();
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
                let count = (r.picture.as_deref()?.dimensions[0] as usize)
                    .checked_mul(r.picture.as_deref()?.dimensions[1] as usize)?;
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
        if super::hevc_poc::updates_previous(header.nal) {
            self.previous_poc = Some(poc);
        }
        if header.nal.unit_type >= 16 || header.nal.unit_type & 1 != 0 {
            self.references.push(Reference {
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

fn reference_lists(
    header: &SliceHeader,
    poc: i32,
    references: &[Reference],
    poc_bits: u8,
) -> Result<(Vec<(i32, bool)>, [Vec<Reference>; 2])> {
    let mut retained = Vec::new();
    let mut before = Vec::new();
    let mut after = Vec::new();
    for r in &header.short_term {
        let target = poc
            .checked_add(r.delta_poc)
            .ok_or_else(|| invalid("HEVC reference POC overflow"))?;
        if references.iter().any(|r| r.poc == target && r.long_term) {
            return Err(invalid("HEVC RPS uses a long-term picture as short-term"));
        }
        retained.push((target, false));
        if r.used {
            let mut reference = references
                .iter()
                .find(|r| r.poc == target)
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
    let dpb_pocs: Vec<_> = references.iter().map(|r| r.poc).collect();
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
                    .find(|r| r.poc == target)
                    .ok_or_else(|| invalid("HEVC long-term current reference is missing"))?
                    .clone();
                reference.long_term = true;
                long.push(reference);
            }
        }
    }
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
                    lists[list].push(Reference {
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
    use super::super::{hevc_long_term::LongTermReference, hevc_rps::ShortTermReference};
    use super::*;
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
            .map(|poc| Reference {
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
