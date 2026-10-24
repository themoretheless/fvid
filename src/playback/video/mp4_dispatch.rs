//! Common MP4 entry: single-layer codecs retain their existing source; layered
//! HEVC uses owned operation-point extraction and one displayed layer per AU.
use super::*;
use crate::codec::{config::HevcConfig, hevc_vps::LayeredVps};

/// Indexed MP4 video source with automatic codec and layered HEVC dispatch.
pub struct Mp4VideoReader<R> {
    source: Source<R>,
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
    failed: bool,
}
fn signal(vui: Option<&crate::codec::hevc_vui::Vui>) -> ColourDescription {
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
// hev1 may introduce or replace its active parameter chain before the first
// coded picture. Inspect only that prefix; ordinary complete hvc1 is unchanged
// and does not incur an additional packet scan. Ignore unactivated VPS order.
fn first_active_hevc_vps<R: Read + Seek>(
    demuxer: &mut Mp4Reader<R>,
    index: usize,
    budget: usize,
) -> Result<Option<Vec<u8>>> {
    let track = &demuxer.tracks()[index];
    let configuration = HevcConfig::parse(&track.configuration)?;
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
    if track.codec == *b"hvc1"
        && !multiple
        && let Some(first) = first
    {
        return copy(first).map(Some);
    }
    let fallback = first.map(copy).transpose()?;
    let length = configuration.length_size;
    let count = track.samples.len();
    let mut parameters = crate::codec::hevc_decoder::LayeredParameters::from_configuration(
        &track.configuration,
        budget / 2,
    )?;
    let mut packet = Vec::new();
    for sample in 0..count {
        let size = demuxer.tracks()[index]
            .samples
            .get(sample)
            .ok_or_else(|| invalid("HEVC dispatch sample is absent"))?
            .size as usize;
        if size > budget / 8 {
            return Err(invalid("HEVC dispatch packet exceeds memory budget"));
        }
        if packet.capacity() < size {
            packet = Vec::new();
            packet
                .try_reserve_exact(size)
                .map_err(|_| invalid("cannot allocate HEVC dispatch packet"))?;
        }
        if packet.capacity() > budget / 8 {
            return Err(invalid(
                "HEVC dispatch packet capacity exceeds memory budget",
            ));
        }
        demuxer.read_packet(index, sample, &mut packet)?;
        for nal in crate::codec::config::NalUnits::new(&packet, length)? {
            let nal = nal?;
            let header = crate::codec::hevc_nal::NalHeader::parse(nal)?;
            if header.layer_id == 63 {
                continue;
            }
            if matches!(header.unit_type, 32..=34) {
                parameters.receive(nal)?;
            }
            if header.is_vcl() {
                let result = copy(parameters.referenced_vps(nal)?);
                demuxer.invalidate_position();
                return result.map(Some);
            }
        }
    }
    demuxer.invalidate_position();
    Ok(fallback)
}

impl<R: Read + Seek> SingleView<R> {
    fn reset(&mut self) {
        self.source.rewind();
        self.lookahead = None;
        self.selected = None;
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
        frame.duration.ticks = presentation_duration(
            frame.presentation_time.ticks,
            self.lookahead.as_ref().map(|f| f.presentation_time.ticks),
            frame.duration.ticks,
        )?;
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
        let demuxer = Mp4Reader::open(reader, limits)?;
        let index = demuxer
            .automatic_track_indices(&[*b"vide"])
            .into_iter()
            .find(|&index| {
                matches!(
                    &demuxer.tracks()[index].codec,
                    b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"vp09" | b"av01"
                )
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
        let track = demuxer
            .tracks()
            .get(index)
            .ok_or_else(|| invalid("MP4 track index out of range"))?;
        track.ensure_constant_description()?;
        if matches!(&track.codec, b"hvc1" | b"hev1") {
            let nal = first_active_hevc_vps(&mut demuxer, index, budget)?;
            if let Some(nal) = nal {
                // Inspect topology without relaxing the base decoder's guard.
                let topology =
                    crate::codec::hevc_vps::Vps::parse_layered_metadata(&nal, budget / 2)?;
                if topology.layers.effective_max_layers() > 1 {
                    let vps = LayeredVps::parse(&nal, budget / 2)?;
                    let e = vps
                        .extension
                        .as_ref()
                        .ok_or_else(|| invalid("layered HEVC VPS extension is absent"))?;
                    let ols = e
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
                        .ok_or_else(|| invalid("HEVC VPS has no output layer set"))?;
                    let tid = (vps.vps.ordering.len() - 1) as u8;
                    let mut colour = crate::playback_native::AvcColour::default();
                    let mut stated = ColourDescription::default();
                    let config = HevcConfig::parse(&demuxer.tracks()[index].configuration)?;
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
                    let source =
                        LayeredMp4VideoReader::from_demuxer(demuxer, index, ols, tid, budget / 2)?;
                    return Ok(Self {
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
                            failed: false,
                        })),
                    });
                }
            }
        }
        Ok(Self {
            source: Source::Single(Box::new(
                SingleLayerMp4VideoReader::from_demuxer_with_hardware(
                    demuxer, index, budget, hardware,
                )?,
            )),
        })
    }
    /// Last displayed layered HEVC output layer; absent for ordinary sources.
    pub fn selected_layer_id(&self) -> Option<u8> {
        match &self.source {
            Source::Single(_) => None,
            Source::Layered(s) => s.selected,
        }
    }
    pub fn track(&self) -> &Track {
        match &self.source {
            Source::Single(s) => s.track(),
            Source::Layered(s) => s.source.track(),
        }
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
