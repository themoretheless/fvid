//! Own HEVC POC and reference DPB scheduling for direct NVDEC submission.
use super::owned_nvdec_hevc::{HevcPicture, HevcReference};
use fvid_codecs::codec::{
    config::NalUnits, hevc_nal::NalHeader, hevc_poc, hevc_pps::Pps, hevc_slice::SliceHeader,
    hevc_sps::Sps,
};
use fvid_cuda::{CodecDevice, NvdecCodec, NvdecSession, NvdecSurface};
use std::sync::{Arc, Weak};

struct FrameSlot {
    index: u32,
    poc: i32,
}
/// A retained ticket reserves its decode slot even after DPB eviction.
#[derive(Clone)]
pub struct DecodedHevc {
    slot: Arc<FrameSlot>,
    pub picture_order: i32,
    pub output: bool,
    pub no_output_of_prior_pictures: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_mp4::Mp4Reader;
    use fvid_codecs::codec::{config::HevcConfig, hevc_decoder::HevcDecoder};
    use std::io::Cursor;
    const MAIN: &[u8] = include_bytes!("../../../tests/fixtures/hevc/main-ipb.mp4");
    const MAIN10: &[u8] = include_bytes!("../../../tests/fixtures/hevc/main10-ipb.mp4");
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA HEVC Main/Main10 NVDEC"]
    fn owned_hevc_ipb_scheduler_submits_and_maps_on_nvidia() {
        for bytes in [include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc.mp4").as_slice(), include_bytes!("../../../tests/fixtures/playback-errors/cuda-hevc-main10.mp4").as_slice()] {
            let (mut reader, state, mut software) = input(bytes);
            let mut decoder =
                HevcNvdecDecoder::new(state.sps, state.pps, state.length_size, 0, 32, 2, 1 << 20)
                    .unwrap();
            let mut packet = Vec::new();
            for sample in 0..reader.tracks()[0].samples.len() {
                reader.read_packet(0, sample, &mut packet).unwrap();
                let expected = software.decode_packet(&packet).unwrap().unwrap();
                let frame = decoder.decode(&packet).unwrap().unwrap();
                assert_eq!(frame.picture_order, expected.poc);
                let surface = decoder.map(&frame).unwrap();
                assert_ne!(surface.pointer, 0);
                decoder.unmap(surface.slot).unwrap();
            }
            decoder.close().unwrap();
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires physical NVIDIA HEVC NVDEC long-term qualification"]
    fn owned_hevc_long_term_submits_and_maps_on_nvidia() {
        for bytes in [include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-lsb-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-mixed-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-sps-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-l1-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-reordered-base-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-reordered-mixed-main8.mp4").as_slice()] {
        let (mut reader, state, mut software) = input(bytes);
        let mut decoder = HevcNvdecDecoder::new(state.sps, state.pps, state.length_size, 0, 32, 2, 1 << 20).unwrap();
        let mut packet = Vec::new();
        for sample in 0..reader.tracks()[0].samples.len() {
            reader.read_packet(0, sample, &mut packet).unwrap();
            let expected = software.decode_packet(&packet).unwrap().unwrap();
            let frame = decoder.decode(&packet).unwrap().unwrap();
            assert_eq!(frame.picture_order, expected.poc);
            assert_eq!(frame.output, expected.output);
            let surface = decoder.map(&frame).unwrap();
            assert_ne!(surface.pointer, 0);
            decoder.unmap(surface.slot).unwrap();
        }
        decoder.close().unwrap();
        }
    }
    fn input(bytes: &[u8]) -> (Mp4Reader<Cursor<&[u8]>>, Scheduler, HevcDecoder) {
        let reader = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let config = HevcConfig::parse(&reader.tracks()[0].configuration).unwrap();
        let software =
            HevcDecoder::from_configuration(&reader.tracks()[0].configuration, 64 << 20).unwrap();
        let (sps, pps) = software.parameters();
        let state =
            Scheduler::new(sps.clone(), pps.clone(), config.length_size, 32, 1 << 20).unwrap();
        (reader, state, software)
    }
    #[test]
    fn long_term_to_short_term_refusal_preserves_scheduler_state() {
        let data = include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-invalid-short-rext8.mp4");
        let (mut reader, mut state, _) = input(data);
        let mut packet = Vec::new();
        for sample in 0..2 {
            reader.read_packet(0, sample, &mut packet).unwrap();
            let pending = state.prepare(&packet).unwrap().unwrap();
            drop(state.commit(pending));
        }
        assert_eq!(state.reference_long_term, [0]);
        let retained: Vec<_> = state.references.iter().map(|r| (r.index, r.poc)).collect();
        reader.read_packet(0, 2, &mut packet).unwrap();
        let error = state.prepare(&packet).err().expect("must refuse invalid reference classification");
        assert!(error.contains("long-term picture as short-term"), "{error}");
        assert_eq!(state.previous_poc, Some(1));
        assert_eq!(state.reference_long_term, [0]);
        assert_eq!(state.references.iter().map(|r| (r.index, r.poc)).collect::<Vec<_>>(), retained);
    }
    #[test]
    fn mixed_and_lsb_only_fixtures_follow_software_and_keep_live_reference_slots() {
        for bytes in [include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-mixed-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-lsb-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-sps-rext8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-b-mixed-l1-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-reordered-base-main8.mp4").as_slice(),
                      include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-reordered-mixed-main8.mp4").as_slice()] {
            let (mut reader, mut state, mut software) = input(bytes);
            let mut packet = Vec::new();
            for sample in 0..reader.tracks()[0].samples.len() {
                reader.read_packet(0, sample, &mut packet).unwrap();
                let expected = software.decode_packet(&packet).unwrap().unwrap();
                let pending = state.prepare(&packet).unwrap().unwrap();
                assert_eq!(pending.frame.picture_order, expected.poc);
                assert_eq!(pending.frame.output, expected.output);
                for reference in &pending.references {
                    assert!(state.slots[reference.index as usize].upgrade().is_some());
                }
                drop(state.commit(pending));
            }
        }
    }
    #[test]
    fn long_term_fixture_retains_slots_and_aborted_submission_preserves_state() {
        let data = include_bytes!("../../../tests/fixtures/playback-errors/hevc-long-term-rext8.mp4");
        let (mut reader, mut state, mut software) = input(data);
        let mut packet = Vec::new();
        for sample in 0..3 {
            reader.read_packet(0, sample, &mut packet).unwrap();
            let expected = software.decode_packet(&packet).unwrap().unwrap();
            let previous = state.previous_poc;
            let previous_classification = state.reference_long_term.clone();
            let retained: Vec<_> = state.references.iter().map(|r| (r.index, r.poc)).collect();
            let pending = state.prepare(&packet).unwrap().unwrap();
            assert_eq!(pending.frame.picture_order, expected.poc);
            drop(pending);
            assert_eq!(state.previous_poc, previous);
            assert_eq!(state.reference_long_term, previous_classification);
            assert_eq!(state.references.iter().map(|r| (r.index, r.poc)).collect::<Vec<_>>(), retained);
            let pending = state.prepare(&packet).unwrap().unwrap();
            assert_eq!(pending.references.len(), if sample == 0 { 1 } else { 2 });
            if sample != 0 { assert_eq!(pending.references[0].poc, sample as i32 - 1); }
            let frame = state.commit(pending);
            assert!(state.owns(&frame));
            assert_eq!(state.previous_poc, Some(sample as i32));
        }
    }
    #[test]
    fn main_and_main10_all_ipb_pictures_follow_owned_software_poc_and_dpb() {
        for bytes in [MAIN, MAIN10] {
            let (mut reader, mut state, mut software) = input(bytes);
            let mut packet = Vec::new();
            let mut orders = Vec::new();
            let mut tickets = Vec::new();
            for sample in 0..reader.tracks()[0].samples.len() {
                reader.read_packet(0, sample, &mut packet).unwrap();
                let expected = software.decode_packet(&packet).unwrap().unwrap();
                let pending = state.prepare(&packet).unwrap().unwrap();
                assert_eq!(pending.frame.picture_order, expected.poc);
                assert_eq!(pending.frame.output, expected.output);
                let frame = state.commit(pending);
                assert!(state.owns(&frame));
                assert!(
                    tickets
                        .iter()
                        .all(|held: &DecodedHevc| held.slot.index != frame.slot.index)
                );
                orders.push(frame.picture_order);
                tickets.push(frame);
            }
            assert!(orders.len() > 2);
            assert!(
                tickets.iter().all(|held| state.owns(held)),
                "output tickets must survive DPB eviction"
            );
            assert!(
                orders.windows(2).any(|pair| pair[0] > pair[1]),
                "fixture must exercise reordered B pictures"
            );
        }
    }
    #[test]
    fn held_slots_and_aborted_submission_do_not_advance_poc_or_references() {
        let (mut reader, mut state, _) = input(MAIN);
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let held: Vec<_> = (0..state.slots.len())
            .map(|i| {
                let slot = Arc::new(FrameSlot {
                    index: i as u32,
                    poc: i as i32,
                });
                state.slots[i] = Arc::downgrade(&slot);
                slot
            })
            .collect();
        assert!(state.prepare(&packet).is_err());
        assert_eq!(state.previous_poc, None);
        assert!(state.references.is_empty());
        drop(held);
        let pending = state.prepare(&packet).unwrap().unwrap();
        let first_slot = pending.frame.slot.index;
        drop(pending);
        assert_eq!(state.previous_poc, None);
        assert!(state.references.is_empty());
        let pending = state.prepare(&packet).unwrap().unwrap();
        let frame = state.commit(pending);
        assert_eq!(frame.slot.index, first_slot);
        assert_eq!(state.previous_poc, Some(0));
        // Tickets from a different decoder cannot authorize map/slot reuse.
        let (_, other, _) = input(MAIN);
        assert!(!other.owns(&frame));
    }
    #[test]
    fn shared_poc_handles_both_wrap_directions_and_temporal_update_rules() {
        let (_, state, _) = input(MAIN);
        let mut sps = state.sps.clone();
        sps.poc_bits = 4;
        let nal = NalHeader {
            unit_type: 1,
            layer_id: 0,
            temporal_id: 0,
        };
        assert_eq!(hevc_poc::derive(&sps, nal, 1, Some(14)).unwrap(), 17);
        assert_eq!(hevc_poc::derive(&sps, nal, 14, Some(1)).unwrap(), -2);
        assert!(hevc_poc::derive(&sps, nal, 1, None).is_err());
        assert!(hevc_poc::derive(&sps, nal, 16, Some(0)).is_err());
        assert!(hevc_poc::updates_previous(nal));
        assert!(!hevc_poc::updates_previous(NalHeader {
            temporal_id: 1,
            ..nal
        }));
        assert!(!hevc_poc::updates_previous(NalHeader {
            unit_type: 9,
            ..nal
        }));
    }
}
struct Scheduler {
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_bytes: usize,
    previous_poc: Option<i32>,
    suppress_rasl: bool,
    references: Vec<Arc<FrameSlot>>,
    reference_long_term: Vec<i32>,
    slots: Vec<Weak<FrameSlot>>,
}
struct Pending {
    picture: HevcPicture,
    frame: DecodedHevc,
    previous_poc: Option<i32>,
    suppress_rasl: bool,
    references: Vec<Arc<FrameSlot>>,
    reference_long_term: Vec<i32>,
}
impl Scheduler {
    fn new(
        sps: Sps,
        pps: Pps,
        length_size: u8,
        capacity: u32,
        max_bytes: usize,
    ) -> Result<Self, String> {
        super::owned_nvdec_hevc::configuration(&sps, &pps)?;
        let bound = sps
            .ordering
            .last()
            .ok_or("HEVC has no DPB ordering")?
            .max_decoded_pictures as u32;
        if !matches!(length_size, 1 | 2 | 4)
            || !(2..=32).contains(&capacity)
            || bound == 0
            || bound > 16
            || capacity <= bound
            || max_bytes == 0
        {
            return Err("HEVC NVDEC needs valid limits and a spare decode slot".into());
        }
        Ok(Self {
            sps,
            pps,
            length_size,
            max_bytes,
            previous_poc: None,
            suppress_rasl: false,
            references: Vec::new(),
            reference_long_term: Vec::new(),
            slots: (0..capacity).map(|_| Weak::new()).collect(),
        })
    }
    fn prepare(&mut self, packet: &[u8]) -> Result<Option<Pending>, String> {
        if packet.len() > self.max_bytes {
            return Err("HEVC packet exceeds byte limit".into());
        }
        let mut slices = Vec::new();
        for nal in NalUnits::new(packet, self.length_size).map_err(|e| e.to_string())? {
            let nal = nal.map_err(|e| e.to_string())?;
            let header = NalHeader::parse(nal).map_err(|e| e.to_string())?;
            header.require_base_layer().map_err(|e| e.to_string())?;
            if matches!(header.unit_type, 32..=34) {
                return Err("HEVC in-band parameter changes need decoder reconfiguration".into());
            }
            if header.is_vcl() {
                slices.push(nal);
            }
        }
        let first = *slices.first().ok_or("HEVC packet has no picture")?;
        let header = SliceHeader::parse(first, &self.sps, &self.pps, self.max_bytes)
            .map_err(|e| e.to_string())?;
        if header.nal.temporal_id as usize >= self.sps.ordering.len() {
            return Err("HEVC picture exceeds SPS temporal layers".into());
        }
        let suppress_rasl = if header.nal.is_irap() {
            self.previous_poc.is_none()
                || header.nal.is_idr()
                || matches!(header.nal.unit_type, 16..=18)
        } else {
            self.suppress_rasl
        };
        if suppress_rasl && matches!(header.nal.unit_type, 8 | 9) {
            return Ok(None);
        }
        let poc = hevc_poc::derive(&self.sps, header.nal, header.poc_lsb, self.previous_poc)
            .map_err(|e| e.to_string())?;
        let mut references = Vec::new();
        for r in &header.short_term {
            let target = poc
                .checked_add(r.delta_poc)
                .ok_or("HEVC reference POC overflow")?;
            if self.reference_long_term.contains(&target) {
                return Err("HEVC NVDEC RPS uses a long-term picture as short-term".into());
            }
            let reference = self
                .references
                .iter()
                .find(|r| r.poc == target)
                .ok_or("HEVC RPS reference has not been decoded")?;
            references.push(reference.clone());
        }
        let dpb_pocs: Vec<_> = self.references.iter().map(|r| r.poc).collect();
        let mut long_pocs = Vec::new();
        for entry in &header.long_term {
            let Some(target) = entry.resolve(poc, self.sps.poc_bits, &dpb_pocs)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            if target == poc
                || long_pocs.contains(&target)
                || references.iter().any(|r| r.poc == target)
            {
                return Err("HEVC NVDEC long-term RPS repeats a reference POC".into());
            }
            long_pocs.push(target);
            if let Some(reference) = self.references.iter().find(|r| r.poc == target) {
                references.push(reference.clone());
            } else if entry.used {
                return Err("HEVC NVDEC long-term reference has not been decoded".into());
            }
        }
        // Weak slots include references from the old DPB until submission commits,
        // plus display tickets and mappings. No live driver source is overwritten.
        let index = self
            .slots
            .iter()
            .position(|s| s.strong_count() == 0)
            .ok_or("HEVC decode slots retained by references/display/mappings")?
            as u32;
        let gpu: Vec<_> = references
            .iter()
            .map(|r| HevcReference {
                slot: r.index,
                poc: r.poc,
                long_term: long_pocs.contains(&r.poc),
            })
            .collect();
        let picture = HevcPicture::prepare(
            &self.sps,
            &self.pps,
            &slices,
            index,
            poc,
            &gpu,
            self.max_bytes,
        )?;
        let slot = Arc::new(FrameSlot { index, poc });
        if header.nal.is_irap() || header.nal.unit_type & 1 != 0 {
            references.push(slot.clone());
        }
        if references.len() > self.sps.ordering.last().unwrap().max_decoded_pictures as usize {
            return Err("HEVC reference count exceeds SPS DPB capacity".into());
        }
        let previous_poc = if hevc_poc::updates_previous(header.nal) {
            Some(poc)
        } else {
            self.previous_poc
        };
        let reference_long_term = long_pocs.into_iter()
            .filter(|poc| references.iter().any(|reference| reference.poc == *poc))
            .collect();
        self.slots[index as usize] = Arc::downgrade(&slot);
        Ok(Some(Pending {
            picture,
            frame: DecodedHevc {
                slot,
                picture_order: poc,
                output: header.picture_output,
                no_output_of_prior_pictures: header.no_output_of_prior_pictures,
            },
            previous_poc,
            suppress_rasl,
            references,
            reference_long_term,
        }))
    }
    fn commit(&mut self, pending: Pending) -> DecodedHevc {
        self.previous_poc = pending.previous_poc;
        self.suppress_rasl = pending.suppress_rasl;
        self.references = pending.references;
        self.reference_long_term = pending.reference_long_term;
        pending.frame
    }
    fn owns(&self, frame: &DecodedHevc) -> bool {
        self.slots
            .get(frame.slot.index as usize)
            .and_then(Weak::upgrade)
            .is_some_and(|slot| Arc::ptr_eq(&slot, &frame.slot))
    }
}

pub struct HevcNvdecDecoder {
    scheduler: Scheduler,
    session: NvdecSession,
    mapped: Vec<(usize, Arc<FrameSlot>)>,
    failed: bool,
}
pub(crate) fn qualify_packets(
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_bytes: usize,
    mut next: impl FnMut(&mut Vec<u8>) -> Result<bool, String>,
) -> Result<(), String> {
    let mut scheduler = Scheduler::new(sps, pps, length_size, 32, max_bytes)?;
    let mut packet = Vec::new();
    while next(&mut packet)? {
        if let Some(pending) = scheduler.prepare(&packet)? {
            drop(scheduler.commit(pending));
        }
    }
    Ok(())
}
pub(crate) fn movie_visibility(
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_bytes: usize,
    max_packets: usize,
    mut next: impl FnMut(&mut Vec<u8>) -> Result<bool, String>,
) -> Result<Vec<bool>, String> {
    let mut scheduler = Scheduler::new(sps, pps, length_size, 32, max_bytes)?;
    let mut packet = Vec::new();
    let mut visibility = Vec::new();
    while next(&mut packet)? {
        if visibility.len() >= max_packets {
            return Err("HEVC visibility exceeds sample count".into());
        }
        let frame = if let Some(pending) = scheduler.prepare(&packet)? {
            if !visibility.is_empty() && pending.frame.no_output_of_prior_pictures {
                return Err(
                    "HEVC mid-stream prior-output suppression needs movie qualification".into(),
                );
            }
            Some(scheduler.commit(pending))
        } else {
            None
        };
        visibility.try_reserve(1).map_err(|e| e.to_string())?;
        visibility.push(frame.is_some_and(|f| f.output));
    }
    if visibility.len() != max_packets {
        return Err("HEVC visibility is missing source samples".into());
    }
    Ok(visibility)
}
impl HevcNvdecDecoder {
    pub fn new(
        sps: Sps,
        pps: Pps,
        length_size: u8,
        ordinal: usize,
        decode_surfaces: u32,
        output_surfaces: u32,
        max_bytes: usize,
    ) -> Result<Self, String> {
        let [width, height] = sps.dimensions;
        let depth = sps.depth[0];
        let scheduler = Scheduler::new(sps, pps, length_size, decode_surfaces, max_bytes)?;
        let session = NvdecSession::open(
            CodecDevice::new(ordinal)?,
            NvdecCodec::Hevc,
            depth,
            width,
            height,
            decode_surfaces,
            output_surfaces,
        )?;
        Ok(Self {
            scheduler,
            session,
            mapped: Vec::new(),
            failed: false,
        })
    }
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<DecodedHevc>, String> {
        if self.failed {
            return Err("HEVC NVDEC submission failed; reopen decoder".into());
        }
        let Some(pending) = self.scheduler.prepare(packet)? else {
            return Ok(None);
        };
        // SAFETY: Matching session configuration, owned bytes and reserved live slots.
        if let Err(error) = unsafe { pending.picture.submit(&mut self.session) } {
            self.failed = true;
            return Err(error);
        }
        Ok(Some(self.scheduler.commit(pending)))
    }
    pub fn map(&mut self, frame: &DecodedHevc) -> Result<NvdecSurface, String> {
        if self.failed
            || !self.scheduler.owns(frame)
            || self.mapped.iter().any(|e| Arc::ptr_eq(&e.1, &frame.slot))
        {
            return Err("HEVC mapping needs an unmapped live ticket from this decoder".into());
        }
        self.mapped.try_reserve(1).map_err(|e| e.to_string())?;
        // SAFETY: Successfully decoded ticket reserves this session's picture.
        let surface = match unsafe { self.session.map_progressive(frame.slot.index) } {
            Ok(surface) => surface,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        self.mapped.push((surface.slot, frame.slot.clone()));
        Ok(surface)
    }
    pub fn unmap(&mut self, slot: usize) -> Result<(), String> {
        self.session.unmap(slot)?;
        self.mapped.retain(|e| e.0 != slot);
        Ok(())
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.session.close()?;
        self.mapped.clear();
        self.failed = true;
        Ok(())
    }
}
