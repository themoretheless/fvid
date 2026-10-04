//! Native AVC NVDEC scheduling backed by FVid's POC and reference marking.
use super::owned_nvdec_avc::{AvcPicture, AvcReference};
use fvid_codecs::codec::{
    avc::{Pps, Sps},
    avc_access_unit,
    avc_dpb::ReferenceBuffer,
    avc_poc::PocDecoder,
    avc_slice::{MemoryOperation, SliceType},
};
use fvid_cuda::{CodecDevice, NvdecCodec, NvdecSession, NvdecSurface};
use std::sync::{Arc, Weak};

#[derive(Clone)]
struct FrameSlot {
    index: u32,
    field_order: [i32; 2],
}
/// Holding this ticket reserves the decode slot, including after DPB eviction.
#[derive(Clone)]
pub struct DecodedAvc {
    slot: Arc<FrameSlot>,
    pub picture_order: i32,
}
struct Scheduler {
    sps: Sps,
    pps: Pps,
    length_size: u8,
    max_bytes: usize,
    poc: PocDecoder,
    references: ReferenceBuffer<FrameSlot>,
    slots: Vec<Weak<FrameSlot>>,
    next_id: u64,
    previous_reference: Option<u32>,
}
struct Pending {
    picture: AvcPicture,
    frame: DecodedAvc,
    poc: PocDecoder,
    references: ReferenceBuffer<FrameSlot>,
    next_id: u64,
    previous_reference: Option<u32>,
}
impl Scheduler {
    fn new(
        sps: Sps,
        pps: Pps,
        length_size: u8,
        capacity: u32,
        max_bytes: usize,
    ) -> Result<Self, String> {
        if !(1..=4).contains(&length_size)
            || capacity == 0
            || capacity > 64
            || capacity <= sps.max_num_ref_frames
            || max_bytes == 0
        {
            return Err(
                "NVDEC AVC needs valid NAL length, byte limit and a spare decode slot".into(),
            );
        }
        let references = ReferenceBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sps,
            pps,
            length_size,
            max_bytes,
            poc: PocDecoder::new(),
            references,
            slots: (0..capacity).map(|_| Weak::new()).collect(),
            next_id: 0,
            previous_reference: None,
        })
    }
    /// Compute all fallible codec state before submitting work to the driver.
    fn prepare(&mut self, packet: &[u8]) -> Result<Pending, String> {
        let slices = avc_access_unit::prepare(
            packet,
            self.length_size,
            &self.sps,
            &self.pps,
            self.max_bytes,
        )
        .map_err(|e| e.to_string())?;
        let header = &slices.first().ok_or("NVDEC AVC missing picture")?.header;
        if !matches!(
            header.slice_type,
            SliceType::I | SliceType::P | SliceType::B
        ) {
            return Err("NVDEC AVC SP/SI scheduling is not implemented".into());
        }
        if !header.idr
            && self.previous_reference.is_some_and(|previous| {
                header.frame_num != previous
                    && header.frame_num != (previous + 1) % (1 << self.sps.frame_num_bits)
            })
        {
            return Err("NVDEC AVC frame-number gaps need inferred-reference scheduling".into());
        }
        let index = self
            .slots
            .iter()
            .position(|slot| slot.strong_count() == 0)
            .ok_or("NVDEC AVC decode slots are retained by references/display/mappings")?
            as u32;
        let mut poc = self.poc.clone();
        let order = poc.decode(&self.sps, header).map_err(|e| e.to_string())?;
        let mut references = if header.idr {
            ReferenceBuffer::new(self.sps.frame_num_bits, self.sps.max_num_ref_frames)
                .map_err(|e| e.to_string())?
        } else {
            self.references.clone()
        };
        for slice in &slices {
            references
                .lists(&slice.header, order.before_marking.picture())
                .map_err(|e| e.to_string())?;
        }
        let gpu_references: Vec<_> = references
            .references()
            .into_iter()
            .map(|r| {
                let slot = references
                    .get(r.id)
                    .expect("DPB descriptor has a retained slot");
                AvcReference {
                    slot: slot.index,
                    frame_index: r.long_term_index.unwrap_or(r.frame_num) as i32,
                    long_term: r.long_term_index.is_some(),
                    non_existing: false,
                    field_order: slot.field_order,
                }
            })
            .collect();
        let picture = AvcPicture::prepare(
            &self.sps,
            &self.pps,
            &slices,
            index,
            [
                order.before_marking.top.ok_or("missing top POC")?,
                order.before_marking.bottom.ok_or("missing bottom POC")?,
            ],
            &gpu_references,
            self.max_bytes,
        )?;
        let slot = Arc::new(FrameSlot {
            index,
            field_order: [
                order.after_marking.top.ok_or("missing marked top POC")?,
                order
                    .after_marking
                    .bottom
                    .ok_or("missing marked bottom POC")?,
            ],
        });
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or("NVDEC AVC picture ID overflow")?;
        references
            .finish(
                header,
                order.after_marking.picture(),
                self.next_id,
                slot.clone(),
            )
            .map_err(|e| e.to_string())?;
        let previous_reference = if header.nal_ref_idc != 0 {
            Some(
                if header
                    .memory_operations
                    .iter()
                    .any(|op| matches!(op, MemoryOperation::Reset))
                {
                    0
                } else {
                    header.frame_num
                },
            )
        } else {
            self.previous_reference
        };
        self.slots[index as usize] = Arc::downgrade(&slot);
        Ok(Pending {
            picture,
            frame: DecodedAvc {
                slot,
                picture_order: order.before_marking.picture(),
            },
            poc,
            references,
            next_id,
            previous_reference,
        })
    }
    fn commit(&mut self, pending: Pending) -> DecodedAvc {
        self.poc = pending.poc;
        self.references = pending.references;
        self.next_id = pending.next_id;
        self.previous_reference = pending.previous_reference;
        pending.frame
    }
    fn owns(&self, frame: &DecodedAvc) -> bool {
        self.slots
            .get(frame.slot.index as usize)
            .and_then(Weak::upgrade)
            .is_some_and(|slot| Arc::ptr_eq(&slot, &frame.slot))
    }
}
/// Direct decoder with owned reference state. Output ordering/timestamps remain
/// the demuxer's responsibility; no libav fallback is opened.
pub struct AvcNvdecDecoder {
    scheduler: Scheduler,
    session: NvdecSession,
    mapped: Vec<(usize, Arc<FrameSlot>)>,
    failed: bool,
}
impl AvcNvdecDecoder {
    pub fn new(
        sps: Sps,
        pps: Pps,
        length_size: u8,
        ordinal: usize,
        decode_surfaces: u32,
        output_surfaces: u32,
        max_bytes: usize,
    ) -> Result<Self, String> {
        let (width, height) = sps.coded_dimensions();
        let scheduler = Scheduler::new(sps, pps, length_size, decode_surfaces, max_bytes)?;
        let session = NvdecSession::open(
            CodecDevice::new(ordinal)?,
            NvdecCodec::H264,
            8,
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
    pub fn decode(&mut self, packet: &[u8]) -> Result<DecodedAvc, String> {
        if self.failed {
            return Err("NVDEC AVC driver submission failed; reopen decoder".into());
        }
        let pending = self.scheduler.prepare(packet)?;
        // SAFETY: Scheduler owns matching SPS/PPS, reserved reference slots and
        // byte arrays; session was created with the same geometry and codec.
        if let Err(error) = unsafe { pending.picture.submit(&mut self.session) } {
            self.failed = true;
            return Err(error);
        }
        Ok(self.scheduler.commit(pending))
    }
    /// Returns a borrowed CUDA pointer. All use must finish before unmap/close.
    pub fn map(&mut self, frame: &DecodedAvc) -> Result<NvdecSurface, String> {
        if self.failed || !self.scheduler.owns(frame) {
            return Err("NVDEC AVC mapping needs a live ticket from this decoder".into());
        }
        if self
            .mapped
            .iter()
            .any(|entry| Arc::ptr_eq(&entry.1, &frame.slot))
        {
            return Err("NVDEC AVC picture is already mapped".into());
        }
        self.mapped.try_reserve(1).map_err(|e| e.to_string())?;
        // SAFETY: Ticket owns this successfully submitted picture and reserves
        // its slot. Mapping pin below also retains it if caller drops the ticket.
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
        self.mapped.retain(|entry| entry.0 != slot);
        Ok(())
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.session.close()?;
        self.mapped.clear();
        self.failed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned_mp4::{Limits, Mp4Reader};
    use fvid_codecs::codec::config::AvcConfig;
    use std::io::Cursor;
    const CONTROL: &[u8] = include_bytes!("../../../tests/fixtures/playback-errors/control.mp4");
    const IPB: &[u8] =
        include_bytes!("../../../tests/fixtures/playback-errors/avc-multislice-ipb.mp4");
    fn input(data: &[u8]) -> (Mp4Reader<Cursor<&[u8]>>, Scheduler) {
        let reader = Mp4Reader::open(Cursor::new(data), Limits::default()).unwrap();
        let config = AvcConfig::parse(&reader.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let scheduler = Scheduler::new(sps, pps, config.length_size, 32, 1 << 20).unwrap();
        (reader, scheduler)
    }

    #[test]
    fn retained_slots_apply_backpressure_without_advancing_codec_state() {
        let (mut reader, mut state) = input(CONTROL);
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let held: Vec<_> = (0..state.slots.len())
            .map(|index| {
                let slot = Arc::new(FrameSlot {
                    index: index as u32,
                    field_order: [0, 0],
                });
                state.slots[index] = Arc::downgrade(&slot);
                slot
            })
            .collect();
        assert!(state.prepare(&packet).err().unwrap().contains("retained"));
        assert_eq!(state.next_id, 0);
        drop(held);
        assert!(state.prepare(&packet).is_ok());
    }
    #[test]
    fn owned_poc_and_dpb_schedule_every_synthetic_ipb_picture() {
        for data in [CONTROL, IPB] {
            let (mut reader, mut state) = input(data);
            let count = reader.tracks()[0].samples.len();
            assert!(count > 1);
            let mut packet = Vec::new();
            let mut display = Vec::new();
            for index in 0..count {
                reader.read_packet(0, index, &mut packet).unwrap();
                let pending = state.prepare(&packet).unwrap();
                assert_eq!(state.next_id, index as u64);
                let frame = state.commit(pending);
                assert!(state.owns(&frame));
                assert!(
                    display
                        .iter()
                        .all(|other: &DecodedAvc| other.slot.index != frame.slot.index)
                );
                display.push(frame);
            }
            assert_eq!(state.next_id, count as u64);
            for r in state.references.references() {
                assert!(state.references.get(r.id).is_some());
            }
        }
    }
    #[test]
    fn failed_preparation_preserves_state_and_display_tickets_pin_slots() {
        let (mut reader, mut state) = input(CONTROL);
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let pending = state.prepare(&packet).unwrap();
        let index = pending.frame.slot.index;
        drop(pending);
        assert_eq!(state.next_id, 0);
        assert!(state.references.references().is_empty());
        assert_eq!(state.slots[index as usize].strong_count(), 0);
        assert!(state.prepare(&[0]).is_err());
        assert_eq!(state.next_id, 0);
        let pending = state.prepare(&packet).unwrap();
        let frame = state.commit(pending);
        assert!(state.owns(&frame));
        let (other_reader, other) = input(CONTROL);
        drop(other_reader);
        assert!(!other.owns(&frame));
        // Even after DPB eviction, a display ticket must reserve its slot.
        state.references =
            ReferenceBuffer::new(state.sps.frame_num_bits, state.sps.max_num_ref_frames).unwrap();
        assert_eq!(state.slots[index as usize].strong_count(), 1);
        drop(frame);
        assert_eq!(state.slots[index as usize].strong_count(), 0);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC and NVENC"]
    fn synthetic_avc_decode_filter_encode_chain_without_libav() {
        use fvid_cuda::{Nv12Buffer, Nv12Processor, Nv12Transform, Nv12View, NvencSession};
        let (mut reader, state) = input(CONTROL);
        let (width, height) = state.sps.coded_dimensions();
        let mut decoder =
            AvcNvdecDecoder::new(state.sps, state.pps, state.length_size, 0, 32, 2, 1 << 20)
                .unwrap();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let frame = decoder.decode(&packet).unwrap();
        let source = decoder.map(&frame).unwrap();
        let src = Nv12View {
            y: source.pointer,
            uv: source
                .pointer
                .checked_add(u64::from(source.pitch) * u64::from(height))
                .unwrap(),
            pitch_y: source.pitch,
            pitch_uv: source.pitch,
            width,
            height,
        };
        let output = Nv12Buffer::new(0, width, height).unwrap();
        let mut filter = Nv12Processor::new(0).unwrap();
        filter.follow_stream(output.stream_handle().unwrap());
        filter
            .apply(
                src,
                output.view().unwrap(),
                Nv12Transform {
                    crop_x: 0,
                    crop_y: 0,
                    out_width: width,
                    out_height: height,
                    hflip: true,
                    vflip: true,
                },
            )
            .unwrap();
        output.synchronize().unwrap();
        decoder.unmap(source.slot).unwrap();
        let view = output.view().unwrap();
        let mut encoder = NvencSession::open(CodecDevice::new(0).unwrap()).unwrap();
        encoder.initialize_h264(width, height, 60, 1).unwrap();
        // SAFETY: Allocation shares primary context, filtering is complete,
        // and output remains live until successful encoder close.
        let input =
            unsafe { encoder.register_nv12(view.y, view.pitch_y, output.byte_len() as u64) }
                .unwrap();
        let slot = encoder.create_output().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while encoder.submit_nv12(input, slot, 0, 1).unwrap() == fvid_cuda::NvencSubmit::Busy {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        encoder.finish().unwrap();
        let packet = loop {
            if let Some(packet) = encoder.receive().unwrap() {
                break packet;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert!(!packet.bytes.is_empty());
        assert_eq!((packet.timestamp, packet.duration), (0, 1));
        let converted = crate::owned_avc_annexb::convert(&packet.bytes, 1 << 20).unwrap();
        let configuration = converted
            .configuration
            .as_ref()
            .expect("NVENC first IDR must carry SPS/PPS");
        let mut container = std::io::Cursor::new(Vec::new());
        let tracks = [crate::owned_matroska::TrackSpec {
            encoding: crate::owned_matroska::Encoding::Avc {
                configuration,
                width,
                height,
            },
            name: "",
            language: "und",
        }];
        let mut writer = crate::owned_matroska::PacketWriter::new(&mut container, &tracks).unwrap();
        writer
            .write_packet(0, 0, 16_666_667, converted.sync, &converted.sample)
            .unwrap();
        writer.finish().unwrap();
        let mut saved = crate::owned_webm::WebmReader::open(
            std::io::Cursor::new(container.into_inner()),
            Default::default(),
        )
        .unwrap();
        let payload = saved.read_packet(0).unwrap();
        assert_eq!(payload, converted.sample);
        let mut software =
            fvid_codecs::codec::avc_decoder::AvcDecoder::new(configuration, 16 << 20).unwrap();
        let picture = software.decode_order(&payload).unwrap().unwrap();
        assert_eq!(picture.dimensions(), (width as usize, height as usize));

        encoder.close().unwrap();
        decoder.close().unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn synthetic_ipb_packets_decode_map_and_release_without_libav() {
        let (mut reader, state) = input(IPB);
        let mut decoder =
            AvcNvdecDecoder::new(state.sps, state.pps, state.length_size, 0, 32, 2, 1 << 20)
                .unwrap();
        let mut packet = Vec::new();
        for index in 0..reader.tracks()[0].samples.len() {
            reader.read_packet(0, index, &mut packet).unwrap();
            let frame = decoder.decode(&packet).unwrap();
            let surface = decoder.map(&frame).unwrap();
            assert_ne!(surface.pointer, 0);
            assert!(surface.pitch >= surface.width);
            decoder.unmap(surface.slot).unwrap();
        }
        decoder.close().unwrap();
        assert!(decoder.decode(&packet).is_err());
    }
}
