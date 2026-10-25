//! Matroska's complete-AU timestamps around the owned layered HEVC scheduler.
use super::*;
use crate::codec::{
    config::HevcConfig,
    hevc_decoder::{
        LayeredDecoded, LayeredDecoder, LayeredOutputQueue, LayeredParameters, LayeredSubstream,
        SubstreamScan,
    },
    hevc_vps::{LayeredVps, Vps},
};
use std::collections::VecDeque;
struct Epoch {
    first: usize,
    vps: Arc<LayeredVps>,
    scan: SubstreamScan,
    selection: LayeredSubstream,
}
fn selection_bytes(epochs: &Vec<Epoch>) -> usize {
    epochs
        .capacity()
        .saturating_mul(std::mem::size_of::<Epoch>())
        .saturating_add(
            epochs
                .iter()
                .map(|e| e.selection.retained_bytes())
                .fold(0usize, usize::saturating_add),
        )
}
fn retain_epoch(
    epochs: &mut Vec<Epoch>,
    first: usize,
    scan: SubstreamScan,
    vps: Arc<LayeredVps>,
    budget: usize,
) -> Result<()> {
    let ols = vps.automatic_output_layer_set()?;
    let rows = vps
        .extension
        .as_ref()
        .and_then(|e| e.dpb.sets.get(ols))
        .and_then(Option::as_ref)
        .map_or(vps.vps.ordering.len(), Vec::len);
    if rows == 0 {
        return Err(invalid("Matroska layered VPS has no temporal ordering"));
    }
    let merge = epochs.last().is_some_and(|last| last.vps == vps);
    let scan = if merge {
        epochs.last().unwrap().scan.merged(scan)
    } else {
        scan
    };
    let selection = scan.select(vps.clone(), ols, (rows - 1) as u8)?;
    if merge {
        let last = epochs.last_mut().unwrap();
        last.scan = scan;
        last.selection = selection;
    } else {
        epochs
            .try_reserve_exact(1)
            .map_err(|_| invalid("cannot allocate Matroska HEVC epochs"))?;
        epochs.push(Epoch {
            first,
            vps,
            scan,
            selection,
        });
    }
    if selection_bytes(epochs) > budget {
        return Err(invalid("Matroska HEVC epochs exceed budget"));
    }
    Ok(())
}
fn inventory<R: Read + Seek>(
    demux: &mut WebmReader<R>,
    track: u64,
    configuration: &[u8],
    length: u8,
    initial: Arc<LayeredVps>,
    start: usize,
    budget: usize,
) -> Result<Vec<Epoch>> {
    let mut parameters = LayeredParameters::from_configuration(configuration, budget)?;
    let mut epochs = Vec::new();
    let mut vps = initial;
    let mut first = start;
    let mut ordinal = 0;
    let mut scan = SubstreamScan::default();
    let mut seen_vcl = false;
    let mut boundary = false;
    for i in 0..demux.packets.len() {
        if demux.packets[i].track != track {
            continue;
        }
        if ordinal < start {
            ordinal += 1;
            continue;
        }
        if boundary {
            retain_epoch(&mut epochs, first, scan, vps.clone(), budget)?;
            first = ordinal;
            scan = Default::default();
            seen_vcl = false;
            boundary = false;
        }
        if demux.packet_payload_size(i)? > budget / 2 {
            return Err(invalid("Matroska HEVC inventory packet exceeds budget"));
        }
        let packet = demux.read_packet(i)?;
        let mut candidate = None;
        for nal in crate::codec::config::NalUnits::new(&packet, length)? {
            let nal = nal?;
            let h = crate::codec::hevc_nal::NalHeader::parse(nal)?;
            if h.layer_id == 63 {
                continue;
            }
            if matches!(h.unit_type, 32..=34) {
                parameters.receive(nal)?;
            }
            if h.is_vcl() && candidate.is_none() {
                let available = budget
                    .checked_sub(selection_bytes(&epochs).saturating_add(vps.retained_bytes()))
                    .ok_or_else(|| invalid("Matroska HEVC inventory exceeds budget"))?;
                candidate = Some(Arc::new(LayeredVps::parse(
                    parameters.referenced_vps(nal)?,
                    available,
                )?));
            }
        }
        if let Some(received) = candidate {
            if *received != *vps {
                if seen_vcl {
                    retain_epoch(&mut epochs, first, scan, vps.clone(), budget)?;
                    first = ordinal;
                    scan = Default::default();
                    seen_vcl = false;
                }
                vps = received;
            }
        }
        scan.observe(&packet, length)?;
        for nal in crate::codec::config::NalUnits::new(&packet, length)? {
            let h = crate::codec::hevc_nal::NalHeader::parse(nal?)?;
            if h.layer_id == 63 {
                continue;
            }
            seen_vcl |= h.is_vcl();
            boundary |= h.unit_type == 37 || h.unit_type == 36 && h.layer_id == 0;
        }
        ordinal += 1;
    }
    retain_epoch(&mut epochs, first, scan, vps, budget)?;
    Ok(epochs)
}
pub(super) struct Layered {
    decoder: LayeredDecoder,
    prefix: Option<Box<HevcDecoder>>,
    prefix_end: usize,
    prefix_ready: Option<Frame>,
    queue: LayeredOutputQueue,
    timeline: Vec<(i64, Option<u64>, bool)>,
    ready: VecDeque<LayeredDecoded>,
    selections: Vec<Epoch>,
    packet_index: usize,
    ready_budget: usize,
    ended: bool,
    pub signal: ColourDescription,
    pub hdr: HdrMetadata,
}
impl Layered {
    pub fn open<R: Read + Seek>(
        configuration: &[u8],
        demux: &mut WebmReader<R>,
        track: u64,
        budget: usize,
    ) -> Result<Option<Self>> {
        let cfg = HevcConfig::parse(configuration)?;
        let copy_vps = |nal: &[u8]| -> Result<Vec<u8>> {
            if nal.len() > budget / 8 {
                return Err(invalid("Matroska HEVC dispatch VPS exceeds budget"));
            }
            let mut out = Vec::new();
            out.try_reserve_exact(nal.len())
                .map_err(|_| invalid("cannot allocate Matroska dispatch VPS"))?;
            out.extend_from_slice(nal);
            Ok(out)
        };
        // Resolve activated VPSs, including a base-only prefix before a later
        // multilayer CVS. Receiving an unused VPS does not select this path.
        let mut parameters = crate::codec::hevc_decoder::LayeredParameters::from_configuration(
            configuration,
            budget / 2,
        )?;
        let mut active = cfg
            .arrays
            .iter()
            .filter(|a| a.nal_type == 32)
            .flat_map(|a| a.units.iter())
            .next()
            .map(|n| copy_vps(n))
            .transpose()?;
        let mut seen_vcl = false;
        let mut vpsless = false;
        let mut prefix_ended = false;
        let mut activated_vps = false;
        let mut first_vps = None;
        let mut ordinal = 0;
        'packets: for i in 0..demux.packets.len() {
            if demux.packets[i].track != track {
                continue;
            }
            if demux.packet_payload_size(i)? > budget / 8 {
                return Err(invalid("Matroska HEVC dispatch packet exceeds budget"));
            }
            let packet_ordinal = ordinal;
            ordinal += 1;
            let packet = demux.read_packet(i)?;
            for nal in crate::codec::config::NalUnits::new(&packet, cfg.length_size)? {
                let nal = nal?;
                let h = crate::codec::hevc_nal::NalHeader::parse(nal)?;
                if h.layer_id == 63 {
                    continue;
                }
                if h.unit_type == 37 || h.unit_type == 36 && h.layer_id == 0 {
                    prefix_ended = true;
                }
                if matches!(h.unit_type, 32..=34) {
                    parameters.receive(nal)?;
                }
                if h.unit_type == 32 {
                    first_vps.get_or_insert(packet_ordinal);
                }
                if h.is_vcl() {
                    let Some(referenced) = parameters.referenced_vps_if_available(nal)? else {
                        h.require_base_layer()?;
                        vpsless = true;
                        prefix_ended = false;
                        seen_vcl = true;
                        continue;
                    };
                    if !activated_vps {
                        if vpsless && !prefix_ended && !matches!(h.unit_type, 16..=20) {
                            return Err(invalid("HEVC active VPS changed before sequence end"));
                        }
                        activated_vps = true;
                    }
                    let layered = Vps::parse_layered_metadata(referenced, budget / 2)?
                        .layers
                        .effective_max_layers()
                        > 1;
                    if !seen_vcl || layered {
                        active = Some(copy_vps(referenced)?);
                        seen_vcl = true;
                    }
                    if layered {
                        break 'packets;
                    }
                }
            }
        }
        let Some(nal) = active else {
            return Ok(None);
        };
        if Vps::parse_layered_metadata(&nal, budget / 2)?
            .layers
            .effective_max_layers()
            <= 1
        {
            return Ok(None);
        }
        let vps = LayeredVps::parse(&nal, budget / 2)?;
        let ols = vps.automatic_output_layer_set()?;
        let received_configuration = if vpsless {
            Some(parameters.received_configuration(configuration, budget / 8)?)
        } else {
            None
        };
        drop(parameters);
        let layered_configuration = received_configuration.as_deref().unwrap_or(configuration);
        let prefix_end = if vpsless {
            first_vps.ok_or_else(|| invalid("Matroska layered activation has no received VPS"))?
        } else {
            0
        };
        let selections = inventory(
            demux,
            track,
            layered_configuration,
            cfg.length_size,
            Arc::new(vps),
            prefix_end,
            budget / 4,
        )?;
        let count = demux.packets.iter().filter(|p| p.track == track).count();
        let bytes = count
            .checked_mul(std::mem::size_of::<(i64, Option<u64>, bool)>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| invalid("Matroska layered timeline overflow"))?;
        if bytes > budget / 2 {
            return Err(invalid("Matroska layered timeline exceeds budget"));
        }
        let mut timeline = Vec::new();
        timeline
            .try_reserve_exact(count)
            .map_err(|_| invalid("cannot allocate Matroska layered timeline"))?;
        timeline.extend(
            demux
                .packets
                .iter()
                .filter(|p| p.track == track)
                .map(|p| (p.pts_ns, p.duration_ns, p.invisible)),
        );
        let owned_timeline = timeline
            .capacity()
            .checked_mul(std::mem::size_of::<(i64, Option<u64>, bool)>())
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| invalid("Matroska layered timeline capacity overflow"))?;
        let decode_budget = (budget / 2)
            .checked_sub(owned_timeline.saturating_add(selection_bytes(&selections)))
            .ok_or_else(|| invalid("Matroska layered timeline capacity exceeds budget"))?;
        let prefix_budget = if prefix_end > 0 { decode_budget / 2 } else { 0 };
        Ok(Some(Self {
            decoder: LayeredDecoder::from_configuration(
                layered_configuration,
                ols,
                decode_budget - prefix_budget,
            )?,
            prefix: if prefix_end > 0 {
                Some(Box::new(HevcDecoder::from_inband_configuration(
                    configuration,
                    prefix_budget,
                )?))
            } else {
                None
            },
            prefix_end,
            prefix_ready: None,
            queue: Default::default(),
            timeline,
            ready: VecDeque::new(),
            selections,
            packet_index: 0,
            ready_budget: budget - budget / 2,
            ended: false,
            signal: Default::default(),
            hdr: Default::default(),
        }))
    }
    pub fn reset(&mut self) {
        self.decoder.reset_queued(&mut self.queue);
        if let Some(p) = &mut self.prefix {
            p.reset();
        }
        self.prefix_ready = None;
        self.ready.clear();
        self.packet_index = 0;
        self.ended = false;
        self.signal = Default::default();
        self.hdr = Default::default();
    }
    fn retain(&mut self, pictures: Vec<LayeredDecoded>) -> Result<()> {
        // One display view per AU, using the same highest-output-layer policy
        // as the MP4 frontend. Lower output views remain owned by the codec.
        let mut chosen: Vec<LayeredDecoded> = Vec::new();
        for p in pictures {
            if let Some(previous) = chosen.last_mut().filter(|v| v.access_unit == p.access_unit) {
                if p.layer_id > previous.layer_id {
                    *previous = p;
                }
            } else {
                chosen
                    .try_reserve(1)
                    .map_err(|_| invalid("cannot allocate Matroska layered views"))?;
                chosen.push(p);
            }
        }
        let held = self
            .ready
            .iter()
            .chain(chosen.iter())
            .try_fold(0usize, |n, p| {
                p.retained_bytes().and_then(|v| {
                    n.checked_add(v)
                        .ok_or_else(|| invalid("Matroska layered output size overflow"))
                })
            })?;
        if held > self.ready_budget {
            return Err(invalid("Matroska layered output exceeds budget"));
        }
        self.ready
            .try_reserve_exact(chosen.len())
            .map_err(|_| invalid("cannot allocate Matroska layered output"))?;
        let metadata = self
            .ready
            .capacity()
            .checked_add(chosen.capacity())
            .and_then(|n| n.checked_mul(std::mem::size_of::<LayeredDecoded>()))
            .and_then(|n| n.checked_add(held))
            .filter(|&n| n <= self.ready_budget);
        if metadata.is_none() {
            return Err(invalid("Matroska layered output capacity exceeds budget"));
        }
        self.ready.extend(chosen);
        Ok(())
    }
    pub fn push(&mut self, packet: &[u8]) -> Result<()> {
        if self.packet_index < self.prefix_end {
            if self.prefix_ready.is_some() {
                return Err(invalid("Matroska VPS-less prefix output must be consumed"));
            }
            let decoder = self.prefix.as_mut().unwrap();
            let decoded = decoder.decode_packet(packet)?;
            let (pts, duration, invisible) = self.timeline[self.packet_index];
            self.packet_index += 1;
            if let Some(decoded) = decoded.filter(|p| p.output && !invisible) {
                let s = &decoder.parameters().0;
                let packed = crate::playback_native::hevc_picture_with_vui(
                    &decoded.picture,
                    s.chroma_format,
                    s.vui.as_ref(),
                )?;
                if packed.frame.data.len() > self.ready_budget {
                    return Err(invalid("Matroska VPS-less prefix display exceeds budget"));
                }
                let signal = s
                    .vui
                    .as_ref()
                    .and_then(|v| v.signal.as_ref())
                    .map(|v| {
                        let [primaries, transfer, matrix] = v.colour.unwrap_or([0; 3]);
                        ColourDescription {
                            primaries,
                            transfer,
                            matrix,
                            full_range: v.full_range,
                        }
                    })
                    .unwrap_or_default();
                self.prefix_ready = Some(Frame {
                    layer_id: Some(0),
                    signal: Some(signal),
                    paired: false,
                    hdr: decoder.hdr(),
                    decoded: Picture::Packed(Arc::new(packed)),
                    pts,
                    duration,
                });
            }
            return Ok(());
        }
        let epoch = self
            .selections
            .partition_point(|e| e.first <= self.packet_index)
            .saturating_sub(1);
        let selection = &self.selections[epoch].selection;
        let (pictures, _) = self.decoder.decode_reselected_queued_packet(
            &mut self.queue,
            selection,
            packet,
            selection.highest_temporal_id(),
        )?;
        self.packet_index += 1;
        self.retain(pictures)
    }
    pub fn finish(&mut self) -> Result<()> {
        if !self.ended {
            let tail = self.queue.drain();
            self.retain(tail)?;
            self.ended = true;
        }
        Ok(())
    }
    pub fn take(&mut self) -> Result<Option<Frame>> {
        if let Some(frame) = self.prefix_ready.take() {
            self.signal = frame.signal.unwrap_or_default();
            self.hdr = frame.hdr;
            return Ok(Some(frame));
        }
        while let Some(p) = self.ready.pop_front() {
            let &(pts, duration, invisible) = self
                .timeline
                .get(
                    usize::try_from(p.access_unit)
                        .map_err(|_| invalid("Matroska layered AU index overflow"))?
                        .checked_add(self.prefix_end)
                        .ok_or_else(|| invalid("Matroska layered AU index overflow"))?,
                )
                .ok_or_else(|| invalid("Matroska layered output has no source timestamp"))?;
            if invisible {
                continue;
            }
            let s = &p.binding.sps.sps;
            let packed = crate::playback_native::hevc_picture_with_vui(
                &p.picture,
                s.chroma_format,
                s.vui.as_ref(),
            )?;
            self.signal = s
                .vui
                .as_ref()
                .and_then(|v| v.signal.as_ref())
                .map(|v| {
                    let [primaries, transfer, matrix] = v.colour.unwrap_or([0; 3]);
                    ColourDescription {
                        primaries,
                        transfer,
                        matrix,
                        full_range: v.full_range,
                    }
                })
                .unwrap_or_default();
            self.hdr = p.hdr;
            return Ok(Some(Frame {
                layer_id: Some(p.layer_id),
                signal: Some(self.signal),
                paired: false,
                hdr: p.hdr,
                decoded: Picture::Packed(Arc::new(packed)),
                pts,
                duration,
            }));
        }
        Ok(None)
    }
}
