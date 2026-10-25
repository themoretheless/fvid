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
    ols: usize,
    budget: usize,
) -> Result<()> {
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
    ols: usize,
    budget: usize,
) -> Result<Vec<Epoch>> {
    let mut parameters = LayeredParameters::from_configuration(configuration, budget)?;
    let mut epochs = Vec::new();
    let mut vps = initial;
    let mut first = 0;
    let mut ordinal = 0;
    let mut scan = SubstreamScan::default();
    let mut seen_vcl = false;
    let mut boundary = false;
    for i in 0..demux.packets.len() {
        if demux.packets[i].track != track {
            continue;
        }
        if boundary {
            retain_epoch(&mut epochs, first, scan, vps.clone(), ols, budget)?;
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
                    retain_epoch(&mut epochs, first, scan, vps.clone(), ols, budget)?;
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
    retain_epoch(&mut epochs, first, scan, vps, ols, budget)?;
    Ok(epochs)
}
pub(super) struct Layered {
    decoder: LayeredDecoder,
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
        // Matroska permits parameter banks to arrive in CodecPrivate or in
        // early blocks. Resolve the first coded slice's actual PPS/SPS/VPS.
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
        'packets: for i in 0..demux.packets.len() {
            if demux.packets[i].track != track {
                continue;
            }
            if demux.packet_payload_size(i)? > budget / 8 {
                return Err(invalid("Matroska HEVC dispatch packet exceeds budget"));
            }
            let packet = demux.read_packet(i)?;
            for nal in crate::codec::config::NalUnits::new(&packet, cfg.length_size)? {
                let nal = nal?;
                let h = crate::codec::hevc_nal::NalHeader::parse(nal)?;
                if h.layer_id == 63 {
                    continue;
                }
                if matches!(h.unit_type, 32..=34) {
                    parameters.receive(nal)?;
                }
                if h.is_vcl() {
                    active = Some(copy_vps(parameters.referenced_vps(nal)?)?);
                    break 'packets;
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
        let extension = vps
            .extension
            .as_ref()
            .ok_or_else(|| invalid("layered Matroska HEVC lacks a VPS extension"))?;
        let ols = extension
            .output
            .sets
            .iter()
            .enumerate()
            .filter(|(_, s)| s.output != 0)
            .max_by_key(|(i, s)| {
                (
                    63 - s.output.leading_zeros(),
                    std::cmp::Reverse(s.output.count_ones()),
                    std::cmp::Reverse(s.necessary.count_ones()),
                    std::cmp::Reverse(*i),
                )
            })
            .map(|(i, _)| i)
            .ok_or_else(|| invalid("layered Matroska HEVC lacks an output layer set"))?;
        drop(parameters);
        let selections = inventory(
            demux,
            track,
            configuration,
            cfg.length_size,
            Arc::new(vps),
            ols,
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
        Ok(Some(Self {
            decoder: LayeredDecoder::from_configuration(configuration, ols, decode_budget)?,
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
        let epoch = self
            .selections
            .partition_point(|e| e.first <= self.packet_index)
            .saturating_sub(1);
        let selection = &self.selections[epoch].selection;
        let (pictures, _) = self.decoder.decode_selected_queued_packet(
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
        while let Some(p) = self.ready.pop_front() {
            let &(pts, duration, invisible) = self
                .timeline
                .get(
                    usize::try_from(p.access_unit)
                        .map_err(|_| invalid("Matroska layered AU index overflow"))?,
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
