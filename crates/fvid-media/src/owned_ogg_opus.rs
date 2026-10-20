//! Owned Ogg Opus presentation, including pre-skip, final granule and seek preroll.
use crate::owned_ogg::{Coding, Limits, Ogg};
use fvid_control::error::{Result, invalid, unsupported};
use std::{io::Read, time::Duration};
/// Metadata of a decoded Opus program track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub sample_rate: u32,
    pub channels: u16,
    pub name: String,
    pub language: String,
}
/// One compressed packet and its source clock, in 48 kHz ticks.
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub duration: i64,
}
/// Interleaved float32 PCM and its presentation clock.
pub struct AudioPacket {
    pub data: Vec<u8>,
    pub pts: u64,
    pub timebase_num: u32,
    pub timebase_den: u32,
}
/// Decoder-history transition or the next compressed packet.
pub enum AudioStep {
    ResetDecoder,
    Encoded(EncodedPacket),
}
fn header(data: &[u8]) -> Result<fvid_opus::OpusHead> {
    let head =
        fvid_opus::OpusHead::parse(data).map_err(|e| invalid(&format!("Opus header: {e}")))?;
    head.decoding_layout()
        .map_err(|e| invalid(&format!("Opus layout: {e}")))?;
    Ok(head)
}

enum Selection {
    Ordinal(usize),
    Source(Option<usize>),
}
struct Stamp {
    chain: usize,
    start: i64,
    end: i64,
    index: usize,
    pts: i64,
    samples: i64,
}
/// Indexed Opus programs with track selection, chaining and sample presentation.
/// Encoded timestamps use signed 48 kHz ticks. Segment reset steps expose a new
/// active header; callers rebuild decoder history or use `decode_pcm`.
pub struct Program {
    ogg: Ogg,
    decoder_chain: usize,
    stamps: Vec<Stamp>,
    cursor: usize,
    channels: u16,
    output_mask: Option<u32>,
    routes: Vec<Vec<usize>>,
    end: i64,
    target: i64,
    tracks: Vec<AudioTrack>,
}
impl Program {
    /// Negotiated speaker positions; `None` preserves discrete channel indices.
    pub fn output_channel_mask(&self) -> Option<u32> {
        self.output_mask
    }
    fn output_layout(ogg: &Ogg, chains: &[usize]) -> Result<(u16, Option<u32>)> {
        let mut mask = 0u32;
        let mut named = false;
        let mut discrete = false;
        let mut maximum = 0u16;
        for &chain in chains {
            let head = header(&ogg.chains()[chain].packets[0].data)?;
            maximum = maximum.max(u16::from(head.channel_count));
            if let Some(speakers) = head.wave_channel_mask() {
                named = true;
                mask |= speakers;
            } else {
                discrete = true;
            }
        }
        if named && discrete {
            return Err(unsupported(
                "cannot combine unidentified and speaker-mapped Opus layouts",
            ));
        }
        Ok(if named {
            (mask.count_ones() as u16, Some(mask))
        } else {
            (maximum, None)
        })
    }
    /// Index the first supported Opus track using the supplied input limits.
    pub fn open<R: Read>(source: R, limits: Limits) -> Result<Self> {
        Self::open_track(source, limits, 0)
    }
    /// Index a track in supported first-link BOS order and follow it through links.
    pub fn open_track<R: Read>(source: R, limits: Limits, nth: usize) -> Result<Self> {
        Self::open_selected(source, limits, Selection::Ordinal(nth))
    }
    /// Select a first-link logical BOS index, including non-Opus bitstreams.
    /// `None` requires exactly one supported Opus program.
    pub fn open_source_track<R: Read>(
        source: R,
        limits: Limits,
        index: Option<usize>,
    ) -> Result<Self> {
        Self::open_selected(source, limits, Selection::Source(index))
    }
    fn open_selected<R: Read>(mut source: R, limits: Limits, selection: Selection) -> Result<Self> {
        let cap =
            u64::try_from(limits.file_bytes).map_err(|_| invalid("Ogg input limit overflow"))?;
        let mut bytes = Vec::new();
        source
            .by_ref()
            .take(cap.saturating_add(1))
            .read_to_end(&mut bytes)?;
        Self::from_ogg(Ogg::parse(&bytes, &limits)?, selection)
    }
    pub(crate) fn from_chain(chain: crate::owned_ogg::Chain) -> Result<Self> {
        Self::from_ogg(Ogg::from_chain(chain), Selection::Ordinal(0))
    }
    fn from_ogg(ogg: Ogg, selection: Selection) -> Result<Self> {
        let chains: Vec<_> = ogg
            .chains()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.coding == Coding::Opus)
            .map(|(i, _)| i)
            .collect();
        if chains.is_empty() {
            return Err(unsupported("Ogg file has no Opus bitstream"));
        }
        // Overlapping page spans form one multiplexed link; successive links
        // are chained programs. Preserve BOS order as the track order per link.
        let mut links: Vec<Vec<usize>> = Vec::new();
        let mut link_end = 0;
        for chain in chains {
            let span = &ogg.chains()[chain];
            if links.is_empty() || span.first_page > link_end {
                links.push(Vec::new());
                link_end = span.last_page;
            } else {
                link_end = link_end.max(span.last_page);
            }
            links.last_mut().unwrap().push(chain);
        }
        let mut choices = Vec::new();
        let mut tracks = Vec::new();
        for (position, &chain) in links[0].iter().enumerate() {
            let stream = &ogg.chains()[chain];
            if let Ok(head) = header(&stream.packets[0].data) {
                let tags = stream
                    .packets
                    .get(1)
                    .ok_or_else(|| invalid("Ogg Opus has no comment header"))?;
                let tags = fvid_opus::OpusTags::parse(&tags.data)
                    .map_err(|e| invalid(&format!("OpusTags: {e}")))?;
                choices.push(position);
                tracks.push(AudioTrack {
                    sample_rate: 48000,
                    channels: u16::from(head.channel_count),
                    name: tags.get("TITLE").unwrap_or("").into(),
                    language: tags.get("LANGUAGE").unwrap_or("").into(),
                });
            }
        }
        for (track, &position) in tracks.iter_mut().zip(&choices) {
            let program: Option<Vec<_>> = links
                .iter()
                .map(|link| link.get(position).copied())
                .collect();
            if let Some(program) = program {
                if let Ok((channels, _)) = Self::output_layout(&ogg, &program) {
                    track.channels = channels;
                }
            }
        }
        let nth = match selection {
            Selection::Ordinal(nth) => nth,
            Selection::Source(Some(index)) => choices
                .iter()
                .position(|&position| links[0][position] == index)
                .ok_or_else(|| invalid("selected Ogg Opus source stream is absent"))?,
            Selection::Source(None) => {
                if choices.len() != 1 {
                    return Err(invalid("select exactly one audio stream"));
                }
                0
            }
        };
        let position = *choices
            .get(nth)
            .ok_or_else(|| invalid("selected Ogg Opus track is absent"))?;
        let chains: Vec<_> = links
            .iter()
            .map(|link| {
                link.get(position)
                    .copied()
                    .ok_or_else(|| unsupported("selected Opus track is absent from a chained link"))
            })
            .collect::<Result<_>>()?;
        for pair in chains.windows(2) {
            if !ogg.chains()[pair[0]].ended {
                return Err(invalid(
                    "Ogg Opus chain starts before the previous segment ended",
                ));
            }
        }
        let (channels, output_mask) = Self::output_layout(&ogg, &chains)?;
        tracks[nth].channels = channels;
        let mut routes = vec![Vec::new(); ogg.chains().len()];
        let mut stamps = Vec::new();
        let mut end = 0i64;
        for &chain in &chains {
            let stream = &ogg.chains()[chain];
            if stream.packets.len() < 3 {
                return Err(invalid("Ogg Opus has no audio packets"));
            }
            let head = header(&stream.packets[0].data)?;
            routes[chain] =
                if let (Some(mut source), Some(target)) = (head.wave_channel_mask(), output_mask) {
                    let mut positions = Vec::new();
                    while source != 0 {
                        let bit = source.trailing_zeros();
                        positions.push((target & ((1u32 << bit) - 1)).count_ones() as usize);
                        source &= source - 1;
                    }
                    positions
                } else {
                    (0..usize::from(head.channel_count)).collect()
                };
            fvid_opus::OpusTags::parse(&stream.packets[1].data)
                .map_err(|e| invalid(&format!("OpusTags: {e}")))?;
            let base = end;
            let begin = stamps.len();
            let mut coded = 0i64;
            let mut offset = None;
            let mut previous = 0i64;
            let mut final_granule = None;
            for (index, packet) in stream.packets.iter().enumerate().skip(2) {
                let samples = fvid_opus::multistream_packet_samples(
                    &packet.data,
                    usize::from(head.stream_count),
                )
                .map_err(|e| invalid(&format!("Opus packet: {e}")))?
                    as i64;
                let pts = base
                    .checked_add(coded)
                    .and_then(|n| n.checked_sub(i64::from(head.pre_skip)))
                    .ok_or_else(|| invalid("Opus sample clock overflow"))?;
                stamps.push(Stamp {
                    chain,
                    start: base,
                    end: base,
                    index,
                    pts,
                    samples,
                });
                coded = coded
                    .checked_add(samples)
                    .ok_or_else(|| invalid("Opus sample clock overflow"))?;
                let last = index + 1 == stream.packets.len() && stream.ended;
                if let Some(granule) = packet.end_granule {
                    let origin = *offset.get_or_insert((granule - coded).max(0));
                    let relative = granule
                        .checked_sub(origin)
                        .ok_or_else(|| invalid("Opus granule overflow"))?;
                    if relative < previous || relative > coded || (!last && relative != coded) {
                        return Err(invalid(
                            "Ogg Opus granule disagrees with coded packet clock",
                        ));
                    }
                    previous = relative;
                    if last {
                        final_granule = Some(relative);
                    }
                } else if last {
                    return Err(invalid("Ogg Opus EOS has no final granule"));
                }
            }
            let length = final_granule
                .unwrap_or(coded)
                .saturating_sub(i64::from(head.pre_skip))
                .max(0);
            end = base
                .checked_add(length)
                .ok_or_else(|| invalid("Opus chained clock overflow"))?;
            for stamp in &mut stamps[begin..] {
                stamp.end = end;
            }
        }
        Ok(Self {
            ogg,
            decoder_chain: chains[0],
            stamps,
            cursor: 0,
            channels,
            output_mask,
            routes,
            end,
            target: 0,
            tracks,
        })
    }
}
impl Program {
    /// Decode the configured seek window to caller-owned packed float32 PCM.
    /// History is rebuilt from the selected segment's beginning on each call.
    /// Input indexing is bounded by `Limits`; output is streamed, not retained.
    /// An I/O or codec error may leave partial PCM in the caller's writer.
    pub fn decode_pcm(
        &mut self,
        output: &mut impl std::io::Write,
    ) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
        self.seek_to(self.target);
        let mut decoder = self.native_decoder()?;
        let mut stats = crate::owned_matroska_audio::AudioDecodeStats {
            sample_frames: 0,
            decoded_frames: 0,
            sample_rate: 48000,
            channels: self.channels,
        };
        while let Some(step) = self.next_step()? {
            match step {
                AudioStep::ResetDecoder => decoder = self.native_decoder()?,
                AudioStep::Encoded(packet) => {
                    let data = decoder
                        .decode_bytes(&packet.data)
                        .map_err(|e| invalid(&format!("Opus decode: {e}")))?;
                    stats.decoded_frames = stats
                        .decoded_frames
                        .checked_add(1)
                        .ok_or_else(|| invalid("Opus decoded frame count overflow"))?;
                    if let Some(pcm) = self.present_decoded(
                        AudioPacket {
                            data,
                            pts: 0,
                            timebase_num: 1,
                            timebase_den: 48000,
                        },
                        packet.pts,
                    )? {
                        let frames = (pcm.data.len() / (usize::from(self.channels) * 4)) as u64;
                        output.write_all(&pcm.data)?;
                        stats.sample_frames = stats
                            .sample_frames
                            .checked_add(frames)
                            .ok_or_else(|| invalid("Opus sample count overflow"))?;
                    }
                }
            }
        }
        Ok(stats)
    }
    pub(crate) fn native_decoder(&self) -> Result<crate::owned_opus::OpusDecoder> {
        let head = header(self.extra_data())?;
        crate::owned_opus::OpusDecoder::new(self.extra_data(), 48000, u16::from(head.channel_count))
            .map_err(|e| invalid(&format!("Opus initialization: {e}")))
    }
    pub(crate) fn next_packet_size(&self) -> Option<usize> {
        self.stamps
            .get(self.cursor)
            .map(|s| self.ogg.chains()[s.chain].packets[s.index].data.len())
    }
    pub(crate) fn next_presentation_start(&self) -> Option<i64> {
        self.stamps.get(self.cursor).map(|s| s.pts.max(s.start))
    }
    pub(crate) fn decoder_admission_bytes(&self) -> Result<usize> {
        let mut maximum = 0;
        let mut previous = None;
        for stamp in &self.stamps {
            if previous == Some(stamp.chain) {
                continue;
            }
            previous = Some(stamp.chain);
            let data = &self.ogg.chains()[stamp.chain].packets[0].data;
            let head = header(data)?;
            let bytes = crate::owned_opus::OpusDecoder::decode_admission_bytes(
                data,
                48000,
                u16::from(head.channel_count),
            )
            .map_err(|e| invalid(&e))?;
            maximum = maximum.max(bytes);
        }
        Ok(maximum)
    }
    pub(crate) fn source_tags(&self) -> Result<fvid_opus::OpusTags> {
        fvid_opus::OpusTags::parse(&self.ogg.chains()[self.decoder_chain].packets[1].data)
            .map_err(|e| invalid(&format!("OpusTags: {e}")))
    }
    pub fn codec(&self) -> &str {
        "A_OPUS"
    }
    pub fn timescale(&self) -> u32 {
        48000
    }
    pub fn sample_rate(&self) -> u32 {
        48000
    }
    /// Negotiated output channels, which may exceed an individual segment's count.
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Active source OpusHead, updated by seek, rewind and segment reset steps.
    pub fn extra_data(&self) -> &[u8] {
        &self.ogg.chains()[self.decoder_chain].packets[0].data
    }
    pub fn end_samples(&self) -> i64 {
        self.end
    }
    pub fn duration(&self) -> Option<Duration> {
        Some(Duration::new(
            self.end as u64 / 48000,
            ((self.end as u64 % 48000) * 1_000_000_000 / 48000) as u32,
        ))
    }
    pub fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.tracks.clone()
    }
    /// Next encoded packet; use `next_step` to handle chained decoder resets.
    pub fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(stamp) = self.stamps.get(self.cursor) else {
            return Ok(None);
        };
        if stamp.chain != self.decoder_chain {
            return Err(invalid(
                "Ogg Opus chain boundary requires next_step decoder reset",
            ));
        }
        let packet = EncodedPacket {
            data: self.ogg.chains()[stamp.chain].packets[stamp.index]
                .data
                .clone(),
            pts: stamp.pts,
            duration: stamp.samples,
        };
        self.cursor += 1;
        Ok(Some(packet))
    }
    /// Next packet or a boundary reset; the active header is set before reset.
    pub fn next_step(&mut self) -> Result<Option<AudioStep>> {
        if let Some(stamp) = self.stamps.get(self.cursor) {
            if stamp.chain != self.decoder_chain {
                self.decoder_chain = stamp.chain;
                return Ok(Some(AudioStep::ResetDecoder));
            }
        }
        self.next_packet()
            .map(|packet| packet.map(AudioStep::Encoded))
    }
    pub fn rewind(&mut self) {
        self.cursor = 0;
        self.target = 0;
        if let Some(stamp) = self.stamps.first() {
            self.decoder_chain = stamp.chain;
        }
    }
    /// Configure an exact output sample target and preroll from its segment start.
    pub fn seek_to(&mut self, pts: i64) -> i64 {
        self.target = pts.clamp(0, self.end);
        let selected = self
            .stamps
            .iter()
            .find(|s| s.end > self.target)
            .or_else(|| self.stamps.last())
            .map(|s| s.chain);
        if let Some(chain) = selected {
            self.cursor = self.stamps.iter().position(|s| s.chain == chain).unwrap();
            self.decoder_chain = chain;
        }
        self.target
    }
    pub fn preroll_target(&self) -> Option<i64> {
        Some(self.target)
    }
    /// Trim the just-consumed packet in its source layout, then preserve sample
    /// bytes while routing into the negotiated layout. `source_pts` is the
    /// signed timestamp returned with that encoded packet.
    pub fn present_decoded(
        &self,
        mut packet: AudioPacket,
        source_pts: i64,
    ) -> Result<Option<AudioPacket>> {
        let stamp = self
            .cursor
            .checked_sub(1)
            .and_then(|i| self.stamps.get(i))
            .ok_or_else(|| invalid("Opus presentation has no source packet"))?;
        let route = &self.routes[stamp.chain];
        let width = route.len() * 4;
        if stamp.pts != source_pts
            || packet.data.len() % width != 0
            || packet.data.len() / width != stamp.samples as usize
        {
            return Err(invalid(
                "Opus decoded packet geometry disagrees with Ogg clock",
            ));
        }
        let from = source_pts.max(self.target).max(stamp.start);
        let to = (source_pts + stamp.samples).min(stamp.end);
        if to <= from {
            return Ok(None);
        }
        let first = (from - source_pts) as usize * width;
        let last = (to - source_pts) as usize * width;
        if route.len() == usize::from(self.channels)
            && route.iter().enumerate().all(|(i, &target)| i == target)
        {
            packet.data.copy_within(first..last, 0);
            packet.data.truncate(last - first);
        } else {
            let output_width = usize::from(self.channels) * 4;
            let size = ((last - first) / width)
                .checked_mul(output_width)
                .ok_or_else(|| invalid("Opus presentation size overflow"))?;
            let mut output = Vec::new();
            output
                .try_reserve_exact(size)
                .map_err(|_| invalid("Opus presentation allocation failed"))?;
            output.resize(size, 0);
            for (source, target) in packet.data[first..last]
                .chunks_exact(width)
                .zip(output.chunks_exact_mut(output_width))
            {
                for (channel, &position) in route.iter().enumerate() {
                    target[position * 4..position * 4 + 4]
                        .copy_from_slice(&source[channel * 4..channel * 4 + 4]);
                }
            }
            packet.data = output;
        }
        packet.pts = from as u64;
        packet.timebase_num = 1;
        packet.timebase_den = 48000;
        Ok(Some(packet))
    }
}
