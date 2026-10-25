//! Indexed, software-only layered HEVC source. A finite source scan pins the
//! extraction policy; all selected output layers retain their coded identity.
use super::{MediaTime, VideoFrame};
use crate::codec::{
    config::{HevcConfig, NalUnits},
    hevc_decoder::{
        LayeredDecoded, LayeredDecoder, LayeredOutputQueue, LayeredParameters, LayeredSubstream,
        SubstreamScan,
    },
    hevc_nal::NalHeader,
    hevc_vps::LayeredVps,
};
use crate::container::mp4::{Limits, Mp4Reader, Track};
use crate::{Result, invalid};
use std::{
    collections::VecDeque,
    io::{Read, Seek},
    sync::Arc,
};

pub struct LayeredVideoFrame {
    pub decoded: LayeredDecoded,
    pub presentation_time: MediaTime,
    pub duration: MediaTime,
    pub sample_index: usize,
}
impl LayeredVideoFrame {
    /// Preserve source chroma, depth, crop and this frame's immutable colour
    /// binding in the common player's packed-planar payload. The lightweight
    /// picture carries geometry; pixels belong to `packed`.
    pub fn into_video_frame(self) -> Result<VideoFrame> {
        let p = &self.decoded.picture;
        let s = &self.decoded.binding.sps.sps;
        let packed = Arc::new(crate::playback_native::hevc_picture_with_vui(
            p,
            s.chroma_format,
            s.vui.as_ref(),
        )?);
        let picture = Arc::new(crate::codec::avc_picture::IntraPicture {
            coded_width: p.dimensions[0] as usize,
            coded_height: p.dimensions[1] as usize,
            crop: p.crop.map(|n| n as usize),
            bit_depth: p.depth[0],
            chroma_bit_depth: p.depth[1],
            y: vec![],
            cb: vec![],
            cr: vec![],
        });
        Ok(VideoFrame {
            packed: Some(packed),
            picture,
            planes8: None,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            surface: None,
            presentation_time: self.presentation_time,
            duration: self.duration,
            sample_index: self.sample_index,
        })
    }
}
struct SelectionEpoch {
    first_sample: usize,
    selection: LayeredSubstream,
    scan: SubstreamScan,
    vps: Arc<LayeredVps>,
}
fn seek_storage(entries: &Vec<(usize, usize)>) -> usize {
    entries
        .capacity()
        .saturating_mul(std::mem::size_of::<(usize, usize)>())
}
fn selection_storage(epochs: &Vec<SelectionEpoch>) -> usize {
    epochs
        .capacity()
        .saturating_mul(std::mem::size_of::<SelectionEpoch>())
        .saturating_add(
            epochs
                .iter()
                .map(|e| e.selection.retained_bytes())
                .fold(0usize, usize::saturating_add),
        )
}
fn retain_selection(
    epochs: &mut Vec<SelectionEpoch>,
    first_sample: usize,
    scan: SubstreamScan,
    vps: Arc<LayeredVps>,
    ols: usize,
    automatic: bool,
    highest_tid: u8,
    budget: usize,
) -> Result<()> {
    let ols = if automatic {
        vps.automatic_output_layer_set()?
    } else {
        ols
    };
    // HighestTid is an upper bound; a later VPS may have fewer sublayers.
    let rows = vps
        .extension
        .as_ref()
        .and_then(|e| e.dpb.sets.get(ols))
        .and_then(Option::as_ref)
        .map_or(vps.vps.ordering.len(), Vec::len);
    let temporal = highest_tid.min((rows - 1) as u8);
    let merge = epochs.last().is_some_and(|last| *last.vps == *vps);
    let scan = if merge {
        epochs.last().unwrap().scan.merged(scan)
    } else {
        scan
    };
    let selection = scan.select(vps.clone(), ols, temporal)?;
    if merge {
        let previous = epochs.last().unwrap().selection.retained_bytes();
        if selection_storage(epochs)
            .saturating_sub(previous)
            .saturating_add(selection.retained_bytes())
            > budget
        {
            return Err(invalid("HEVC source selections exceed memory budget"));
        }
        let last = epochs.last_mut().unwrap();
        last.selection = selection;
        last.scan = scan;
        return Ok(());
    }
    let growth = (epochs.len() + 1).saturating_mul(std::mem::size_of::<SelectionEpoch>());
    if selection_storage(epochs)
        .saturating_add(selection.retained_bytes())
        .saturating_add(growth)
        > budget
    {
        return Err(invalid("HEVC source selections exceed memory budget"));
    }
    epochs
        .try_reserve_exact(1)
        .map_err(|_| invalid("cannot allocate HEVC source selections"))?;
    epochs.push(SelectionEpoch {
        first_sample,
        selection,
        scan,
        vps,
    });
    Ok(())
}
/// Each returned layer keeps the PTS/duration of its originating MP4 sample,
/// even when decoding a later packet releases it. Equal-PTS layers are distinct.
/// Output follows native POC ordering; container edit lists remain on track().
/// A completed inventory pins extraction separately for each sequence/VPS epoch.
/// The native parameter bank still enforces legal VPS activation boundaries.
/// Demux metadata has its own Limits; caller-owned returned frames are separate.
pub struct LayeredMp4VideoReader<R> {
    demuxer: Mp4Reader<R>,
    track_index: usize,
    decoder: LayeredDecoder,
    automatic_selection: bool,
    queue: LayeredOutputQueue,
    selections: Vec<SelectionEpoch>,
    seek_preroll: Vec<(usize, usize)>,
    visible_sample: usize,
    ready: VecDeque<LayeredDecoded>,
    ready_budget: usize,
    packet: Vec<u8>,
    raw_packet: Vec<u8>,
    packet_description: u32,
    initial_description: u32,
    length_size: u8,
    sample_index: usize,
    first_sample: usize,
    prime_until: Option<usize>,
    eof: bool,
    failed: bool,
}
impl<R: Read + Seek> LayeredMp4VideoReader<R> {
    pub fn open(
        reader: R,
        mut limits: Limits,
        output_layer_set: usize,
        highest_tid: u8,
        budget: usize,
    ) -> Result<Self> {
        // Two input/extraction buffers fit in a quarter; working decoder and
        // its native queue use half; emitted-but-unconsumed pictures a quarter.
        limits.packet_bytes = limits.packet_bytes.min(budget / 8);
        let demuxer = Mp4Reader::open_indexed(reader, limits)?;
        let track_index = demuxer
            .automatic_track_indices(&[*b"vide"])
            .into_iter()
            .find(|&index| super::layered_packet::all_hevc(&demuxer.tracks()[index]))
            .ok_or_else(|| invalid("MP4 has no HEVC video track"))?;
        Self::from_demuxer(demuxer, track_index, output_layer_set, highest_tid, budget)
    }
    pub fn from_demuxer(
        demuxer: Mp4Reader<R>,
        track_index: usize,
        output_layer_set: usize,
        highest_tid: u8,
        budget: usize,
    ) -> Result<Self> {
        Self::from_demuxer_impl(
            demuxer,
            track_index,
            output_layer_set,
            highest_tid,
            budget,
            false,
        )
    }
    pub(super) fn from_demuxer_automatic(
        demuxer: Mp4Reader<R>,
        track_index: usize,
        initial_ols: usize,
        budget: usize,
    ) -> Result<Self> {
        Self::from_demuxer_impl(demuxer, track_index, initial_ols, 6, budget, true)
    }
    fn from_demuxer_impl(
        mut demuxer: Mp4Reader<R>,
        track_index: usize,
        output_layer_set: usize,
        highest_tid: u8,
        budget: usize,
        automatic: bool,
    ) -> Result<Self> {
        let track = demuxer
            .tracks()
            .get(track_index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"vide" || !super::layered_packet::all_hevc(track) {
            return Err(invalid("selected MP4 track is not HEVC video"));
        }
        if track
            .samples
            .expanded()
            .is_some_and(|samples| samples.iter().any(|s| s.size as usize > budget / 8))
        {
            return Err(invalid("HEVC packet exceeds source memory budget"));
        }
        if track.samples.expanded().is_none() {
            return Err(invalid("HEVC video samples are not indexed"));
        }
        let initial_description = track.samples.get(0).unwrap().description_index;
        let initial_setup = track.description_for_sample(0).unwrap();
        let config = HevcConfig::parse(&initial_setup.configuration)?;
        let length_size = 4;
        let vps = config
            .arrays
            .iter()
            .filter(|a| a.nal_type == 32)
            .flat_map(|a| a.units.iter())
            .next();
        if highest_tid > 6 {
            return Err(invalid("invalid HEVC extraction target"));
        }
        let selection_budget = budget / 4;
        let mut vps = vps
            .map(|nal| LayeredVps::parse(nal, selection_budget).map(Arc::new))
            .transpose()?;
        let mut inventory_parameters =
            LayeredParameters::from_configuration(&initial_setup.configuration, budget / 8)?;
        let count = track.samples.len();
        let mut epochs = Vec::new();
        let mut seek_preroll = Vec::new();
        let mut safe_sync = 0usize;
        let mut restart_pending = true;
        let mut first_sample = 0usize;
        let mut scan = SubstreamScan::default();
        let mut seen_vcl = false;
        let mut boundary = false;
        let mut packet = Vec::new();
        let mut raw_packet = Vec::new();
        let mut packet_description = initial_description;
        // Inventory the whole sequence before extracting even its earliest SEI.
        // Metadata-only samples after EOS belong to the upcoming sequence, whose
        // VPS may arrive later. Keep their inventory with that upcoming epoch.
        for index in 0..count {
            let next = demuxer.tracks()[track_index]
                .samples
                .get(index)
                .unwrap()
                .description_index;
            let setups = &demuxer.tracks()[track_index].sample_descriptions;
            let changed = setups[packet_description as usize - 1].configuration
                != setups[next as usize - 1].configuration;
            // hvcC may arrive on a metadata-only sample before its first coded
            // picture. Keep that seed in the upcoming extraction inventory.
            boundary |= changed;
            packet_description = super::layered_packet::read_normalized(
                &mut demuxer,
                track_index,
                index,
                packet_description,
                &mut raw_packet,
                &mut packet,
                budget / 8,
            )?;
            let mut base_restart = false;
            if boundary {
                if let Some(vps) = &vps {
                    retain_selection(
                        &mut epochs,
                        first_sample,
                        scan,
                        vps.clone(),
                        output_layer_set,
                        automatic,
                        highest_tid,
                        selection_budget.saturating_sub(seek_storage(&seek_preroll)),
                    )?;
                    first_sample = index;
                    scan = SubstreamScan::default();
                    seen_vcl = false;
                }
                boundary = false;
            }
            let mut candidate: Option<Arc<LayeredVps>> = None;
            for nal in NalUnits::new(&packet, length_size)? {
                let nal = nal?;
                let h = NalHeader::parse(nal)?;
                if h.layer_id == 63 {
                    continue;
                }
                if matches!(h.unit_type, 32..=34) {
                    inventory_parameters.receive(nal)?;
                }
                if h.layer_id == 0 && h.is_vcl() {
                    base_restart |= matches!(h.unit_type, 16..=18)
                        || inventory_parameters.base_cross_layer_bla(nal)?;
                }
                if h.is_vcl() && candidate.is_none() {
                    let referenced = inventory_parameters.referenced_vps(nal)?;
                    let snapshot_bytes = |v: &LayeredVps| {
                        v.retained_bytes()
                            .saturating_add(2 * std::mem::size_of::<usize>())
                    };
                    let available = selection_budget
                        .checked_sub(
                            selection_storage(&epochs)
                                .saturating_add(seek_storage(&seek_preroll))
                                .saturating_add(vps.as_ref().map_or(0, |v| snapshot_bytes(v)))
                                .saturating_add(
                                    candidate.as_ref().map_or(0, |v| snapshot_bytes(v)),
                                ),
                        )
                        .ok_or_else(|| invalid("HEVC source selections exceed memory budget"))?;
                    candidate = Some(Arc::new(LayeredVps::parse(referenced, available)?));
                }
            }
            if let Some(received) = candidate {
                if vps.as_ref().is_none_or(|v| **v != *received) {
                    if seen_vcl {
                        retain_selection(
                            &mut epochs,
                            first_sample,
                            scan,
                            vps.take()
                                .ok_or_else(|| invalid("HEVC coded epoch has no VPS"))?,
                            output_layer_set,
                            automatic,
                            highest_tid,
                            selection_budget.saturating_sub(seek_storage(&seek_preroll)),
                        )?;
                        first_sample = index;
                        scan = SubstreamScan::default();
                        seen_vcl = false;
                    }
                    vps = Some(received);
                }
            }
            scan.observe(&packet, length_size)?;
            let mut present = 0u64;
            let mut irap = 0u64;
            for nal in NalUnits::new(&packet, length_size)? {
                let h = NalHeader::parse(nal?)?;
                if h.layer_id == 63 {
                    continue;
                }
                seen_vcl |= h.is_vcl();
                if h.is_vcl() {
                    present |= 1u64 << h.layer_id;
                }
                if h.is_irap() {
                    irap |= 1u64 << h.layer_id;
                }
                boundary |= h.unit_type == 37 || h.unit_type == 36 && h.layer_id == 0;
            }
            let current_ols = if automatic {
                vps.as_ref()
                    .map(|v| v.automatic_output_layer_set())
                    .transpose()?
                    .unwrap_or(output_layer_set)
            } else {
                output_layer_set
            };
            let required = vps
                .as_ref()
                .and_then(|v| v.extension.as_ref())
                .and_then(|e| e.output.sets.get(current_ols))
                .map_or(0, |set| set.necessary);
            let safe = present != 0
                && (restart_pending || base_restart || required != 0 && required & !irap == 0);
            if demuxer.tracks()[track_index]
                .samples
                .get(index)
                .unwrap()
                .sync
            {
                if safe {
                    safe_sync = index;
                } else {
                    let growth = (seek_preroll.len() + 1)
                        .saturating_mul(std::mem::size_of::<(usize, usize)>());
                    if selection_storage(&epochs)
                        .saturating_add(seek_storage(&seek_preroll))
                        .saturating_add(growth)
                        .saturating_add(vps.as_ref().map_or(0, |v| v.retained_bytes()))
                        > selection_budget
                    {
                        return Err(invalid("HEVC seek inventory exceeds memory budget"));
                    }
                    seek_preroll
                        .try_reserve_exact(1)
                        .map_err(|_| invalid("cannot allocate HEVC seek inventory"))?;
                    seek_preroll.push((index, safe_sync));
                }
            }
            if present != 0 {
                restart_pending = false;
            }
            if boundary {
                restart_pending = true;
            }
        }
        retain_selection(
            &mut epochs,
            first_sample,
            scan,
            vps.ok_or_else(|| invalid("layered HEVC source has no active VPS"))?,
            output_layer_set,
            automatic,
            highest_tid,
            selection_budget.saturating_sub(seek_storage(&seek_preroll)),
        )?;
        drop(inventory_parameters);
        let work_budget = (budget / 2)
            .checked_sub(
                selection_storage(&epochs)
                    .saturating_add(seek_storage(&seek_preroll))
                    .saturating_add(std::mem::size_of::<Self>()),
            )
            .ok_or_else(|| invalid("HEVC source selections exceed memory budget"))?;
        let mut initial_configuration = demuxer.tracks()[track_index]
            .description_for_sample(0)
            .unwrap()
            .configuration
            .clone();
        initial_configuration[21] |= 3;
        let decoder = LayeredDecoder::from_configuration(
            &initial_configuration,
            output_layer_set,
            work_budget,
        )?;
        demuxer.invalidate_position();
        packet.clear();
        Ok(Self {
            demuxer,
            track_index,
            decoder,
            automatic_selection: automatic,
            queue: LayeredOutputQueue::default(),
            selections: epochs,
            seek_preroll,
            visible_sample: 0,
            ready: VecDeque::new(),
            ready_budget: budget / 4,
            packet,
            raw_packet,
            packet_description: initial_description,
            initial_description,
            length_size,
            sample_index: 0,
            first_sample: 0,
            prime_until: None,
            eof: false,
            failed: false,
        })
    }
    pub fn track(&self) -> &Track {
        &self.demuxer.tracks()[self.track_index]
    }
    /// Completed extraction policy at a source sample, including metadata-only
    /// samples before its sequence's first coded picture or replacement VPS.
    pub fn selection_at(&self, sample_index: usize) -> Option<&LayeredSubstream> {
        if sample_index >= self.track().samples.len() {
            return None;
        }
        Some(&self.selections[self.selection_index(sample_index)].selection)
    }
    fn selection_index(&self, sample_index: usize) -> usize {
        self.selections
            .partition_point(|e| e.first_sample <= sample_index)
            .saturating_sub(1)
    }
    pub(super) fn visible_sample(&self) -> usize {
        self.visible_sample
    }
    pub fn movie_timescale(&self) -> u32 {
        self.demuxer.movie_timescale()
    }
    pub fn rewind(&mut self) {
        self.decoder.reset_queued(&mut self.queue);
        self.ready.clear();
        self.packet.clear();
        self.raw_packet.clear();
        self.packet_description = self.initial_description;
        self.demuxer.invalidate_position();
        self.sample_index = 0;
        self.first_sample = 0;
        self.visible_sample = 0;
        self.prime_until = None;
        self.eof = false;
        self.failed = false;
    }
    /// Reset at an indexed sync sample and recover preceding parameter and HDR
    /// header history without reconstructing skipped pixels. Reading performs
    /// the deferred I/O. Mixed sync AUs preroll from the last complete layer
    /// entry; their preparatory frames are hidden while references initialize.
    pub fn seek_to_sync(&mut self, pts: i64) -> i64 {
        let index = self
            .track()
            .samples
            .expanded()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync && s.pts <= pts)
            .max_by_key(|(i, s)| (s.pts, std::cmp::Reverse(*i)))
            .map_or(0, |(i, _)| i);
        let start = self
            .seek_preroll
            .binary_search_by_key(&index, |entry| entry.0)
            .ok()
            .map_or(index, |at| self.seek_preroll[at].1);
        self.rewind();
        self.sample_index = start;
        self.first_sample = start;
        self.visible_sample = index;
        self.prime_until = (start > 0).then_some(start);
        self.track().samples.get(index).map_or(0, |s| s.pts)
    }
    fn retain_ready(&mut self, pictures: Vec<LayeredDecoded>) -> Result<()> {
        let mut bytes = self
            .ready
            .capacity()
            .saturating_add(pictures.capacity())
            .saturating_add(self.ready.len())
            .saturating_add(pictures.len())
            .saturating_mul(2 * std::mem::size_of::<LayeredDecoded>());
        for p in self.ready.iter().chain(&pictures) {
            bytes = bytes
                .checked_add(p.retained_bytes()?)
                .ok_or_else(|| invalid("HEVC reader output storage overflow"))?;
        }
        if bytes > self.ready_budget {
            return Err(invalid("HEVC reader outputs exceed memory budget"));
        }
        self.ready
            .try_reserve(pictures.len())
            .map_err(|_| invalid("cannot allocate HEVC reader outputs"))?;
        self.ready.extend(pictures);
        Ok(())
    }
    pub fn read_frame(&mut self) -> Result<Option<LayeredVideoFrame>> {
        if self.failed {
            return Err(invalid("layered MP4 source requires rewind after an error"));
        }
        let result = self.read_frame_inner();
        self.failed = result.is_err();
        result
    }
    fn read_frame_inner(&mut self) -> Result<Option<LayeredVideoFrame>> {
        if let Some(end) = self.prime_until.take() {
            for index in 0..end {
                self.packet_description = super::layered_packet::read_normalized(
                    &mut self.demuxer,
                    self.track_index,
                    index,
                    self.packet_description,
                    &mut self.raw_packet,
                    &mut self.packet,
                    self.ready_budget / 2,
                )?;
                let selection = self.selection_index(index);
                let extracted = self.selections[selection].selection.extract_packet(
                    &self.packet,
                    self.length_size,
                    self.ready_budget / 2,
                )?;
                if extracted.capacity().saturating_add(
                    self.ready
                        .capacity()
                        .saturating_mul(std::mem::size_of::<LayeredDecoded>()),
                ) > self.ready_budget
                {
                    return Err(invalid("HEVC seek priming exceeds memory budget"));
                }
                if self.automatic_selection {
                    self.decoder.prime_reselected_metadata(
                        &extracted,
                        self.selections[selection].selection.output_layer_set(),
                    )?;
                } else {
                    self.decoder.prime_metadata(&extracted)?;
                }
            }
            self.decoder.finish_metadata_priming();
        }
        loop {
            if let Some(decoded) = self.ready.pop_front() {
                let sample_index = self
                    .first_sample
                    .checked_add(
                        usize::try_from(decoded.access_unit)
                            .map_err(|_| invalid("HEVC output sample index overflow"))?,
                    )
                    .ok_or_else(|| invalid("HEVC output sample index overflow"))?;
                if sample_index < self.visible_sample {
                    continue;
                }
                let sample = self
                    .track()
                    .samples
                    .get(sample_index)
                    .ok_or_else(|| invalid("HEVC output sample is absent"))?;
                let timescale = self.track().timescale;
                return Ok(Some(LayeredVideoFrame {
                    decoded,
                    sample_index,
                    presentation_time: MediaTime {
                        ticks: sample.pts,
                        timescale,
                    },
                    duration: MediaTime {
                        ticks: i64::from(sample.duration),
                        timescale,
                    },
                }));
            }
            if self.eof {
                return Ok(None);
            }
            if self.sample_index == self.track().samples.len() {
                let tail = self.queue.drain();
                self.retain_ready(tail)?;
                self.eof = true;
                continue;
            }
            self.packet_description = super::layered_packet::read_normalized(
                &mut self.demuxer,
                self.track_index,
                self.sample_index,
                self.packet_description,
                &mut self.raw_packet,
                &mut self.packet,
                self.ready_budget / 2,
            )?;
            let selection = self.selection_index(self.sample_index);
            let selection = &self.selections[selection].selection;
            let (pictures, _) = if self.automatic_selection {
                self.decoder.decode_reselected_queued_packet(
                    &mut self.queue,
                    selection,
                    &self.packet,
                    selection.highest_temporal_id(),
                )?
            } else {
                self.decoder.decode_selected_queued_packet(
                    &mut self.queue,
                    selection,
                    &self.packet,
                    selection.highest_temporal_id(),
                )?
            };
            self.sample_index += 1;
            self.retain_ready(pictures)?;
        }
    }
}
