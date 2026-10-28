//! Linked Vorbis/Opus programs decoded natively to one rational PCM clock.
use crate::{
    owned_audio_clock::Clock,
    owned_ogg::{Chain, Coding, Limits, Ogg},
    owned_ogg_opus::{AudioPacket, AudioStep, AudioTrack},
    owned_resample::Resampler,
};
use fvid_control::{
    CopyOptions, ProgressEvent,
    error::{Result, invalid},
};
use std::io::{Read, Write};
enum Source {
    Vorbis(crate::owned_ogg_vorbis::Program),
    Opus(crate::owned_ogg_opus::Program),
}
enum Decoder {
    Vorbis(fvid_codecs::vorbis_audio::Decoder),
    Opus(crate::owned_opus::OpusDecoder),
}
impl Source {
    fn new(chain: Chain) -> Result<Self> {
        match chain.coding {
            Coding::Vorbis => Ok(Self::Vorbis(crate::owned_ogg_vorbis::Program::from_chain(
                chain,
            )?)),
            Coding::Opus => Ok(Self::Opus(crate::owned_ogg_opus::Program::from_chain(
                chain,
            )?)),
            _ => Err(invalid("selected Ogg audio coding has no native program")),
        }
    }
    fn decoder(&self) -> Result<Decoder> {
        match self {
            Self::Vorbis(p) => Ok(Decoder::Vorbis(p.native_decoder()?)),
            Self::Opus(p) => Ok(Decoder::Opus(p.native_decoder()?)),
        }
    }
    fn rate(&self) -> u32 {
        match self {
            Self::Vorbis(p) => p.sample_rate(),
            Self::Opus(_) => 48000,
        }
    }
    fn channels(&self) -> u16 {
        match self {
            Self::Vorbis(p) => p.channels(),
            Self::Opus(p) => p.channels(),
        }
    }
    fn mask(&self) -> u32 {
        match self {
            Self::Vorbis(p) => p.output_channel_mask(),
            Self::Opus(p) => p.output_channel_mask().unwrap_or(0),
        }
    }
    fn end(&self) -> i64 {
        match self {
            Self::Vorbis(p) => p.end_samples(),
            Self::Opus(p) => p.end_samples(),
        }
    }
    fn rewind(&mut self) {
        match self {
            Self::Vorbis(p) => p.rewind(),
            Self::Opus(p) => p.rewind(),
        }
    }
    fn step(&mut self) -> Result<Option<AudioStep>> {
        match self {
            Self::Vorbis(p) => p.next_step(),
            Self::Opus(p) => p.next_step(),
        }
    }
    fn packet_size(&self) -> Option<usize> {
        match self {
            Self::Vorbis(p) => p.next_packet_size(),
            Self::Opus(p) => p.next_packet_size(),
        }
    }
    fn present(&self, packet: AudioPacket, pts: i64) -> Result<Option<AudioPacket>> {
        match self {
            Self::Vorbis(p) => p.present_decoded(packet, pts),
            Self::Opus(p) => p.present_decoded(packet, pts),
        }
    }
    fn wave_order(&self) -> Vec<usize> {
        match self {
            Self::Vorbis(p) => p.wave_channel_order().to_vec(),
            Self::Opus(p) => (0..p.channels() as usize).collect(),
        }
    }
}
struct Link {
    source: Source,
    origin: Clock,
    end: i64,
    route: Vec<usize>,
}
struct Filter {
    engine: Resampler<Vec<u8>>,
    next: i64,
    source_next: u64,
}
/// Owns compressed packets and at most one active codec/filter state. PCM is
/// emitted incrementally; neither a whole decoded link nor private files are copied.
pub struct Program {
    links: Vec<Link>,
    cursor: usize,
    decoder: Option<Decoder>,
    filter: Option<Filter>,
    rate: u32,
    channels: u16,
    mask: u32,
    end: i64,
    target: i64,
    stopped: bool,
    tracks: Vec<AudioTrack>,
    selected: usize,
    packets: u64,
    payload_bytes: u64,
    codec: &'static str,
}
impl Program {
    pub fn open(source: impl Read, limits: Limits, nth: usize) -> Result<Self> {
        Self::from_ogg(crate::owned_ogg_vorbis::Program::read(source, limits)?, nth)
    }
    pub fn from_ogg(ogg: Ogg, nth: usize) -> Result<Self> {
        let mut tracks: Vec<_> = crate::owned_ogg_audio_catalog::from_ogg(&ogg)?
            .into_iter()
            .map(|t| t.audio)
            .collect();
        let selected = crate::owned_ogg_audio_catalog::program_chains(&ogg, nth)?;
        for pair in selected.windows(2) {
            if !ogg.chains()[pair[0]].ended {
                return Err(invalid("Ogg linked audio begins before the previous EOS"));
            }
        }
        let mut chains: Vec<_> = ogg.into_chains().into_iter().map(Some).collect();
        let mut sources = Vec::new();
        for index in selected {
            sources.push(Source::new(chains[index].take().unwrap())?);
        }
        drop(chains);
        let rate = sources
            .first()
            .ok_or_else(|| invalid("Ogg audio program has no links"))?
            .rate();
        let vorbis = sources.iter().any(|s| matches!(s, Source::Vorbis(_)));
        let opus = sources.iter().any(|s| matches!(s, Source::Opus(_)));
        let codec = if vorbis && opus {
            "A_OGG/VORBIS_OPUS"
        } else if vorbis {
            "A_VORBIS"
        } else {
            "A_OPUS"
        };
        let discrete = sources.iter().any(|s| s.mask() == 0);
        let mask = if discrete {
            0
        } else {
            sources.iter().fold(0, |mask, s| mask | s.mask())
        };
        let channels = if discrete {
            sources.iter().map(Source::channels).max().unwrap()
        } else {
            mask.count_ones() as u16
        };
        let mut clock = Clock::new(rate);
        let mut links = Vec::new();
        for source in sources {
            let origin = clock;
            clock.advance(source.end(), source.rate())?;
            let order = source.wave_order();
            let source_mask = source.mask();
            let route = if discrete {
                let mut route = vec![0; order.len()];
                for (wave, &native) in order.iter().enumerate() {
                    route[native] = wave;
                }
                route
            } else {
                let positions: Vec<_> = (0..32)
                    .filter(|bit| source_mask & (1 << bit) != 0)
                    .map(|bit| (mask & ((1u32 << bit) - 1)).count_ones() as usize)
                    .collect();
                // Source PCM is native Vorbis order or ascending Opus speakers.
                let mut route = vec![0; order.len()];
                for (wave, &native) in order.iter().enumerate() {
                    route[native] = positions[wave];
                }
                route
            };
            links.push(Link {
                source,
                origin,
                end: clock.ceil()?,
                route,
            });
        }
        tracks[nth].sample_rate = rate;
        tracks[nth].channels = channels;
        Ok(Self {
            links,
            cursor: 0,
            decoder: None,
            filter: None,
            rate,
            channels,
            mask,
            end: clock.ceil()?,
            target: 0,
            stopped: false,
            tracks,
            selected: nth,
            packets: 0,
            payload_bytes: 0,
            codec,
        })
    }
    pub fn codec(&self) -> &'static str {
        self.codec
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn output_channel_mask(&self) -> u32 {
        self.mask
    }
    pub fn end_samples(&self) -> i64 {
        self.end
    }
    pub fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.tracks.clone()
    }
    pub fn audio_track_index(&self) -> usize {
        self.selected
    }
    pub fn decoded_packets(&self) -> u64 {
        self.packets
    }
    pub(crate) fn max_packet_size_in_prefix(
        &self,
        until: Option<u64>,
        max_packets: Option<u64>,
    ) -> usize {
        let mut remaining = max_packets;
        let mut maximum = 0;
        for link in &self.links {
            if remaining == Some(0) {
                break;
            }
            let start = link.origin.whole;
            if until.is_some_and(|end| start >= 0 && start as u64 >= end) {
                break;
            }
            let local_until = (link.source.rate() == self.rate && link.origin.phase.0 == 0)
                .then(|| until.map(|end| end.saturating_sub(start.max(0) as u64)))
                .flatten();
            let count = match &link.source {
                Source::Vorbis(p) => p.packet_times().len() as u64,
                Source::Opus(p) => p.packet_count() as u64,
            };
            maximum = maximum.max(match &link.source {
                Source::Vorbis(p) => p.max_packet_size_in_prefix(local_until, remaining),
                Source::Opus(p) => p.max_packet_size_in_prefix(local_until, remaining),
            });
            if let Some(rem) = &mut remaining {
                *rem = rem.saturating_sub(count);
            }
        }
        maximum
    }
    pub fn seek_to(&mut self, target: i64) -> i64 {
        self.target = target.clamp(0, self.end);
        self.cursor = self
            .links
            .iter()
            .position(|l| l.end > self.target)
            .unwrap_or(self.links.len());
        self.decoder = None;
        self.filter = None;
        self.stopped = false;
        self.packets = 0;
        self.payload_bytes = 0;
        self.target
    }
    pub fn rewind(&mut self) {
        self.seek_to(0);
    }
    fn take(&mut self) -> Result<Option<AudioPacket>> {
        let Some(filter) = self.filter.as_mut() else {
            return Ok(None);
        };
        let at = self.links[self.cursor]
            .origin
            .whole
            .checked_add(filter.next)
            .ok_or_else(|| invalid("Ogg PCM clock overflow"))?;
        let mut data = filter.engine.take_output();
        let width = self.channels as usize * 4;
        let frames = data.len() / width;
        filter.next = filter
            .next
            .checked_add(frames as i64)
            .ok_or_else(|| invalid("Ogg PCM clock overflow"))?;
        let first = (self.target - at).max(0).min(frames as i64) as usize;
        if first == frames {
            return Ok(None);
        }
        data.drain(..first * width);
        Ok(Some(AudioPacket {
            data,
            pts: (at + first as i64) as u64,
            timebase_num: 1,
            timebase_den: self.rate,
        }))
    }
    pub fn next_pcm(&mut self) -> Result<Option<AudioPacket>> {
        self.next_pcm_with(&CopyOptions::default())
    }
    /// Apply packet, cancellation, RSS and progress controls before every decode.
    pub fn next_pcm_with(&mut self, options: &CopyOptions) -> Result<Option<AudioPacket>> {
        loop {
            crate::owned_ogg_opus_export::check(options).map_err(|e| invalid(&e))?;
            if self.stopped || self.cursor >= self.links.len() {
                return Ok(None);
            }
            if options
                .max_packets
                .is_some_and(|limit| self.packets >= limit)
            {
                self.stopped = true;
                if let Some(filter) = &mut self.filter {
                    filter.engine.finish()?;
                }
                return self.take();
            }
            if self.decoder.is_none() {
                self.links[self.cursor].source.rewind();
                self.decoder = Some(self.links[self.cursor].source.decoder()?);
            }
            if self.links[self.cursor]
                .source
                .packet_size()
                .is_some_and(|n| n > options.max_packet_bytes)
            {
                return Err(invalid("Ogg audio compressed packet exceeds byte limit"));
            }
            let Some(step) = self.links[self.cursor].source.step()? else {
                if let Some(filter) = &mut self.filter {
                    filter.engine.finish()?;
                }
                let tail = self.take()?;
                self.cursor += 1;
                self.decoder = None;
                self.filter = None;
                if tail.is_some() {
                    return Ok(tail);
                }
                continue;
            };
            let packet = match step {
                AudioStep::ResetDecoder => {
                    self.decoder = Some(self.links[self.cursor].source.decoder()?);
                    continue;
                }
                AudioStep::Encoded(p) => p,
            };
            let data = match self.decoder.as_mut().unwrap() {
                Decoder::Vorbis(d) => d
                    .decode(&packet.data)?
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect(),
                Decoder::Opus(d) => d.decode_bytes(&packet.data).map_err(|e| invalid(&e))?,
            };
            self.packets += 1;
            self.payload_bytes += packet.data.len() as u64;
            if let Some(hook) = &options.progress {
                hook.emit(ProgressEvent {
                    packets: self.packets,
                    payload_bytes: self.payload_bytes,
                    done: false,
                });
            }
            crate::owned_ogg_opus_export::check(options).map_err(|e| invalid(&e))?;
            if data.is_empty() {
                continue;
            }
            let Some(pcm) = self.links[self.cursor].source.present(
                AudioPacket {
                    data,
                    pts: 0,
                    timebase_num: 1,
                    timebase_den: self.links[self.cursor].source.rate(),
                },
                packet.pts,
            )?
            else {
                continue;
            };
            let link = &self.links[self.cursor];
            let input_width = link.source.channels() as usize * 4;
            let output_width = self.channels as usize * 4;
            let frames = pcm.data.len() / input_width;
            let mut routed = vec![0; frames * output_width];
            for (source, target) in pcm
                .data
                .chunks_exact(input_width)
                .zip(routed.chunks_exact_mut(output_width))
            {
                for (channel, &position) in link.route.iter().enumerate() {
                    target[position * 4..position * 4 + 4]
                        .copy_from_slice(&source[channel * 4..channel * 4 + 4]);
                }
            }
            if self.filter.is_none() {
                let engine = Resampler::new_with_origin(
                    Vec::new(),
                    link.source.rate(),
                    self.rate,
                    self.channels,
                    pcm.pts,
                    link.origin.phase,
                )?;
                let next = i64::try_from(engine.output_position())
                    .map_err(|_| invalid("Ogg PCM clock overflow"))?;
                self.filter = Some(Filter {
                    engine,
                    next,
                    source_next: pcm.pts,
                });
            }
            let filter = self.filter.as_mut().unwrap();
            if filter.source_next != pcm.pts {
                return Err(invalid("Ogg decoded source PCM has a discontinuity"));
            }
            filter.source_next += frames as u64;
            filter.engine.write_all(&routed)?;
            if let Some(pcm) = self.take()? {
                return Ok(Some(pcm));
            }
        }
    }
    pub fn decode_pcm(
        &mut self,
        writer: &mut impl Write,
    ) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
        self.seek_to(self.target);
        let mut frames = 0;
        while let Some(pcm) = self.next_pcm()? {
            writer.write_all(&pcm.data)?;
            frames += pcm.data.len() as u64 / (u64::from(self.channels) * 4);
        }
        Ok(crate::owned_matroska_audio::AudioDecodeStats {
            sample_frames: frames,
            decoded_frames: self.packets,
            sample_rate: self.rate,
            channels: self.channels,
        })
    }
}

/// Allocation-free codec admission plus bounded filter/transport storage.
pub(crate) fn decoder_reserve(ogg: &Ogg, nth: usize) -> Result<usize> {
    let selected = crate::owned_ogg_audio_catalog::program_chains(ogg, nth)?;
    let first = &ogg.chains()[selected[0]];
    let output_rate = if first.coding == Coding::Opus {
        48000
    } else {
        fvid_codecs::vorbis::Identification::parse(&first.packets[0].data)?.sample_rate
    };
    let mut maximum = 0usize;
    for index in selected {
        let chain = &ogg.chains()[index];
        let (input_rate, block, codec) = match chain.coding {
            Coding::Vorbis => {
                let id = fvid_codecs::vorbis::Identification::parse(&chain.packets[0].data)?;
                let setup = chain
                    .packets
                    .get(2)
                    .ok_or_else(|| invalid("Vorbis setup header missing"))?;
                let admission = fvid_codecs::vorbis_admission::inspect(&setup.data)?;
                let n = 1usize << id.block_exponents[1];
                let ch = id.channels as usize;
                let dim = admission.max_dimension;
                (
                    id.sample_rate,
                    n,
                    admission.bytes
                        + ch * n * 32
                        + n * 40
                        + 64 * 1024
                        + 16 * (ch * (n / 2 + dim) + dim + ch * n / 2),
                )
            }
            Coding::Opus => {
                let head = fvid_opus::OpusHead::parse(&chain.packets[0].data)
                    .map_err(|e| invalid(&e.to_string()))?;
                let bytes = crate::owned_opus::OpusDecoder::decode_admission_bytes(
                    &chain.packets[0].data,
                    48000,
                    u16::from(head.channel_count),
                )
                .map_err(|e| invalid(&e))?;
                (48000, 5760, bytes + 5760 * 255 * 4)
            }
            _ => return Err(invalid("selected Ogg audio codec has no reserve model")),
        };
        let radius = (32u128 * u128::from(input_rate)).div_ceil(u128::from(output_rate)) + 1;
        let step = u128::from(input_rate).div_ceil(u128::from(output_rate));
        let history = (2 * radius + step + 4) * 255 * 4;
        let converted =
            (block as u128 * u128::from(output_rate)).div_ceil(u128::from(input_rate)) * 255 * 4;
        let extra = usize::try_from(history + converted + block as u128 * 255 * 4 + 64 * 1024)
            .map_err(|_| invalid("Ogg audio reserve overflow"))?;
        maximum = maximum.max(
            codec
                .checked_add(extra)
                .ok_or_else(|| invalid("Ogg audio reserve overflow"))?,
        );
    }
    Ok(maximum)
}
