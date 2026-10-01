//! Stateful decoding of length-prefixed AVC access units using FVid codecs.
//! Accepts one supported progressive I/P/B slice per picture.
//! `decode_order` leaves timestamp association and display reordering to callers.
use super::{
    avc::{Pps, Sps},
    avc_dpb::ReferenceBuffer,
    avc_inter_picture::decode_inter_resolved_slices_with_motion,
    avc_picture::{IntraPicture, decode_intra_slices},
    avc_poc::PocDecoder,
    avc_reference_motion::ReferenceMotionField,
    avc_slice::{MemoryOperation, SliceHeader, SliceType},
    config::{AvcConfig, NalUnits},
};
use crate::{Result, invalid};
use std::sync::Arc;

/// One DPB entry owns image and motion metadata together. Intra-only pictures
/// have no motion field; co-located lookup treats them as intra at every cell.
pub struct DecodedReferencePicture {
    pub picture: Arc<IntraPicture>,
    pub motion: Option<ReferenceMotionField>,
}

/// Owns parameter sets, POC state and reference pictures. No external decoder.
/// The budget covers decoded reference storage and picture reconstruction;
/// input/configuration bytes and output Arcs retained by callers are excluded.
/// After an error, call `reset` before decoding another access unit.
#[derive(Clone)]
struct Parameters {
    sets: Vec<Sps>,
    pps_nals: Vec<(u32, Vec<u8>)>,
    pairs: Vec<(Sps, Pps)>,
}
pub struct AvcDecoder {
    length_size: u8,
    params: Parameters,
    initial_params: Parameters,
    decoded_sps: Option<Sps>,
    budget: usize,
    poc: PocDecoder,
    dpb: Option<ReferenceBuffer<DecodedReferencePicture>>,
    active_sps: Option<u32>,
    previous_reference: Option<u32>,
    last_poc: Option<i32>,
    next_id: u64,
    failed: bool,
}
impl AvcDecoder {
    pub fn new(configuration: &[u8], budget: usize) -> Result<Self> {
        let config = AvcConfig::parse(configuration)?;
        let sets: Vec<_> = config
            .sps
            .iter()
            .map(|nal| Sps::parse(nal))
            .collect::<Result<_>>()?;
        for (index, sps) in sets.iter().enumerate() {
            if sets[..index].iter().any(|previous| previous.id == sps.id) {
                return Err(invalid("duplicate AVC sequence parameter set ID"));
            }
        }
        let mut pairs = Vec::new();
        let mut pps_nals = Vec::new();
        for nal in config.pps {
            let pair = sets
                .iter()
                .find_map(|sps| Pps::parse(nal, sps).ok().map(|pps| (sps.clone(), pps)))
                .ok_or_else(|| invalid("invalid PPS or missing SPS"))?;
            if pairs.iter().any(|(_, p): &(Sps, Pps)| p.id == pair.1.id) {
                return Err(invalid("duplicate AVC parameter set ID"));
            }
            pps_nals.push((pair.1.id, nal.to_vec()));
            pairs.push(pair);
        }
        if pairs.is_empty() {
            return Err(invalid("AVC configuration has no parameter sets"));
        }
        let params = Parameters {
            sets,
            pps_nals,
            pairs,
        };
        Ok(Self {
            length_size: config.length_size,
            initial_params: params.clone(),
            params,
            decoded_sps: None,
            budget,
            poc: PocDecoder::new(),
            dpb: None,
            active_sps: None,
            previous_reference: None,
            last_poc: None,
            next_id: 0,
            failed: false,
        })
    }
    pub fn active_vui(&self) -> Option<&super::avc::Vui> {
        self.params
            .pairs
            .iter()
            .find(|(s, _)| Some(s.id) == self.active_sps)
            .and_then(|(s, _)| s.vui.as_ref())
    }
    /// The VUI of the sequence parameter set the configuration record lists
    /// first. A coded picture names the set it wants, so [`Self::active_vui`]
    /// answers only once one has been decoded; what a stream says about its
    /// pictures before that is here, and a reader that grades the first picture
    /// asks at that moment.
    pub fn recorded_vui(&self) -> Option<&super::avc::Vui> {
        self.params.pairs.first().and_then(|(s, _)| s.vui.as_ref())
    }
    /// Drops reference pictures and requires the next coded picture to be IDR.
    pub fn reset(&mut self) {
        self.params.clone_from(&self.initial_params);
        self.decoded_sps = None;
        self.poc = PocDecoder::new();
        self.dpb = None;
        self.active_sps = None;
        self.previous_reference = None;
        self.last_poc = None;
        self.next_id = 0;
        self.failed = false;
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Arc<IntraPicture>>> {
        if self.failed {
            return Err(invalid("AVC decoder requires reset after an error"));
        }
        let result = self.decode_inner(packet, false);
        self.failed = result.is_err();
        result
    }
    /// Returns pictures in access-unit decode order, including B pictures.
    /// The caller must associate timestamps and perform display reordering.
    pub fn decode_order(&mut self, packet: &[u8]) -> Result<Option<Arc<IntraPicture>>> {
        if self.failed {
            return Err(invalid("AVC decoder requires reset after an error"));
        }
        let result = self.decode_inner(packet, true);
        self.failed = result.is_err();
        result
    }
    fn updated_parameters(&self, packet: &[u8]) -> Result<Option<Parameters>> {
        let mut updated: Option<Parameters> = None;
        let mut seen_slice = false;
        for nal in NalUnits::new(packet, self.length_size)? {
            let nal = nal?;
            let kind = nal[0] & 31;
            seen_slice |= matches!(kind, 1 | 5);
            let params = updated.as_ref().unwrap_or(&self.params);
            match kind {
                7 => {
                    let new = Sps::parse(nal)?;
                    if params.sets.iter().any(|s| *s == new) {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("AVC changed SPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.sets.iter_mut().find(|s| s.id == new.id) {
                        *old = new;
                    } else {
                        params.sets.push(new);
                    }
                }
                8 => {
                    let new = params
                        .sets
                        .iter()
                        .find_map(|s| Pps::parse(nal, s).ok())
                        .ok_or_else(|| invalid("AVC in-band PPS has no valid SPS"))?;
                    if params
                        .pps_nals
                        .iter()
                        .any(|(id, bytes)| *id == new.id && bytes == nal)
                    {
                        continue;
                    }
                    if seen_slice {
                        return Err(invalid("AVC changed PPS follows picture slices"));
                    }
                    let params = updated.get_or_insert_with(|| self.params.clone());
                    if let Some(old) = params.pps_nals.iter_mut().find(|(id, _)| *id == new.id) {
                        old.1 = nal.to_vec();
                    } else {
                        params.pps_nals.push((new.id, nal.to_vec()));
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
                        .find_map(|s| Pps::parse(nal, s).ok().map(|p| (s.clone(), p)))
                        .ok_or_else(|| invalid("AVC updated PPS has no valid SPS"))
                })
                .collect::<Result<_>>()?;
        }
        Ok(updated)
    }
    fn decode_inner(
        &mut self,
        packet: &[u8],
        allow_reordering: bool,
    ) -> Result<Option<Arc<IntraPicture>>> {
        if let Some(params) = self.updated_parameters(packet)? {
            self.params = params;
        }
        let mut coded = None;
        for nal in NalUnits::new(packet, self.length_size)? {
            let nal = nal?;
            match nal[0] & 31 {
                1 | 5 => {
                    coded.get_or_insert(nal);
                }
                6 | 7 | 8 | 9 | 12 => {}
                _ => return Err(invalid("unsupported in-band AVC NAL")),
            }
        }
        let Some(nal) = coded else {
            return Ok(None);
        };
        let id = SliceHeader::parameter_set_id(nal)?;
        let (sps, pps) = self
            .params
            .pairs
            .iter()
            .find(|(_, p)| p.id == id)
            .ok_or_else(|| invalid("unknown AVC PPS"))?;
        let slices =
            super::avc_access_unit::prepare(packet, self.length_size, sps, pps, self.budget)?;
        let header = &slices
            .first()
            .ok_or_else(|| invalid("missing AVC slice"))?
            .header;
        if !matches!(
            header.slice_type,
            SliceType::I | SliceType::P | SliceType::B
        ) {
            return Err(invalid("AVC picture type is not implemented"));
        }
        if !header.idr
            && (self.active_sps != Some(sps.id) || self.decoded_sps.as_ref() != Some(sps))
        {
            return Err(invalid("AVC stream/configuration change requires IDR"));
        }
        if !header.idr
            && self.previous_reference.is_some_and(|previous| {
                header.frame_num != previous
                    && header.frame_num != (previous + 1) % (1 << sps.frame_num_bits)
            })
        {
            return Err(invalid("AVC frame-number gaps are not implemented"));
        }
        let (w, h) = sps.coded_dimensions();
        let motion_bytes = ReferenceMotionField::storage_bytes(w as usize, h as usize)?;
        let reference_bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(3))
            .and_then(|n| n.checked_add(motion_bytes))
            .and_then(|n| n.checked_mul(sps.max_num_ref_frames.max(1) as usize))
            .ok_or_else(|| invalid("AVC reference memory overflow"))?;
        let scratch_budget = self
            .budget
            .checked_sub(reference_bytes)
            .ok_or_else(|| invalid("AVC references exceed decoder memory budget"))?;
        let order = self.poc.decode(sps, &header)?;
        if header.idr {
            self.dpb = Some(ReferenceBuffer::new(
                sps.frame_num_bits,
                sps.max_num_ref_frames,
            )?);
            self.active_sps = Some(sps.id);
            self.decoded_sps = Some(sps.clone());
            self.last_poc = None;
            self.previous_reference = None;
        }
        if !allow_reordering
            && self
                .last_poc
                .is_some_and(|p| order.before_marking.picture() <= p)
        {
            return Err(invalid("AVC display reordering is not implemented"));
        }
        let buffer = self
            .dpb
            .as_mut()
            .ok_or_else(|| invalid("AVC stream must begin with IDR"))?;
        let (picture, motion) = match header.slice_type {
            SliceType::I if slices.iter().all(|s|s.header.slice_type==SliceType::I) => (
                decode_intra_slices(
                    &slices.iter().map(|slice| &slice.header).collect::<Vec<_>>(),
                    sps,
                    pps,
                    scratch_budget,
                )?,
                None,
            ),
            SliceType::I | SliceType::P | SliceType::B => {
                let lists = slices
                    .iter()
                    .map(|slice| buffer.lists(&slice.header, order.before_marking.picture()))
                    .collect::<Result<Vec<_>>>()?;
                let entries = buffer.references();
                let mut refs_by_slice = Vec::with_capacity(slices.len());
                let mut metadata_by_slice = Vec::with_capacity(slices.len());
                for lists in &lists {
                    let mut refs = [Vec::new(), Vec::new()];
                    let mut metadata = [Vec::new(), Vec::new()];
                    for (list, ids) in [&lists.l0, &lists.l1].into_iter().enumerate() {
                        for id in ids {
                            refs[list].push(
                                buffer
                                    .get(*id)
                                    .map(|r| r.picture.as_ref())
                                    .ok_or_else(|| invalid("missing decoded AVC reference"))?,
                            );
                            metadata[list].push(
                                *entries
                                    .iter()
                                    .find(|r| r.id == *id)
                                    .ok_or_else(|| invalid("missing AVC reference metadata"))?,
                            );
                        }
                    }
                    refs_by_slice.push(refs);
                    metadata_by_slice.push(metadata);
                }
                let direct_by_slice = slices
                    .iter()
                    .zip(&lists)
                    .zip(&metadata_by_slice)
                    .map(|((slice, lists), metadata)| {
                        if slice.header.slice_type != SliceType::B {
                            return Ok(None);
                        }
                        let first = lists
                            .l1
                            .first()
                            .ok_or_else(|| invalid("B slice has no L1 reference"))?;
                        Ok(Some(super::avc_direct::DirectPrediction {
                            spatial: slice.header.direct_spatial_mv_pred,
                            inference8: sps.direct_8x8_inference,
                            current_poc: order.before_marking.picture(),
                            list0: &metadata[0],
                            list1: &metadata[1],
                            colocated: buffer.get(*first).and_then(|r| r.motion.as_ref()),
                        }))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let retain = header.nal_ref_idc != 0;
                let reconstruction_budget = scratch_budget
                    .checked_sub(if retain { motion_bytes } else { 0 })
                    .ok_or_else(|| invalid("AVC motion snapshot exceeds decoder budget"))?;
                let (picture, working) = decode_inter_resolved_slices_with_motion(
                    &slices.iter().map(|s| &s.header).collect::<Vec<_>>(),
                    sps,
                    pps,
                    &refs_by_slice
                        .iter()
                        .map(|refs| [refs[0].as_slice(), refs[1].as_slice()])
                        .collect::<Vec<_>>(),
                    &direct_by_slice
                        .iter()
                        .map(Option::as_ref)
                        .collect::<Vec<_>>(),
                    reconstruction_budget,
                )?;
                let motion = if retain {
                    Some(
                        working.snapshot_slices(
                            &lists
                                .iter()
                                .enumerate()
                                .map(|(i, lists)| {
                                    (i as u32, [lists.l0.as_slice(), lists.l1.as_slice()])
                                })
                                .collect::<Vec<_>>(),
                            motion_bytes,
                        )?,
                    )
                } else {
                    None
                };
                (picture, motion)
            }
            _ => unreachable!(),
        };
        let picture = Arc::new(picture);
        buffer.finish(
            &header,
            order.after_marking.picture(),
            self.next_id,
            Arc::new(DecodedReferencePicture {
                picture: picture.clone(),
                motion,
            }),
        )?;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("AVC picture ID overflow"))?;
        self.last_poc = Some(order.after_marking.picture());
        if header.nal_ref_idc != 0 {
            self.previous_reference = Some(
                if header
                    .memory_operations
                    .iter()
                    .any(|op| matches!(op, MemoryOperation::Reset))
                {
                    0
                } else {
                    header.frame_num
                },
            );
        }
        Ok(Some(picture))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configuration() -> Vec<u8> {
        let hex = "6742c01fda03c045fbc044000003000400000300f03c60ca80";
        let sps: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut data = vec![1, 66, 0xc0, 31, 0xff, 0xe1];
        data.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        data.extend_from_slice(&sps);
        data.extend_from_slice(&[1, 0, 4, 0x68, 0xce, 0x09, 0xc8]);
        data
    }
    fn packet(nals: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        for nal in nals {
            data.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            data.extend_from_slice(nal);
        }
        data
    }
    #[test]
    fn dpb_retains_motion_identity_and_releases_evicted_metadata() {
        fn hex(text: &str) -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect()
        }
        let sps = hex("674d400ada7a1000000300100000030320f1226a");
        let pps = hex("68ee06cb20");
        let mut config = vec![1, 77, 64, 10, 255, 225];
        config.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        config.extend_from_slice(&sps);
        config.push(1);
        config.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        config.extend_from_slice(&pps);
        let idr = packet(&[&hex("658884fffeedcfe052e36fc1")]);
        let p1 = packet(&[&hex("419a23ff5d2e09a431c3d011f0")]);
        let p2 = packet(&[&hex("419a43ff5d2e09a431c3d011f1")]);
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        assert_eq!(decoder.decode(&idr).unwrap().unwrap().y, vec![64; 256]);
        assert!(
            decoder
                .dpb
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .motion
                .is_none()
        );
        assert_eq!(decoder.decode(&p1).unwrap().unwrap().y, vec![66; 256]);
        let old = decoder.dpb.as_ref().unwrap().get(1).unwrap();
        let motion = old
            .motion
            .as_ref()
            .unwrap()
            .colocated([0, 0])
            .unwrap()
            .unwrap();
        assert_eq!(
            (motion.picture_id, motion.reference_index, motion.vector),
            (0, 0, [0, 0])
        );
        let weak = Arc::downgrade(old);
        assert_eq!(decoder.decode(&p2).unwrap().unwrap().y, vec![68; 256]);
        assert!(weak.upgrade().is_none());
        let dpb = decoder.dpb.as_ref().unwrap();
        assert_eq!(dpb.references().len(), 1);
        assert_eq!(
            dpb.get(2)
                .unwrap()
                .motion
                .as_ref()
                .unwrap()
                .colocated([0, 0])
                .unwrap()
                .unwrap()
                .picture_id,
            1
        );
        let weak = Arc::downgrade(dpb.get(2).unwrap());
        decoder.decode(&idr).unwrap();
        assert!(weak.upgrade().is_none());
        assert!(
            decoder
                .dpb
                .as_ref()
                .unwrap()
                .get(3)
                .unwrap()
                .motion
                .is_none()
        );
        decoder.reset();
        assert!(decoder.dpb.is_none());
    }
    #[test]
    fn errors_poison_until_reset_and_metadata_does_not_create_pictures() {
        let mut decoder = AvcDecoder::new(&configuration(), 1 << 20).unwrap();
        let aud = packet(&[&[9, 0xf0]]);
        assert!(decoder.decode(&aud).unwrap().is_none());
        assert!(decoder.decode(&[0, 0, 0, 10, 9]).is_err());
        assert!(decoder.decode(&aud).is_err());
        decoder.reset();
        assert!(decoder.decode(&aud).unwrap().is_none());
    }
    #[test]
    fn rejects_multiple_slices_before_changing_picture_state_and_bounds_references() {
        let idr = &[0x65, 0x88, 0x84, 0x3a, 0x27, 0x80];
        let mut decoder = AvcDecoder::new(&configuration(), packet(&[idr, idr]).len()).unwrap();
        assert!(decoder.decode(&packet(&[idr, idr])).is_err());
        assert!(decoder.active_sps.is_none());
        decoder.reset();
        let error = decoder.decode(&packet(&[idr])).err().unwrap();
        assert!(error.to_string().contains("references exceed"));
        assert!(decoder.active_sps.is_none());
    }
}
