//! Indexed, software-only layered HEVC source. A finite source scan pins the
//! extraction policy; all selected output layers retain their coded identity.
use super::{MediaTime, VideoFrame};
use crate::codec::{
    config::HevcConfig,
    hevc_decoder::{
        LayeredDecoded, LayeredDecoder, LayeredOutputQueue, LayeredSubstream, SubstreamScan,
    },
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
        let colour = crate::playback_native::AvcColour::from_hevc_vui(s.vui.as_ref())?;
        let packed = Arc::new(crate::playback_native::hevc_picture(
            p,
            s.chroma_format,
            colour,
        )?);
        let picture = Arc::new(crate::codec::avc_picture::IntraPicture {
            coded_width: p.dimensions[0] as usize,
            coded_height: p.dimensions[1] as usize,
            crop: p.crop.map(|n| n as usize),
            bit_depth: p.depth[0],
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
/// Each returned layer keeps the PTS/duration of its originating MP4 sample,
/// even when decoding a later packet releases it. Equal-PTS layers are distinct.
/// Output follows native POC ordering; container edit lists remain on track().
/// The operation point uses the configured VPS for this finite source. A VPS
/// replacement is refused before native state publication, not silently reused.
/// Demux metadata has its own Limits; caller-owned returned frames are separate.
pub struct LayeredMp4VideoReader<R> {
    demuxer: Mp4Reader<R>,
    track_index: usize,
    decoder: LayeredDecoder,
    queue: LayeredOutputQueue,
    selection: LayeredSubstream,
    ready: VecDeque<LayeredDecoded>,
    ready_budget: usize,
    packet: Vec<u8>,
    length_size: u8,
    highest_tid: u8,
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
        let demuxer = Mp4Reader::open(reader, limits)?;
        let track_index = demuxer
            .tracks()
            .iter()
            .position(|t| t.handler == *b"vide" && matches!(&t.codec, b"hvc1" | b"hev1"))
            .ok_or_else(|| invalid("MP4 has no HEVC video track"))?;
        Self::from_demuxer(demuxer, track_index, output_layer_set, highest_tid, budget)
    }
    pub fn from_demuxer(
        mut demuxer: Mp4Reader<R>,
        track_index: usize,
        output_layer_set: usize,
        highest_tid: u8,
        budget: usize,
    ) -> Result<Self> {
        let track = demuxer
            .tracks()
            .get(track_index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        if track.handler != *b"vide" || !matches!(&track.codec, b"hvc1" | b"hev1") {
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
        let config = HevcConfig::parse(&track.configuration)?;
        let length_size = config.length_size;
        let vps = config
            .arrays
            .iter()
            .filter(|a| a.nal_type == 32)
            .flat_map(|a| a.units.iter())
            .next()
            .ok_or_else(|| invalid("layered HEVC configuration has no VPS"))?;
        let vps = Arc::new(LayeredVps::parse(vps, budget / 2)?);
        let initial_selection =
            SubstreamScan::default().select(Arc::clone(&vps), output_layer_set, highest_tid)?;
        let work_budget = (budget / 2)
            .checked_sub(
                initial_selection
                    .retained_bytes()
                    .saturating_add(std::mem::size_of::<Self>()),
            )
            .ok_or_else(|| invalid("HEVC source selection exceeds memory budget"))?;
        let decoder = LayeredDecoder::from_configuration(
            &track.configuration,
            output_layer_set,
            work_budget,
        )?;
        let count = track.samples.len();
        let mut scan = SubstreamScan::default();
        let mut packet = Vec::new();
        // Global timing-SEI reduction must include metadata-only earlier samples.
        for index in 0..count {
            demuxer.read_packet(track_index, index, &mut packet)?;
            scan.observe(&packet, length_size)?;
        }
        let selection = scan.select(vps, output_layer_set, highest_tid)?;
        demuxer.invalidate_position();
        packet.clear();
        Ok(Self {
            demuxer,
            track_index,
            decoder,
            queue: LayeredOutputQueue::default(),
            selection,
            ready: VecDeque::new(),
            ready_budget: budget / 4,
            packet,
            length_size,
            highest_tid,
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
    pub fn movie_timescale(&self) -> u32 {
        self.demuxer.movie_timescale()
    }
    pub fn rewind(&mut self) {
        self.decoder.reset_queued(&mut self.queue);
        self.ready.clear();
        self.packet.clear();
        self.demuxer.invalidate_position();
        self.sample_index = 0;
        self.first_sample = 0;
        self.prime_until = None;
        self.eof = false;
        self.failed = false;
    }
    /// Reset at an indexed sync sample and recover preceding parameter and HDR
    /// header history without reconstructing skipped pixels. Reading performs
    /// the deferred I/O; skipped POC history is discarded before target decoding.
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
        self.rewind();
        self.sample_index = index;
        self.first_sample = index;
        self.prime_until = (index > 0).then_some(index);
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
                self.demuxer
                    .read_packet(self.track_index, index, &mut self.packet)?;
                let extracted = self.selection.extract_packet(
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
                self.decoder.prime_metadata(&extracted)?;
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
            self.demuxer
                .read_packet(self.track_index, self.sample_index, &mut self.packet)?;
            let (pictures, _) = self.decoder.decode_selected_queued_packet(
                &mut self.queue,
                &self.selection,
                &self.packet,
                self.highest_tid,
            )?;
            self.sample_index += 1;
            self.retain_ready(pictures)?;
        }
    }
}
