//! Owned Matroska AAC/ALAC/PCM presentation timeline to caller-owned float32 PCM.
use crate::owned_aac::aac_ps_native::InBandPsProbe;
use crate::owned_matroska_sample_clock::quantized_sample_matches as aac_quantized_sample_matches;
use crate::owned_matroska_grid_clock::grid_sample_matches as aac_quantized_grid_matches;
pub use crate::owned_aac::stream::AudioDecodeStats;
use crate::owned_webm::WebmReader as MatroskaTimelineReader;
use fvid_control::{CopyOptions, ProgressEvent};
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
#[derive(Debug)]
pub struct Error(String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<crate::owned_ebml::Error> for Error {
    fn from(e: crate::owned_ebml::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<crate::owned_alac::Error> for Error {
    fn from(e: crate::owned_alac::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<crate::owned_aac::Error> for Error {
    fn from(e: crate::owned_aac::Error) -> Self {
        Self(e.to_string())
    }
}
impl From<crate::owned_pcm_decoder::Error> for Error {
    fn from(e: crate::owned_pcm_decoder::Error) -> Self {
        Self(e.to_string())
    }
}
type Result<T> = std::result::Result<T, Error>;
pub(crate) fn invalid(message: &str) -> Error {
    Error(message.into())
}
pub(crate) enum MatroskaTimelineDecoder {
    Pcm(crate::owned_pcm_decoder::PcmDecoder),
    Alac(crate::owned_alac::AlacDecoder),
    Aac(crate::owned_aac::NativeAacDecoder),
    Ps(Box<crate::owned_aac::aac_ps_native::NativePsAacDecoder>),
    Opus(crate::owned_opus::OpusDecoder),
    Mp3(Box<Mp3TimelineDecoder>),
    Mpeg12(Box<Mpeg12TimelineDecoder>),
    Flac(crate::owned_flac_decoder::FlacDecoder),
}
enum Mpeg12Core { One(fvid_codecs::mp1::Decoder), Two(fvid_codecs::mp2::Decoder) }
pub(crate) struct Mpeg12TimelineDecoder { core:Mpeg12Core, rate:u32, channels:u16 }
impl Mpeg12TimelineDecoder {
    fn new(track:&crate::owned_webm::Track)->Result<Self>{
        let rate=u32::try_from(track.sample_rate).map_err(|_|invalid("MPEG sample rate overflow"))?;
        let channels=u16::try_from(track.channels).map_err(|_|invalid("MPEG channel count overflow"))?;
        let core=if track.codec=="A_MPEG/L1" { Mpeg12Core::One(fvid_codecs::mp1::Decoder::new(rate,channels).map_err(|e|invalid(&e.to_string()))?) }
        else { Mpeg12Core::Two(fvid_codecs::mp2::Decoder::new(rate,channels).map_err(|e|invalid(&e.to_string()))?) };
        Ok(Self{core,rate,channels})
    }
    fn decode(&mut self,data:&[u8])->Result<Vec<f32>>{match &mut self.core {
        Mpeg12Core::One(d)=>d.decode(data).map_err(|e|invalid(&e.to_string())),
        Mpeg12Core::Two(d)=>d.decode(data).map_err(|e|invalid(&e.to_string()))
    }}
}
pub(crate) struct Mp3TimelineDecoder {
    core: fvid_codecs::mp3::Decoder,
    rate: u32,
    channels: u16,
    started: bool,
}
impl Mp3TimelineDecoder {
    fn new(track: &crate::owned_webm::Track) -> Result<Self> {
        let rate = u32::try_from(track.sample_rate).map_err(|_| invalid("MP3 sample rate overflow"))?;
        let channels = u16::try_from(track.channels).map_err(|_| invalid("MP3 channel count overflow"))?;
        fvid_codecs::mp3_spectral::Bands::for_rate(rate).map_err(|e| invalid(&e.to_string()))?;
        if !(1..=2).contains(&channels) { return Err(invalid("invalid MP3 channel count")); }
        Ok(Self { core: Default::default(), rate, channels, started: false })
    }
    fn decode(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        let header = fvid_codecs::mp3_frame::Header::parse(data).map_err(|e| invalid(&e.to_string()))?;
        if header.sample_rate != self.rate || header.channels != self.channels {
            return Err(invalid("MP3 frame geometry differs from Matroska track"));
        }
        if !self.started && fvid_codecs::mp3::is_info_frame(data).map_err(|e| invalid(&e.to_string()))? {
            self.core.prime_info(data).map_err(|e| invalid(&e.to_string()))?;
            self.started = true;
            // The container counts this priming packet in CodecDelay. Preserve
            // its timeline without advancing zero synthesis history.
            return Ok(vec![0.; header.samples() * usize::from(self.channels)]);
        }
        let pcm = self.core.decode(data).map_err(|e| invalid(&e.to_string()))?;
        self.started = true;
        Ok(pcm.samples)
    }
}
impl MatroskaTimelineDecoder {
    const SAMPLE_BYTES: usize = 4;
    pub(crate) fn with_in_band_ps(track: &crate::owned_webm::Track) -> Result<Self> {
        Ok(Self::Ps(Box::new(crate::owned_aac::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
            &track.codec_private, u32::try_from(track.sample_rate).map_err(|_| invalid("AAC output clock overflow"))?,
        )?)))
    }

    pub(crate) fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        match track.codec.as_str() {
            "A_FLAC" => Ok(Self::Flac(crate::owned_flac_decoder::FlacDecoder::from_matroska(track)?)),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_matroska(track)?,
            )),
            "A_MPEG/L1" | "A_MPEG/L2" => Ok(Self::Mpeg12(Box::new(Mpeg12TimelineDecoder::new(track)?))),
            "A_MPEG/L3" => Ok(Self::Mp3(Box::new(Mp3TimelineDecoder::new(track)?))),
            "A_OPUS" => {
                let decoder = crate::owned_opus::OpusDecoder::new(
                    &track.codec_private,
                    u32::try_from(track.sample_rate)
                        .map_err(|_| invalid("Opus sample rate overflow"))?,
                    u16::try_from(track.channels)
                        .map_err(|_| invalid("Opus channel count overflow"))?,
                )
                .map_err(|e| invalid(&e))?;
                Ok(Self::Opus(decoder))
            }
            "A_ALAC" => Ok(Self::Alac(crate::owned_alac::AlacDecoder::from_matroska(
                track,
            )?)),
            "A_AAC" => {
                let config =
                    crate::owned_aac::config::AudioSpecificConfig::parse(&track.codec_private)?;
                if config.ps_present == Some(true) {
                    Ok(Self::Ps(Box::new(
                        crate::owned_aac::aac_ps_native::NativePsAacDecoder::new(
                            &track.codec_private,
                        )?,
                    )))
                } else {
                    Ok(Self::Aac(
                        crate::owned_aac::NativeAacDecoder::new_with_output_rate(
                            &track.codec_private,
                            u32::try_from(track.sample_rate)
                                .map_err(|_| invalid("AAC output clock overflow"))?,
                        )?,
                    ))
                }
            }
            _ => Err(invalid(
                "selected Matroska audio codec is not owned by the export path",
            )),
        }
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Flac(d) => d.sample_rate(),
            Self::Pcm(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
            Self::Aac(d) => d.sample_rate(),
            Self::Ps(d) => d.sample_rate(),
            Self::Opus(d) => d.sample_rate(),
            Self::Mp3(d) => d.rate,
            Self::Mpeg12(d) => d.rate,
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Flac(d) => d.channels(),
            Self::Pcm(d) => d.channels(),
            Self::Alac(d) => d.channels(),
            Self::Aac(d) => u16::from(d.channels()),
            Self::Ps(d) => u16::from(d.channels()),
            Self::Opus(d) => d.channels(),
            Self::Mp3(d) => d.channels,
            Self::Mpeg12(d) => d.channels,
        }
    }
    pub(crate) fn aac_channel_mask(&self) -> Result<u32> {
        match self {
            Self::Aac(d) => Ok(d.channel_mask()),
            Self::Ps(d) => Ok(d.channel_mask()),
            _ => Err(invalid("channel mask requires AAC decoder")),
        }
    }
    fn delayed(&self) -> bool {
        matches!(self, Self::Ps(_)) || matches!(self,Self::Aac(d) if d.delayed())
    }
    fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<f32>>> {
        let samples = match self {
            Self::Ps(d) => return Ok(d.decode(data)?.map(|f| f.pcm)),
            Self::Mp3(d) => d.decode(data)?,
            Self::Mpeg12(d) => d.decode(data)?,
            Self::Flac(d) => d.decode(data)?,
            Self::Pcm(d) => d.decode_pcm(data)?,
            Self::Alac(d) => d.decode_pcm(data)?,
            Self::Aac(d) => return Ok(d.decode_timed(data,0,u64::from(d.core_frame_samples()))?.map(|f|f.samples)),
            Self::Opus(d) => d.decode(data).map_err(|e| invalid(&e))?,
        };
        Ok(Some(samples))
    }
    fn finish(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.finish()?.map(|f|f.samples)),
            _ => Ok(None),
        }
    }
}
pub(crate) struct DecodeProgress<'a> {
    pub(crate) options: &'a CopyOptions,
    pub(crate) event: ProgressEvent,
}
impl DecodeProgress<'_> {
    pub(crate) fn check(&self) -> Result<()> {
        if self
            .options
            .cancel
            .as_ref()
            .is_some_and(|c| c.is_cancelled())
        {
            return Err(invalid("media operation cancelled"));
        }
        crate::owned_budget::check_rss_budget(self.options).map_err(Error)
    }
    pub(crate) fn packet_limit(&self) -> Option<u64> {
        self.options.max_packets
    }
    pub(crate) fn packet_limit_reached(&self) -> bool {
        self.options
            .max_packets
            .is_some_and(|max| self.event.packets >= max)
    }
    pub(crate) fn packet(&mut self, bytes: usize) -> Result<()> {
        self.event.packets = self
            .event
            .packets
            .checked_add(1)
            .ok_or_else(|| invalid("audio packet count overflow"))?;
        self.event.payload_bytes = self
            .event
            .payload_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| invalid("audio byte count overflow"))?;
        if let Some(hook) = &self.options.progress {
            hook.emit(self.event);
        }
        self.check()
    }
}
pub(crate) fn matroska_audio_index<R: Read + Seek>(
    reader: &MatroskaTimelineReader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    let preferred=reader.preferred_track_numbers(&reader.tracks.iter().filter(|t|t.kind==2).map(|t|t.number).collect::<Vec<_>>());
    let indices: Vec<_> = reader
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| (t.kind == 2 && (selected.is_some() || preferred.contains(&t.number))).then_some(i))
        .collect();
    let index = match selected {
        Some(index) if indices.contains(&index) => index,
        Some(_) => return Err(invalid("selected stream is absent or is not audio")),
        None if indices.len() == 1 => indices[0],
        None => {
            return Err(invalid(
                "native audio export requires exactly one audio track or an explicit stream index",
            ));
        }
    };
    Ok(index)
}
pub(crate) fn open_aac_reader<R: Read + Seek>(
    source: R,
    options: &CopyOptions,
    packet_bytes: usize,
) -> Result<MatroskaTimelineReader<R>> {
    crate::owned_aac::stream::check_decode_admission(1, options)?;
    open_audio_reader(source, options, packet_bytes)
}
pub(crate) fn open_audio_reader<R: Read + Seek>(
    source: R,
    options: &CopyOptions,
    packet_bytes: usize,
) -> Result<MatroskaTimelineReader<R>> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let mut limits = crate::owned_webm::Limits::default();
    let memory_packets = options
        .max_controlled_bytes
        .map(|bytes| bytes / std::mem::size_of::<crate::owned_webm::Packet>());
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let mut reader = MatroskaTimelineReader::open(source, limits).map_err(|error| {
        if memory_packets.is_some_and(|max| max < crate::owned_webm::Limits::default().packets)
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            invalid("controlled memory budget exceeded: Matroska packet index limit")
        } else {
            error.into()
        }
    })?;
    reader.restrict_packet_bytes(packet_bytes);
    Ok(reader)
}
pub(crate) fn validate_audio_packet_budget<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    validate_audio_packet_budget_with_interval(reader, index, None, options)
}
pub(crate) fn validate_audio_packet_budget_with_interval<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<()> {
    if options.max_packet_bytes >= crate::owned_webm::Limits::default().packet_bytes {
        return Ok(());
    }
    reader.scan_all()?;
    let track = reader
        .tracks
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let track_number = track.number;
    let active_limit = matroska_active_track_packets(reader, index, interval, options.max_packets);
    let mut track_packets = 0usize;
    for (packet_index, packet) in reader.packets.iter().enumerate() {
        if packet.track == track_number {
            if track_packets >= active_limit {
                break;
            }
            if reader.packet_payload_size(packet_index)? > options.max_packet_bytes {
                return Err(invalid("WebM packet exceeds budget"));
            }
            track_packets += 1;
        }
    }
    Ok(())
}
pub(crate) fn admit_audio_reader<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    admit_audio_reader_with_interval(reader, index, None, options)
}
pub(crate) fn admit_audio_reader_with_interval<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<()> {
    if options.max_controlled_bytes.is_none() {
        return validate_audio_packet_budget_with_interval(reader, index, interval, options);
    }
    let track = reader
        .tracks
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let decoder = match track.codec.as_str() {
        "A_FLAC" => {
            crate::owned_flac_decoder::FlacDecoder::from_matroska(track)?;
            crate::owned_flac_decoder::FlacDecoder::DECODE_ADMISSION_BYTES
        }
        "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3" => fvid_codecs::mp3::Decoder::decode_admission_bytes(),
        "A_AAC" => crate::owned_aac::stream::decode_config_admission_bytes(
            &track.codec_private,
            u32::try_from(track.sample_rate).map_err(|_| invalid("AAC output clock overflow"))?,
        )?,
        "A_ALAC" => crate::owned_alac::AlacDecoder::decode_admission_bytes(
            &track.codec_private,
            u32::try_from(track.sample_rate)
                .map_err(|_| invalid("ALAC sample rate exceeds decoder geometry"))?,
            u16::try_from(track.channels)
                .map_err(|_| invalid("ALAC channel count exceeds decoder geometry"))?,
        )
        .map_err(|e| invalid(&e.to_string()))?,
        "A_OPUS" => crate::owned_opus::OpusDecoder::decode_admission_bytes(
            &track.codec_private,
            u32::try_from(track.sample_rate).map_err(|_| invalid("Opus sample rate overflow"))?,
            u16::try_from(track.channels).map_err(|_| invalid("Opus channel count overflow"))?,
        )
        .map_err(|e| invalid(&e))?,
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
            // Geometry validation allocates no heap. Widened samples are charged
            // against each packet below, using f64 even for the f32 API.
            crate::owned_pcm_decoder::PcmDecoder::from_matroska(track)?;
            16 * 1024
        }
        _ => {
            return Err(invalid(
                "Matroska audio aggregate allocation admission is not yet implemented",
            ));
        }
    };
    let pcm = matches!(
        track.codec.as_str(),
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
    );
    let opus = track.codec == "A_OPUS";
    let mut visited_packets = 0;
    let mut track_packets = 0u64;
    let mut largest_packet = 0;
    let mut largest_read_overhead = 0;
    reader
        .scan_all_with_admission(|reader| {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(crate::owned_ebml::Error("media operation cancelled".into()).into());
            }
            crate::owned_budget::check_rss_budget(options).map_err(crate::owned_ebml::Error)?;
            let track = reader.tracks.get(index).ok_or_else(|| {
                crate::owned_ebml::Error("selected audio stream is absent".into())
            })?;
            let active_limit = matroska_active_track_packets(reader, index, interval, options.max_packets);
            for (packet_index, packet) in reader.packets.iter().enumerate().skip(visited_packets) {
                if packet.track == track.number {
                    if track_packets as usize >= active_limit {
                        continue;
                    }
                    let payload_size = reader.packet_payload_size(packet_index)?;
                    if payload_size > options.max_packet_bytes {
                        return Err(crate::owned_ebml::Error(
                            "WebM packet exceeds budget".into(),
                        )
                        .into());
                    }
                    largest_packet = largest_packet.max(payload_size);
                    largest_read_overhead = largest_read_overhead.max(reader.packet_read_overhead(packet_index)?);
                    track_packets += 1;
                }
            }
            visited_packets = reader.packets.len();
            // One encoded packet plus at most eight bytes per input byte for
            // f64 conversion of unsigned 8-bit PCM; wider formats cost less.
            // Opus multistream also retains a rebuilt elementary packet, with
            // up to twice its length reserved by Vec growth.
            let packet = largest_packet
                .checked_mul(if pcm {
                    9
                } else if opus {
                    3
                } else {
                    1
                })
                .and_then(|bytes| bytes.checked_add(largest_read_overhead))
                .ok_or_else(|| {
                    crate::owned_ebml::Error("Matroska memory estimate overflow".into())
                })?;
            let cloned_track = track
                .codec
                .len()
                .checked_add(track.name.len())
                .and_then(|bytes| bytes.checked_add(track.language.len()))
                .and_then(|bytes| bytes.checked_add(track.codec_private.len()))
                .ok_or_else(|| {
                    crate::owned_ebml::Error("Matroska memory estimate overflow".into())
                })?;
            let estimated = reader
                .estimated_index_payload_bytes()?
                .checked_add(decoder)
                .and_then(|bytes| bytes.checked_add(packet))
                .and_then(|bytes| bytes.checked_add(cloned_track))
                .ok_or_else(|| {
                    crate::owned_ebml::Error("Matroska memory estimate overflow".into())
                })?;
            if options
                .max_controlled_bytes
                .is_some_and(|max| estimated > max)
            {
                return Err(crate::owned_ebml::Error(format!(
                    "controlled memory budget exceeded: need {estimated} bytes"
                ))
                .into());
            }
            Ok(())
        })
        .map_err(|error| {
            if options.max_controlled_bytes.is_some_and(|max| {
                max / std::mem::size_of::<crate::owned_webm::Packet>()
                    < crate::owned_webm::Limits::default().packets
            }) && error
                .to_string()
                .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error.into()
            }
        })?;
    Ok(())
}
/// Decode the presentation timeline, including delay, signed padding, gaps and
/// ceil-rounded interval boundaries. An error may leave partial caller-owned PCM.
/// Progress never reports publication/completion. Controlled admission is checked
/// before decoding; metadata mutation remains unsupported by this raw-stream API.
pub(crate) fn decode_matroska_audio_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    codec: &str,
) -> Result<AudioDecodeStats> {
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("raw PCM stream cannot apply metadata mutations"));
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err(invalid("select exactly one audio stream")),
    };
    let mut control = DecodeProgress {
        options,
        event: ProgressEvent {
            packets: 0,
            payload_bytes: 0,
            done: false,
        },
    };
    control.check()?;
    if let Some(hook) = &options.progress {
        hook.emit(control.event);
    }
    control.check()?;
    let mut reader = if codec == "A_AAC" {
        open_aac_reader(source, options, options.max_packet_bytes)?
    } else {
        open_audio_reader(source, options, options.max_packet_bytes)?
    };
    let index = matroska_audio_index(&reader, selected)?;
    if if codec == "PCM" {
        !matches!(
            reader.tracks[index].codec.as_str(),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
        )
    } else if codec == "MPEG" {
        !matches!(reader.tracks[index].codec.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3")
    } else {
        reader.tracks[index].codec != codec
    } {
        return Err(invalid(
            "selected Matroska audio stream has a different codec",
        ));
    }
    if matches!(codec, "A_AAC" | "A_FLAC" | "A_ALAC" | "A_OPUS" | "A_MPEG/L3" | "MPEG" | "PCM") {
        admit_audio_reader_with_interval(&mut reader, index, interval, options)?;
    }
    decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
}
include!("owned_matroska_audio_timeline_impl.rs");

impl From<crate::owned_webm::Error> for Error {
    fn from(error: crate::owned_webm::Error) -> Self {
        Self(error.to_string())
    }
}
