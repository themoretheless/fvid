//! Ogg Vorbis presentation with overlap clocks, granule trims and chain resets.
use crate::owned_audio_clock::Clock;
use crate::owned_ogg::{Coding, Limits, Ogg};
use crate::owned_ogg_opus::{AudioPacket, AudioStep, AudioTrack, EncodedPacket};
use fvid_control::error::{Result, invalid, unsupported};
use std::time::Duration;
use std::{
    cell::RefCell,
    io::{Read, Write},
};

/// The native codec tag for Vorbis, as Matroska
/// spells it - the same tag a WebM track of this codec arrives under.
pub const VORBIS: &str = "A_VORBIS";

/// The second and third of the three packets every Vorbis bitstream opens with.
/// The first is what names the bitstream, so the container has already read it.
const COMMENT: &[u8] = b"\x03vorbis";
const SETUP: &[u8] = b"\x05vorbis";

/// One audio packet of the timeline: which packet of which chain it is, and where
/// it sits on the negotiated output clock. Decoder overlap lengths remain
/// separately measured in source samples.
#[derive(Clone, Copy)]
struct Stamp {
    chain: usize,
    packet: usize,
    pts: i64,
    duration: i64,
    end: i64,
    start: i64,
    input_pts: i64,
    input_duration: i64,
    input_end: i64,
    input_rate: u32,
    clock: Clock,
}

/// What an identification header states about the track, once checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sample_rate: u32,
    pub channels: u16,
    /// The two block sizes as their powers of two, in the order a packet's own bit
    /// picks between them.
    pub blocks: [u8; 2],
}

/// Read the 30 bytes an identification header is made of. Every field is checked
/// against what the decoder down the line does with it: a header whose block sizes
/// are out of range or the wrong way round states a timeline this reader cannot lay
/// out. The native decoder admits the same block geometry.
pub fn geometry(ident: &[u8]) -> Result<Geometry> {
    // The concatenated form handed to the decoder assumes the header is exactly as
    // long as the codec defines, since the setup packet starts at the byte after it.
    let header: [u8; 30] = ident.try_into().map_err(|_| {
        invalid(&format!(
            "a Vorbis identification header is 30 bytes and this one is {}",
            ident.len()
        ))
    })?;
    let version = u32::from_le_bytes(header[7..11].try_into().expect("four version bytes"));
    if version != 0 {
        return Err(invalid(&format!(
            "Vorbis version {version} is not the one bitstream this reader times"
        )));
    }
    let width = header[11];
    if width == 0 {
        return Err(invalid(&format!(
            "a Vorbis track of {width} channels has no audio channels"
        )));
    }
    let channels = u16::from(width);
    let sample_rate = u32::from_le_bytes(header[12..16].try_into().expect("four rate bytes"));
    if sample_rate == 0 {
        return Err(invalid("a Vorbis track states a sample rate of zero"));
    }
    // The two exponents share one byte and the framing marker follows them, which
    // is what tells this header from one a draft writer left half-finished.
    let blocks = [header[28] & 0x0f, header[28] >> 4];
    if header[29] != 1 {
        return Err(invalid(&format!(
            "a Vorbis identification header ends with {:#04x} where its framing marker is 1",
            header[29]
        )));
    }
    for (which, exponent) in [("short", blocks[0]), ("long", blocks[1])] {
        if !(6..=13).contains(&exponent) {
            return Err(invalid(&format!(
                "the {which} Vorbis block size exponent {exponent} is out of the range 6..=13"
            )));
        }
    }
    if blocks[0] > blocks[1] {
        return Err(invalid(&format!(
            "a Vorbis track's short block size exponent {} exceeds its long one {}",
            blocks[0], blocks[1]
        )));
    }
    Ok(Geometry {
        sample_rate,
        channels,
        blocks,
    })
}

enum Selection {
    Ordinal(usize),
    Source(Option<usize>),
}
fn select(ogg: &Ogg, selection: Selection) -> Result<(Vec<usize>, Vec<usize>, usize)> {
    let mut ordered: Vec<_> = (0..ogg.chains().len()).collect();
    ordered.sort_by_key(|&i| ogg.chains()[i].first_page);
    let mut links: Vec<Vec<usize>> = Vec::new();
    let mut end = 0;
    for index in ordered {
        let chain = &ogg.chains()[index];
        if links.is_empty() || chain.first_page > end {
            links.push(Vec::new());
            end = chain.last_page;
        } else {
            end = end.max(chain.last_page);
        }
        links.last_mut().unwrap().push(index);
    }
    let choices: Vec<_> = links
        .first()
        .ok_or_else(|| invalid("Ogg contains no logical streams"))?
        .iter()
        .copied()
        .filter(|&i| ogg.chains()[i].coding == Coding::Vorbis)
        .collect();
    let nth = match selection {
        Selection::Ordinal(nth) => nth,
        Selection::Source(Some(index)) => choices
            .iter()
            .position(|&i| i == index)
            .ok_or_else(|| invalid("selected Vorbis source stream is absent from first link"))?,
        Selection::Source(None) => {
            if choices.len() != 1 {
                return Err(invalid("select exactly one Vorbis audio stream"));
            }
            0
        }
    };
    if nth >= choices.len() {
        return Err(invalid("selected Vorbis audio track is absent"));
    }
    let mut selected = Vec::new();
    for link in links {
        selected.push(
            link.into_iter()
                .filter(|&i| ogg.chains()[i].coding == Coding::Vorbis)
                .nth(nth)
                .ok_or_else(|| invalid("selected Vorbis program is absent in a later link"))?,
        );
    }
    Ok((selected, choices, nth))
}
pub(crate) fn select_source_chains(ogg: &Ogg, index: Option<usize>) -> Result<Vec<usize>> {
    select(ogg, Selection::Source(index)).map(|p| p.0)
}

/// Vorbis speaker order for defined layouts. Larger configurations preserve
/// discrete codec indices and intentionally carry no speaker mask.
fn speakers(channels: u16) -> Option<&'static [u32]> {
    Some(match channels {
        1 => &[4],
        2 => &[1, 2],
        3 => &[1, 4, 2],
        4 => &[1, 2, 16, 32],
        5 => &[1, 4, 2, 16, 32],
        6 => &[1, 4, 2, 16, 32, 8],
        7 => &[1, 4, 2, 512, 1024, 256, 8],
        8 => &[1, 4, 2, 512, 1024, 16, 32, 8],
        _ => return None,
    })
}
fn output_layout(
    ogg: &Ogg,
    chains: &[usize],
) -> Result<(u16, u32, Vec<usize>, Vec<(usize, Vec<usize>)>)> {
    let mut maximum = 0;
    let mut mask = 0u32;
    let mut discrete = false;
    let mut shapes = Vec::new();
    for &chain in chains {
        let shape = geometry(&ogg.chains()[chain].packets[0].data)?;
        maximum = maximum.max(shape.channels);
        if let Some(order) = speakers(shape.channels) {
            mask |= order.iter().fold(0, |mask, &speaker| mask | speaker);
        } else {
            discrete = true;
        }
        shapes.push((chain, shape.channels));
    }
    if discrete {
        return Ok((
            maximum,
            0,
            (0..maximum as usize).collect(),
            shapes
                .into_iter()
                .map(|(chain, ch)| (chain, (0..ch as usize).collect()))
                .collect(),
        ));
    }
    // Preserve native ordering when the union is a defined Vorbis layout.
    // Otherwise retain all speaker identities in ascending WAVE order.
    let order = (1..=8)
        .filter_map(speakers)
        .find(|order| order.iter().fold(0, |m, &s| m | s) == mask)
        .map(|order| order.to_vec())
        .unwrap_or_else(|| {
            (0..32)
                .filter_map(|bit| (mask & (1u32 << bit) != 0).then_some(1u32 << bit))
                .collect()
        });
    let routes = shapes
        .into_iter()
        .map(|(chain, ch)| {
            let route = speakers(ch)
                .unwrap()
                .iter()
                .map(|speaker| order.iter().position(|s| s == speaker).unwrap())
                .collect();
            (chain, route)
        })
        .collect();
    let mut wave_order: Vec<usize> = (0..order.len()).collect();
    wave_order.sort_by_key(|&i| order[i]);
    Ok((order.len() as u16, mask, wave_order, routes))
}

struct RateState {
    chain: usize,
    engine: crate::owned_resample::Resampler<Vec<u8>>,
    next: i64,
    base: i64,
}
impl RateState {
    fn take(&mut self, channels: u16, rate: u32, target: i64) -> Result<Option<AudioPacket>> {
        let at = self
            .base
            .checked_add(self.next)
            .ok_or_else(|| invalid("Vorbis resample clock overflow"))?;
        let mut data = self.engine.take_output();
        let width = usize::from(channels) * 4;
        let frames = data.len() / width;
        self.next = self
            .next
            .checked_add(frames as i64)
            .ok_or_else(|| invalid("Vorbis resample clock overflow"))?;
        let first = (target - at).max(0).min(frames as i64) as usize;
        if first == frames {
            return Ok(None);
        }
        data.drain(..first * width);
        Ok(Some(AudioPacket {
            data,
            pts: u64::try_from(at + first as i64)
                .map_err(|_| invalid("negative Vorbis output time"))?,
            timebase_num: 1,
            timebase_den: rate,
        }))
    }
}

/// An Ogg file's Vorbis track, read as an audio stream.
pub struct Program {
    ogg: Ogg,
    packets: Vec<Stamp>,
    packet: usize,
    shape: Geometry,
    /// The identification and setup packets concatenated, which is the form the
    /// decoder unpacks - without the comment packet that sits between them in the
    /// stream.
    configurations: Vec<(usize, Vec<u8>)>,
    /// Samples the chains state that a listener hears, added up.
    stated: i64,
    active_chain: Option<usize>,
    target: i64,
    tracks: Vec<AudioTrack>,
    selected_track: usize,
    output_mask: u32,
    wave_order: Vec<usize>,
    routes: Vec<(usize, Vec<usize>)>,
    rate_state: RefCell<Option<RateState>>,
    rate_changes: bool,
}

impl Program {
    /// Open the file's Vorbis track, whichever chain of the file it sits in. A file
    /// carrying only another coding is refused by its own name: the container was
    /// read, and what it names is the coding with no arm here, which is a different
    /// gap from a container nobody opens.
    pub fn open<R: Read>(reader: R, limits: Limits) -> Result<Self> {
        Self::open_track(reader, limits, 0)
    }
    pub fn open_track<R: Read>(reader: R, limits: Limits, nth: usize) -> Result<Self> {
        Self::open_selected(reader, limits, Selection::Ordinal(nth))
    }
    pub fn open_source_track<R: Read>(
        reader: R,
        limits: Limits,
        index: Option<usize>,
    ) -> Result<Self> {
        Self::open_selected(reader, limits, Selection::Source(index))
    }
    fn open_selected<R: Read>(reader: R, limits: Limits, selection: Selection) -> Result<Self> {
        Self::from_ogg(Self::read(reader, limits)?, selection)
    }
    pub(crate) fn from_chain(chain: crate::owned_ogg::Chain) -> Result<Self> {
        Self::from_ogg(Ogg::from_chain(chain), Selection::Ordinal(0))
    }
    fn from_ogg(ogg: Ogg, selection: Selection) -> Result<Self> {
        let playable: Vec<usize> = ogg
            .chains()
            .iter()
            .enumerate()
            .filter(|(_, chain)| chain.coding == Coding::Vorbis)
            .map(|(number, _)| number)
            .collect();
        if playable.is_empty() {
            let named: Vec<String> = ogg
                .chains()
                .iter()
                .map(|chain| {
                    let opening = chain
                        .packets
                        .first()
                        .map(|packet| packet.data.as_slice())
                        .unwrap_or_default();
                    chain.coding.describe(opening)
                })
                .collect();
            return Err(unsupported(&format!(
                "Ogg bitstream coded {} has no decoder arm here",
                named.join(" / ")
            )));
        }
        let (playable, choices, selected_track) = select(&ogg, selection)?;
        let mut tracks: Vec<AudioTrack> = choices
            .into_iter()
            .map(|i| {
                let data = &ogg.chains()[i].packets[0].data;
                let shape = geometry(data).ok();
                AudioTrack {
                    sample_rate: shape.map_or(0, |s| s.sample_rate),
                    channels: shape.map_or(0, |s| s.channels),
                    name: String::new(),
                    language: String::new(),
                }
            })
            .collect();
        // Each selected link has its own configuration and sample clock.
        let headers = &ogg.chains()[playable[0]].packets;
        if headers.len() < 3
            || !headers[1].data.starts_with(COMMENT)
            || !headers[2].data.starts_with(SETUP)
        {
            return Err(invalid(
                "an Ogg Vorbis bitstream does not hold its three header packets in the order the codec defines",
            ));
        }
        let mut shape = geometry(&headers[0].data)?;
        let (channels, output_mask, wave_order, routes) = output_layout(&ogg, &playable)?;
        shape.channels = channels;
        tracks[selected_track].channels = channels;
        let mut configurations = Vec::new();
        let mut stated = 0i64;
        let mut clock = Clock::new(shape.sample_rate);
        let mut rate_changes = false;
        let mut packets: Vec<Stamp> = Vec::new();
        for chain in playable {
            let stream = &ogg.chains()[chain];
            if stream.packets.len() < 3
                || !stream.packets[1].data.starts_with(COMMENT)
                || !stream.packets[2].data.starts_with(SETUP)
            {
                return Err(invalid(
                    "Vorbis chain has missing identification/comment/setup headers",
                ));
            }
            let chain_shape = geometry(&stream.packets[0].data)?;
            rate_changes |= chain_shape.sample_rate != shape.sample_rate;
            let setup = fvid_codecs::vorbis::Setup::parse(
                &stream.packets[2].data,
                chain_shape.channels as u8,
                fvid_codecs::vorbis_admission::inspect(&stream.packets[2].data)?.limits,
            )?;
            let mut configuration = stream.packets[0].data.clone();
            configuration.extend_from_slice(&stream.packets[2].data);
            configurations.push((chain, configuration));
            if stream.packets.len() < 4 {
                return Err(invalid(&format!(
                    "bitstream {} is three header packets and no audio",
                    stream.serial
                )));
            }
            let audio: Vec<_> = stream.packets[3..]
                .iter()
                .map(|p| (p.data.as_slice(), p.end_granule))
                .collect();
            let timeline = fvid_codecs::vorbis_timeline::build(
                chain_shape.blocks,
                &setup.modes,
                &audio,
                stream.ended,
            )?;
            let origin = clock;
            let start = origin.ceil()?;
            clock.advance(timeline.end, chain_shape.sample_rate)?;
            stated = clock.ceil()?;
            for (index, frame) in timeline.frames.into_iter().enumerate() {
                packets.push(Stamp {
                    chain,
                    packet: index + 3,
                    pts: origin.point(frame.pts, chain_shape.sample_rate)?,
                    duration: origin
                        .point(
                            frame
                                .pts
                                .checked_add(frame.duration)
                                .ok_or_else(|| invalid("Vorbis packet clock overflow"))?,
                            chain_shape.sample_rate,
                        )?
                        .checked_sub(origin.point(frame.pts, chain_shape.sample_rate)?)
                        .ok_or_else(|| invalid("Vorbis packet duration overflow"))?,
                    end: stated,
                    start,
                    input_pts: frame.pts,
                    input_duration: frame.duration,
                    input_end: timeline.end,
                    input_rate: chain_shape.sample_rate,
                    clock: origin,
                });
            }
        }
        Ok(Self {
            ogg,
            packets,
            packet: 0,
            shape,
            configurations,
            stated,
            active_chain: None,
            target: 0,
            tracks,
            selected_track,
            output_mask,
            wave_order,
            routes,
            rate_state: RefCell::new(None),
            rate_changes,
        })
    }

    /// Take the whole file in: Ogg packets carry no lengths of their own, so the
    /// page walk that rebuilds them reads every byte.
    pub fn read<R: Read>(mut reader: R, limits: Limits) -> Result<Ogg> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader
            .by_ref()
            .take(cap.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Ogg::parse(&bytes, &limits)
    }

    /// The container's own shape, for a caller that wants to say what it opened.
    pub fn ogg(&self) -> &Ogg {
        &self.ogg
    }
}

impl Program {
    pub fn codec(&self) -> &str {
        VORBIS
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    pub fn timescale(&self) -> u32 {
        self.shape.sample_rate
    }

    pub fn sample_rate(&self) -> u32 {
        self.shape.sample_rate
    }

    pub fn has_rate_changes(&self) -> bool {
        self.rate_changes
    }
    pub fn output_channel_mask(&self) -> u32 {
        self.output_mask
    }
    /// Source positions in the negotiated PCM frame, in WAVE speaker order.
    pub fn wave_channel_order(&self) -> &[usize] {
        &self.wave_order
    }

    pub fn channels(&self) -> u16 {
        self.shape.channels
    }

    /// What the chains' pages state as their ends, added up: the samples a listener
    /// hears, which is shorter than the timeline above by the pre-roll a Vorbis
    /// stream encodes in front of the audio.
    pub fn duration(&self) -> Option<Duration> {
        let samples = u64::try_from(self.stated).ok()?;
        let rate = u64::from(self.shape.sample_rate);
        Some(Duration::new(samples / rate, ((samples % rate) * 1_000_000_000 / rate) as u32))
    }

    pub fn extra_data(&self) -> &[u8] {
        let chain = self
            .active_chain
            .or_else(|| self.packets.get(self.packet).map(|stamp| stamp.chain));
        self.configurations
            .iter()
            .find(|(index, _)| Some(*index) == chain)
            .unwrap_or(&self.configurations[0])
            .1
            .as_slice()
    }
    /// One program: a file's chains run one after another rather than beside each
    /// other, so there is no second track to choose.
    pub fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.tracks.clone()
    }
    pub fn audio_track_index(&self) -> usize {
        self.selected_track
    }

    pub fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.packets.get(self.packet) else {
            return Ok(None);
        };
        if self.active_chain.is_some_and(|chain| chain != at.chain) {
            return Err(invalid(
                "Vorbis chain boundary requires next_step decoder reset",
            ));
        }
        self.active_chain = Some(at.chain);
        let packet = EncodedPacket {
            data: self.ogg.chains()[at.chain].packets[at.packet].data.clone(),
            pts: at.pts,
            duration: at.duration,
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    pub fn next_step(&mut self) -> Result<Option<AudioStep>> {
        let Some(stamp) = self.packets.get(self.packet) else {
            return Ok(None);
        };
        if self.active_chain.is_some_and(|chain| chain != stamp.chain) {
            self.active_chain = Some(stamp.chain);
            return Ok(Some(AudioStep::ResetDecoder));
        }
        self.active_chain = Some(stamp.chain);
        self.next_packet().map(|p| p.map(AudioStep::Encoded))
    }
    pub fn preroll_target(&self) -> Option<i64> {
        Some(self.target)
    }
    pub fn present_decoded(
        &self,
        mut pcm: AudioPacket,
        source_pts: i64,
    ) -> Result<Option<AudioPacket>> {
        let stamp = self
            .packet
            .checked_sub(1)
            .and_then(|i| self.packets.get(i))
            .ok_or_else(|| invalid("Vorbis PCM without source packet"))?;
        let route = &self
            .routes
            .iter()
            .find(|(chain, _)| *chain == stamp.chain)
            .ok_or_else(|| invalid("missing Vorbis channel route"))?
            .1;
        let width = route.len() * 4;
        if source_pts != stamp.pts
            || pcm.data.len() % width != 0
            || pcm.data.len() / width != stamp.input_duration as usize
        {
            return Err(invalid(
                "Vorbis decoded PCM disagrees with packet overlap clock",
            ));
        }
        let resample = stamp.input_rate != self.sample_rate() || stamp.clock.phase.0 != 0;
        let start = stamp.input_pts.max(0).max(if resample {
            0
        } else {
            self.target - stamp.clock.whole
        });
        let end = stamp
            .input_pts
            .checked_add(stamp.input_duration)
            .ok_or_else(|| invalid("Vorbis PCM clock overflow"))?
            .min(stamp.input_end);
        let from = (start - stamp.input_pts).clamp(0, stamp.input_duration) as usize * width;
        let to = (end - stamp.input_pts).clamp(0, stamp.input_duration) as usize * width;
        if from >= to {
            if !resample {
                return Ok(None);
            }
            pcm.data.clear();
        }
        if pcm.data.is_empty() {
            // An EOS packet can be entirely trimmed while still needing to
            // release the resampler's previously buffered tail.
        } else if route.len() == usize::from(self.channels())
            && route.iter().enumerate().all(|(i, &target)| i == target)
        {
            pcm.data.copy_within(from..to, 0);
            pcm.data.truncate(to - from);
        } else {
            let output_width = usize::from(self.channels()) * 4;
            let size = ((to - from) / width)
                .checked_mul(output_width)
                .ok_or_else(|| invalid("Vorbis channel route size overflow"))?;
            let mut output = Vec::new();
            output
                .try_reserve_exact(size)
                .map_err(|_| invalid("cannot allocate Vorbis channel route"))?;
            output.resize(size, 0);
            for (source, target) in pcm.data[from..to]
                .chunks_exact(width)
                .zip(output.chunks_exact_mut(output_width))
            {
                for (channel, &position) in route.iter().enumerate() {
                    target[position * 4..position * 4 + 4]
                        .copy_from_slice(&source[channel * 4..channel * 4 + 4]);
                }
            }
            pcm.data = output;
        }
        if resample {
            let mut state = self.rate_state.borrow_mut();
            if state.as_ref().is_none_or(|s| s.chain != stamp.chain) {
                if pcm.data.is_empty() {
                    return Ok(None);
                }
                let engine = crate::owned_resample::Resampler::new_with_origin(
                    Vec::new(),
                    stamp.input_rate,
                    self.sample_rate(),
                    self.channels(),
                    start as u64,
                    stamp.clock.phase,
                )?;
                let next = i64::try_from(engine.output_position())
                    .map_err(|_| invalid("Vorbis resample clock overflow"))?;
                *state = Some(RateState {
                    chain: stamp.chain,
                    engine,
                    next,
                    base: stamp.clock.whole,
                });
            }
            let state = state.as_mut().unwrap();
            state.engine.write_all(&pcm.data)?;
            if stamp.packet + 1 == self.ogg.chains()[stamp.chain].packets.len() {
                state.engine.finish()?;
            }
            return state.take(self.channels(), self.sample_rate(), self.target);
        } else {
            pcm.pts = u64::try_from(stamp.clock.whole + start)
                .map_err(|_| invalid("negative Vorbis output time"))?;
        }
        pcm.timebase_num = 1;
        pcm.timebase_den = self.shape.sample_rate;
        Ok(Some(pcm))
    }
    /// Finalize the PCM prefix already decoded, without consuming more packets.
    /// Rewind or seek before continuing the program after an early finalization.
    pub fn finish_pcm_prefix(&self) -> Result<Option<AudioPacket>> {
        let mut state = self.rate_state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return Ok(None);
        };
        state.engine.finish()?;
        state.take(self.channels(), self.sample_rate(), self.target)
    }
    pub fn rewind(&mut self) {
        self.packet = 0;
        self.active_chain = None;
        self.target = 0;
        *self.rate_state.get_mut() = None;
    }
    pub fn seek_to(&mut self, pts: i64) -> i64 {
        self.target = pts.clamp(0, self.stated);
        self.packet = self
            .packets
            .iter()
            .position(|stamp| stamp.end > self.target)
            .unwrap_or(self.packets.len());
        self.active_chain = None;
        *self.rate_state.get_mut() = None;
        self.target
    }
}

impl Program {
    pub fn packet_times(&self) -> impl ExactSizeIterator<Item = (i64, i64)> + '_ {
        self.packets.iter().map(|p| (p.pts, p.duration))
    }
    pub fn end_samples(&self) -> i64 {
        self.stated
    }
    pub fn native_decoder(&self) -> Result<fvid_codecs::vorbis_audio::Decoder> {
        let (ident, setup) = fvid_codecs::vorbis_audio::configuration_headers(self.extra_data())?;
        let ident = fvid_codecs::vorbis::Identification::parse(ident)?;
        let setup = fvid_codecs::vorbis::Setup::parse(
            setup,
            ident.channels,
            fvid_codecs::vorbis_admission::inspect(setup)?.limits,
        )?;
        let n = 1usize << ident.block_exponents[1];
        let ch = usize::from(ident.channels);
        let dimension = setup
            .codebooks
            .iter()
            .map(|b| usize::from(b.dimensions))
            .max()
            .unwrap_or(0);
        let workspace = ch * n * 32
            + n * 40
            + 64 * 1024
            + 16 * (ch * (n / 2 + dimension) + dimension + ch * n / 2);
        fvid_codecs::vorbis_audio::Decoder::new(ident, setup, workspace)
    }
    /// Stream native PCM from the exact configured seek position. Rebuilds codec
    /// history per chain; a failed writer may contain partial PCM.
    pub fn decode_pcm(
        &mut self,
        writer: &mut impl std::io::Write,
    ) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
        self.seek_to(self.target);
        let mut decoder = self.native_decoder()?;
        let mut stats = crate::owned_matroska_audio::AudioDecodeStats {
            decoded_frames: 0,
            sample_frames: 0,
            sample_rate: self.sample_rate(),
            channels: self.channels(),
        };
        while let Some(step) = self.next_step()? {
            match step {
                AudioStep::ResetDecoder => decoder = self.native_decoder()?,
                AudioStep::Encoded(packet) => {
                    let samples = decoder.decode(&packet.data)?;
                    stats.decoded_frames += 1;
                    if samples.is_empty() {
                        continue;
                    }
                    let mut data = Vec::new();
                    data.try_reserve_exact(samples.len() * 4)
                        .map_err(|_| invalid("cannot allocate Vorbis PCM transport"))?;
                    for sample in samples {
                        data.extend_from_slice(&sample.to_le_bytes());
                    }
                    if let Some(pcm) = self.present_decoded(
                        AudioPacket {
                            data,
                            pts: 0,
                            timebase_num: 1,
                            timebase_den: self.sample_rate(),
                        },
                        packet.pts,
                    )? {
                        writer.write_all(&pcm.data)?;
                        stats.sample_frames +=
                            pcm.data.len() as u64 / (u64::from(self.channels()) * 4);
                    }
                }
            }
        }
        Ok(stats)
    }
}

impl Program {
    pub(crate) fn next_packet_size(&self) -> Option<usize> {
        self.packets
            .get(self.packet)
            .map(|p| self.ogg.chains()[p.chain].packets[p.packet].data.len())
    }
    pub(crate) fn next_presentation_start(&self) -> Option<i64> {
        self.packets.get(self.packet).map(|p| p.pts.max(p.start))
    }
}
