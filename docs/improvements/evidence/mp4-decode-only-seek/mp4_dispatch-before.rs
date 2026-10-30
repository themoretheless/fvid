//! Common MP4 entry: single-layer codecs retain their existing source; layered
//! HEVC uses owned operation-point extraction and one displayed layer per AU.
use super::*;
use crate::codec::{config::HevcConfig, hevc_vps::LayeredVps};

/// Indexed MP4 video source with automatic codec and layered HEVC dispatch.
pub struct Mp4VideoReader<R> {
    source: Source<R>,
    presentation_end: std::sync::OnceLock<i128>,
    presentation_bound: Option<(u64, u32)>,
}
enum Source<R> {
    Single(Box<SingleLayerMp4VideoReader<R>>),
    Layered(Box<SingleView<R>>),
}
struct SingleView<R> {
    source: LayeredMp4VideoReader<R>,
    lookahead: Option<LayeredVideoFrame>,
    budget: usize,
    colour: crate::playback_native::AvcColour,
    initial_colour: crate::playback_native::AvcColour,
    signal: ColourDescription,
    initial_signal: ColourDescription,
    hdr: HdrMetadata,
    selected: Option<u8>,
    presented_sample: Option<usize>,
    failed: bool,
}
pub(super) fn signal(vui: Option<&crate::codec::hevc_vui::Vui>) -> ColourDescription {
    vui.and_then(|v| v.signal.as_ref())
        .map(|s| {
            let [primaries, transfer, matrix] = s.colour.unwrap_or([0; 3]);
            ColourDescription {
                primaries,
                transfer,
                matrix,
                full_range: s.full_range,
            }
        })
        .unwrap_or_default()
}
// hvc1 keeps its fixed-configuration fast path. A hev1 base-only prefix may
// activate a multilayer VPS later: choose one owner for the complete source,
// inspecting actual PPS/SPS/VPS references rather than unused parameter NALs.
fn dispatch_hevc_vps<R: Read + Seek>(
    demuxer: &mut Mp4Reader<R>,
    index: usize,
    budget: usize,
) -> Result<(Option<Vec<u8>>, bool)> {
    let track = &demuxer.tracks()[index];
    let setup = track
        .description_for_sample(0)
        .ok_or_else(|| invalid("empty HEVC track"))?;
    let configuration = HevcConfig::parse(&setup.configuration)?;
    let mut vps = configuration
        .arrays
        .iter()
        .filter(|a| a.nal_type == 32)
        .flat_map(|a| a.units.iter().copied());
    let first = vps.next();
    let multiple = vps.next().is_some();
    let copy = |nal: &[u8]| -> Result<Vec<u8>> {
        if nal.len() > budget / 8 {
            return Err(invalid("HEVC dispatch VPS exceeds memory budget"));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(nal.len())
            .map_err(|_| invalid("cannot allocate HEVC dispatch VPS"))?;
        out.extend_from_slice(nal);
        Ok(out)
    };
    if track.ensure_constant_description().is_ok()
        && setup.codec == *b"hvc1"
        && !multiple
        && let Some(first) = first
    {
        return copy(first).map(|v| (Some(v), false));
    }
    let mut fallback = first.map(copy).transpose()?;
    let mut seen_vcl = false;
    let mut vpsless = false;
    let length = 4;
    let mut description = track.samples.get(0).unwrap().description_index;
    let count = track.samples.len();
    let mut parameters = crate::codec::hevc_decoder::LayeredParameters::from_configuration(
        &setup.configuration,
        budget / 2,
    )?;
    let mut packet = Vec::new();
    let mut normalized = Vec::new();
    for sample in 0..count {
        description = super::layered_packet::read_normalized(
            demuxer,
            index,
            sample,
            description,
            &mut packet,
            &mut normalized,
            budget / 8,
        )?;
        for nal in crate::codec::config::NalUnits::new(&normalized, length)? {
            let nal = nal?;
            let header = crate::codec::hevc_nal::NalHeader::parse(nal)?;
            if header.layer_id == 63 {
                continue;
            }
            if matches!(header.unit_type, 32..=34) {
                parameters.receive(nal)?;
            }
            if header.is_vcl() {
                let Some(referenced) = parameters.referenced_vps_if_available(nal)? else {
                    header.require_base_layer()?;
                    vpsless = true;
                    seen_vcl = true;
                    continue;
                };
                let topology =
                    crate::codec::hevc_vps::Vps::parse_layered_metadata(referenced, budget / 2)?;
                if !seen_vcl {
                    fallback = Some(copy(referenced)?);
                    seen_vcl = true;
                }
                if topology.layers.effective_max_layers() > 1 {
                    let result = copy(referenced);
                    demuxer.invalidate_position();
                    return result.map(|v| (Some(v), vpsless));
                }
            }
        }
    }
    demuxer.invalidate_position();
    Ok((fallback, vpsless))
}

impl<R: Read + Seek> SingleView<R> {
    fn reset(&mut self) {
        self.source.rewind();
        self.lookahead = None;
        self.selected = None;
        self.presented_sample = None;
        self.failed = false;
        self.colour = self.initial_colour;
        self.signal = self.initial_signal;
        self.hdr = HdrMetadata::default();
    }
    fn group(&mut self) -> Result<Option<LayeredVideoFrame>> {
        let first = if self.lookahead.is_some() {
            self.lookahead.take()
        } else {
            self.source.read_frame()?
        };
        let Some(mut chosen) = first else {
            return Ok(None);
        };
        while let Some(next) = self.source.read_frame()? {
            if next.decoded.access_unit != chosen.decoded.access_unit {
                self.lookahead = Some(next);
                break;
            }
            if next.decoded.layer_id > chosen.decoded.layer_id {
                chosen = next;
            }
        }
        let held = chosen.decoded.retained_bytes()?.saturating_add(
            self.lookahead
                .as_ref()
                .map(|f| f.decoded.retained_bytes())
                .transpose()?
                .unwrap_or(0),
        );
        if held > self.budget {
            return Err(invalid(
                "HEVC displayed layer selection exceeds memory budget",
            ));
        }
        Ok(Some(chosen))
    }
    fn read(&mut self) -> Result<Option<VideoFrame>> {
        if self.failed {
            return Err(invalid("MP4 frame source requires rewind after an error"));
        }
        let result = self.read_inner();
        self.failed = result.is_err();
        result
    }
    fn read_inner(&mut self) -> Result<Option<VideoFrame>> {
        let Some(mut frame) = self.group()? else {
            return Ok(None);
        };
        while self
            .lookahead
            .as_ref()
            .is_some_and(|f| f.presentation_time == frame.presentation_time)
        {
            let old = frame.duration.ticks;
            drop(frame);
            frame = self
                .group()?
                .ok_or_else(|| invalid("missing duplicate HEVC presentation group"))?;
            frame.duration.ticks = frame
                .duration
                .ticks
                .checked_add(old)
                .ok_or_else(|| invalid("duplicate-PTS duration overflow"))?;
        }
        let Some(duration) = presentation_duration(
            frame.presentation_time.ticks,
            self.lookahead.as_ref().map(|f| f.presentation_time.ticks),
            frame.duration.ticks,
        )? else {
            return Ok(None);
        };
        frame.duration.ticks = duration;
        let conversion = frame
            .decoded
            .retained_bytes()?
            .saturating_mul(2)
            .saturating_add(
                self.lookahead
                    .as_ref()
                    .map(|f| f.decoded.retained_bytes())
                    .transpose()?
                    .unwrap_or(0),
            );
        if conversion > self.budget {
            return Err(invalid(
                "HEVC displayed layer conversion exceeds memory budget",
            ));
        }
        let vui = frame.decoded.binding.sps.sps.vui.as_ref();
        self.colour = crate::playback_native::AvcColour::from_hevc_vui(vui)?;
        self.signal = signal(vui);
        self.selected = Some(frame.decoded.layer_id);
        self.presented_sample = Some(frame.sample_index);
        self.hdr = frame.decoded.hdr;
        Ok(Some(frame.into_video_frame()?))
    }
}
impl<R: Read + Seek> Mp4VideoReader<R> {
    /// Prefer an enabled movie track; retain file order within each preference.
    pub fn open(reader: R, limits: Limits, budget: usize) -> Result<Self> {
        Self::open_inner(reader, limits, budget, true)
    }
    /// Decode with owned FVid codecs regardless of compiled platform features.
    pub fn open_software(reader: R, limits: Limits, budget: usize) -> Result<Self> {
        Self::open_inner(reader, limits, budget, false)
    }
    fn open_inner(reader: R, limits: Limits, budget: usize, hardware: bool) -> Result<Self> {
        let demuxer = Mp4Reader::open_indexed(reader, limits)?;
        let index = demuxer
            .automatic_track_indices(&[*b"vide"])
            .into_iter()
            .find(|&index| {
                demuxer.tracks()[index]
                    .description_for_sample(0)
                    .is_some_and(|d| supported_video_codec(&d.codec))
            })
            .ok_or_else(|| invalid("MP4 has no supported AVC, HEVC, VP9 or AV1 video track"))?;
        Self::from_inner(demuxer, index, budget, hardware)
    }
    pub fn from_demuxer(demuxer: Mp4Reader<R>, index: usize, budget: usize) -> Result<Self> {
        Self::from_inner(demuxer, index, budget, true)
    }
    fn from_inner(
        mut demuxer: Mp4Reader<R>,
        index: usize,
        budget: usize,
        hardware: bool,
    ) -> Result<Self> {
        let presentation_bound = demuxer.track_header_duration(index)
            .or_else(|| demuxer.movie_duration())
            .map(|duration| (duration, demuxer.movie_timescale()));
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;

        if super::layered_packet::all_hevc(track) {
            let (nal, vpsless) = dispatch_hevc_vps(&mut demuxer, index, budget)?;
            if let Some(nal) = nal {
                // Inspect topology without relaxing the base decoder's guard.
                let topology =
                    crate::codec::hevc_vps::Vps::parse_layered_metadata(&nal, budget / 2)?;
                if topology.layers.effective_max_layers() > 1 {
                    // Native layer output follows codec POC. Container PTS can
                    // reorder independent access units across sequence resets;
                    // the common reader retains the bounded PTS queue and seek
                    // coverage required for that presentation order.
                    let reordered =
                        demuxer.tracks()[index]
                            .samples
                            .expanded()
                            .is_some_and(|samples| {
                                samples.windows(2).any(|pair| pair[0].pts > pair[1].pts)
                            });
                    if vpsless || reordered {
                        return Ok(Self {
                            presentation_end: Default::default(),
                            presentation_bound,
                            source: Source::Single(Box::new(
                                SingleLayerMp4VideoReader::from_demuxer_with_hardware(
                                    demuxer, index, budget, false, true,
                                )?,
                            )),
                        });
                    }
                    let vps = LayeredVps::parse(&nal, budget / 2)?;
                    let ols = vps.automatic_output_layer_set()?;
                    let mut colour = crate::playback_native::AvcColour::default();
                    let mut stated = ColourDescription::default();
                    let config = HevcConfig::parse(
                        &demuxer.tracks()[index]
                            .description_for_sample(0)
                            .unwrap()
                            .configuration,
                    )?;
                    for array in &config.arrays {
                        for nal in &array.units {
                            if array.nal_type == 33
                                && crate::codec::hevc_nal::NalHeader::parse(nal)?.layer_id == 0
                            {
                                let s = crate::codec::hevc_sps::Sps::parse(nal, budget / 2)?;
                                colour = crate::playback_native::AvcColour::from_hevc_vui(
                                    s.vui.as_ref(),
                                )?;
                                stated = signal(s.vui.as_ref());
                            }
                        }
                    }
                    let source = LayeredMp4VideoReader::from_demuxer_automatic(
                        demuxer,
                        index,
                        ols,
                        budget / 2,
                    )?;
                    return Ok(Self {
                        presentation_end: Default::default(),
                            presentation_bound,
                        source: Source::Layered(Box::new(SingleView {
                            source,
                            lookahead: None,
                            budget: budget - budget / 2,
                            colour,
                            initial_colour: colour,
                            signal: stated,
                            initial_signal: stated,
                            hdr: HdrMetadata::default(),
                            selected: None,
                            presented_sample: None,
                            failed: false,
                        })),
                    });
                }
            }
        }
        Ok(Self {
            presentation_end: Default::default(),
                            presentation_bound,
            source: Source::Single(Box::new(
                SingleLayerMp4VideoReader::from_demuxer_with_hardware(
                    demuxer, index, budget, hardware, false,
                )?,
            )),
        })
    }
    /// Last displayed layered HEVC output layer; absent for ordinary sources.
    pub fn selected_layer_id(&self) -> Option<u8> {
        match &self.source {
            Source::Single(s) => s.presented_layer,
            Source::Layered(s) => s.selected,
        }
    }
    pub fn track(&self) -> &Track {
        match &self.source {
            Source::Single(s) => s.track(),
            Source::Layered(s) => s.source.track(),
        }
    }
    /// Container setup belonging to the last displayed frame, or the initial
    /// setup before presentation. Decode lookahead can use a later description.
    pub fn container_description(&self) -> &crate::container::mp4::SampleDescription {
        match &self.source {
            Source::Single(s) => s.container_description(),
            Source::Layered(s) => s
                .source
                .track()
                .description_for_sample(s.presented_sample.unwrap_or(s.source.visible_sample()))
                .unwrap(),
        }
    }
    // The media header describes the decode duration, which can end before
    // composition output. Cache the final PTS group's full nominal span so
    // duration queries do not rescan the sample index on every UI refresh.
    pub(crate) fn presentation_end_ticks(&self) -> i128 {
        *self.presentation_end.get_or_init(|| {
            let track = self.track();
            let mut latest = i64::MIN;
            let mut span = 0i128;
            for index in 0..track.samples.len() {
                if let Some(sample) = track.samples.get(index) {
                    match sample.pts.cmp(&latest) {
                        std::cmp::Ordering::Greater => {
                            latest = sample.pts;
                            span = i128::from(sample.duration);
                        }
                        std::cmp::Ordering::Equal => span += i128::from(sample.duration),
                        std::cmp::Ordering::Less => {}
                    }
                }
            }
            let mut end = i128::from(track.duration).max(i128::from(latest) + span);
            if let Some((duration, movie_scale)) = self.presentation_bound {
                let header = i128::from(duration) * i128::from(track.timescale);
                let scale = i128::from(movie_scale);
                // A hidden picture or codec-control sample can carry a late
                // PTS without displaying anything there. Honor a finite track
                // endpoint, allowing only sub-movie-tick header quantization.
                if end.checked_mul(scale).is_none_or(|ticks| ticks - header >= i128::from(track.timescale)) {
                    end = i128::from(track.duration).max((header + scale - 1) / scale);
                }
            }
            end
        })
    }

    pub fn movie_timescale(&self) -> u32 {
        match &self.source {
            Source::Single(s) => s.movie_timescale(),
            Source::Layered(s) => s.source.movie_timescale(),
        }
    }
    pub fn rewind(&mut self) {
        match &mut self.source {
            Source::Single(s) => s.rewind(),
            Source::Layered(s) => s.reset(),
        }
    }
    pub fn seek_to_sync(&mut self, pts: i64) -> i64 {
        let samples = &self.track().samples;
        let empty_tail = samples.len().checked_sub(1).and_then(|i| samples.get(i))
            .is_some_and(|s| s.duration == 0);
        let pts = if empty_tail {
            // Do not restart only at a terminal empty window: callers seeking
            // to/past EOF still need the final displayed frame. Earlier zero
            // durations can acquire a window from a later composition PTS, so
            // retain them in decoding rather than filtering packet contents.
            let mut latest_nonempty = None;
            let mut earliest = None;
            for i in 0..samples.len() {
                if let Some(s) = samples.get(i) {
                    earliest = Some(earliest.map_or(s.pts, |old: i64| old.min(s.pts)));
                    if s.duration != 0 {
                        latest_nonempty = Some(latest_nonempty.map_or(s.pts, |old: i64| old.max(s.pts)));
                    }
                }
            }
            pts.min(latest_nonempty.or(earliest).unwrap_or(pts))
        } else { pts };
        match &mut self.source {
            Source::Single(s) => s.seek_to_sync(pts),
            Source::Layered(s) => {
                s.reset();
                s.source.seek_to_sync(pts)
            }
        }
    }
    pub fn read_frame(&mut self) -> Result<Option<VideoFrame>> {
        match &mut self.source {
            Source::Single(s) => s.read_frame(),
            Source::Layered(s) => s.read(),
        }
    }
    pub fn active_vui(&self) -> Option<&crate::codec::avc::Vui> {
        match &self.source {
            Source::Single(s) => s.active_vui(),
            Source::Layered(_) => None,
        }
    }
    pub fn active_colour(&self) -> Result<crate::playback_native::AvcColour> {
        match &self.source {
            Source::Single(s) => s.active_colour(),
            Source::Layered(s) => Ok(s.colour),
        }
    }
    pub fn bitstream_colour(&self) -> ColourDescription {
        match &self.source {
            Source::Single(s) => s.bitstream_colour(),
            Source::Layered(s) => s.signal,
        }
    }
    pub fn bitstream_hdr(&self) -> HdrMetadata {
        match &self.source {
            Source::Single(s) => s.bitstream_hdr(),
            Source::Layered(s) => s.hdr,
        }
    }
    pub fn hardware_accelerated(&self) -> bool {
        match &self.source {
            Source::Single(s) => s.hardware_accelerated(),
            Source::Layered(_) => false,
        }
    }
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    pub fn enable_shared_surfaces(&mut self) -> Result<bool> {
        match &mut self.source {
            Source::Single(s) => s.enable_shared_surfaces(),
            Source::Layered(_) => Ok(false),
        }
    }
}
