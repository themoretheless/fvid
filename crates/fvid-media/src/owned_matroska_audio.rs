//! Owned Matroska AAC/ALAC/PCM/Opus/FLAC/MPEG presentation timeline to caller-owned PCM.
use crate::owned_aac::aac_ps_native::InBandPsProbe;
pub use crate::owned_aac::stream::AudioDecodeStats;
use crate::owned_audio_control::{AudioDecodeControl, CopyProgress};
use crate::owned_matroska_grid_clock::grid_sample_matches as aac_quantized_grid_matches;
use crate::owned_matroska_sample_clock::quantized_sample_matches as aac_quantized_sample_matches;
use crate::owned_webm::WebmReader as MatroskaTimelineReader;
pub(crate) use fvid_control::error::invalid;
pub use fvid_control::error::{Error, Result};
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};

pub trait MatroskaSampleDecoder: Sized {
    type Sample: Copy;
    const SAMPLE_BYTES: usize;
    fn push_le_bytes(sample: Self::Sample, out: &mut Vec<u8>);
    fn with_in_band_ps(track: &crate::owned_webm::Track) -> Result<Self>;
    fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self>;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    fn delayed(&self) -> bool;
    fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<Self::Sample>>>;
    fn finish(&mut self) -> Result<Option<Vec<Self::Sample>>>;
}

pub enum MatroskaTimelineDecoder {
    Pcm(crate::owned_pcm_decoder::PcmDecoder),
    Alac(crate::owned_alac::AlacDecoder),
    Aac(crate::owned_aac::NativeAacDecoder),
    Ps(Box<crate::owned_aac::aac_ps_native::NativePsAacDecoder>),
    Opus(crate::owned_opus::OpusDecoder),
    Mp3(Box<Mp3TimelineDecoder>),
    Mpeg12(Box<Mpeg12TimelineDecoder>),
    Flac(crate::owned_flac_decoder::FlacDecoder),
}

enum Mpeg12Core {
    One(fvid_codecs::mp1::Decoder),
    Two(fvid_codecs::mp2::Decoder),
}

pub struct Mpeg12TimelineDecoder {
    core: Mpeg12Core,
    rate: u32,
    channels: u16,
}

impl Mpeg12TimelineDecoder {
    fn new(track: &crate::owned_webm::Track) -> Result<Self> {
        let rate =
            u32::try_from(track.sample_rate).map_err(|_| invalid("MPEG sample rate overflow"))?;
        let channels =
            u16::try_from(track.channels).map_err(|_| invalid("MPEG channel count overflow"))?;
        let core = if track.codec == "A_MPEG/L1" {
            Mpeg12Core::One(
                fvid_codecs::mp1::Decoder::new(rate, channels)
                    .map_err(|e| invalid(&e.to_string()))?,
            )
        } else {
            Mpeg12Core::Two(
                fvid_codecs::mp2::Decoder::new(rate, channels)
                    .map_err(|e| invalid(&e.to_string()))?,
            )
        };
        Ok(Self {
            core,
            rate,
            channels,
        })
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        match &mut self.core {
            Mpeg12Core::One(d) => d.decode(data).map_err(|e| invalid(&e.to_string())),
            Mpeg12Core::Two(d) => d.decode(data).map_err(|e| invalid(&e.to_string())),
        }
    }
}

pub struct Mp3TimelineDecoder {
    core: fvid_codecs::mp3::Decoder,
    rate: u32,
    channels: u16,
    started: bool,
}

impl Mp3TimelineDecoder {
    fn new(track: &crate::owned_webm::Track) -> Result<Self> {
        let rate =
            u32::try_from(track.sample_rate).map_err(|_| invalid("MP3 sample rate overflow"))?;
        let channels =
            u16::try_from(track.channels).map_err(|_| invalid("MP3 channel count overflow"))?;
        fvid_codecs::mp3_spectral::Bands::for_rate(rate).map_err(|e| invalid(&e.to_string()))?;
        if !(1..=2).contains(&channels) {
            return Err(invalid("invalid MP3 channel count"));
        }
        Ok(Self {
            core: Default::default(),
            rate,
            channels,
            started: false,
        })
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        let header =
            fvid_codecs::mp3_frame::Header::parse(data).map_err(|e| invalid(&e.to_string()))?;
        if header.sample_rate != self.rate || header.channels != self.channels {
            return Err(invalid("MP3 frame geometry differs from Matroska track"));
        }
        if !self.started
            && fvid_codecs::mp3::is_info_frame(data).map_err(|e| invalid(&e.to_string()))?
        {
            self.core
                .prime_info(data)
                .map_err(|e| invalid(&e.to_string()))?;
            self.started = true;
            // The container counts this priming packet in CodecDelay. Preserve
            // its timeline without advancing zero synthesis history.
            return Ok(vec![0.; header.samples() * usize::from(self.channels)]);
        }
        let pcm = self
            .core
            .decode(data)
            .map_err(|e| invalid(&e.to_string()))?;
        self.started = true;
        Ok(pcm.samples)
    }
}

impl MatroskaTimelineDecoder {
    pub fn with_in_band_ps(track: &crate::owned_webm::Track) -> Result<Self> {
        Ok(Self::Ps(Box::new(
            crate::owned_aac::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
                &track.codec_private,
                u32::try_from(track.sample_rate)
                    .map_err(|_| invalid("AAC output clock overflow"))?,
            )?,
        )))
    }

    pub fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        match track.codec.as_str() {
            "A_FLAC" => Ok(Self::Flac(
                crate::owned_flac_decoder::FlacDecoder::from_matroska(track)?,
            )),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_matroska(track)?,
            )),
            "A_MPEG/L1" | "A_MPEG/L2" => Ok(Self::Mpeg12(Box::new(Mpeg12TimelineDecoder::new(
                track,
            )?))),
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

    pub fn sample_rate(&self) -> u32 {
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

    pub fn channels(&self) -> u16 {
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

    pub fn channel_mask(&self) -> Option<u32> {
        match self {
            Self::Aac(d) => Some(d.channel_mask()),
            Self::Ps(d) => Some(d.channel_mask()),
            Self::Alac(d) => Some(d.channel_mask()),
            _ => None,
        }
    }

    pub(crate) fn aac_channel_mask(&self) -> Result<u32> {
        match self {
            Self::Aac(d) => Ok(d.channel_mask()),
            Self::Ps(d) => Ok(d.channel_mask()),
            _ => Err(invalid("channel mask requires AAC decoder")),
        }
    }

    pub fn delayed(&self) -> bool {
        matches!(self, Self::Ps(_)) || matches!(self, Self::Aac(d) if d.delayed())
    }

    pub fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<f32>>> {
        let samples = match self {
            Self::Ps(d) => return Ok(d.decode(data)?.map(|f| f.pcm)),
            Self::Mp3(d) => d.decode(data)?,
            Self::Mpeg12(d) => d.decode(data)?,
            Self::Flac(d) => d.decode(data)?,
            Self::Pcm(d) => d.decode_pcm(data)?,
            Self::Alac(d) => d.decode_pcm(data)?,
            Self::Aac(d) => {
                return Ok(d
                    .decode_timed(data, 0, u64::from(d.core_frame_samples()))?
                    .map(|f| f.samples));
            }
            Self::Opus(d) => d.decode(data).map_err(|e| invalid(&e))?,
        };
        Ok(Some(samples))
    }

    pub fn finish(&mut self) -> Result<Option<Vec<f32>>> {
        match self {
            Self::Ps(d) => Ok(d.finish()?.map(|f| f.pcm)),
            Self::Aac(d) => Ok(d.finish()?.map(|f| f.samples)),
            _ => Ok(None),
        }
    }
}

impl MatroskaSampleDecoder for MatroskaTimelineDecoder {
    type Sample = f32;
    const SAMPLE_BYTES: usize = 4;

    fn push_le_bytes(sample: f32, out: &mut Vec<u8>) {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    fn with_in_band_ps(track: &crate::owned_webm::Track) -> Result<Self> {
        Self::with_in_band_ps(track)
    }
    fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        Self::from_matroska(track)
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.channels()
    }
    fn delayed(&self) -> bool {
        self.delayed()
    }
    fn decode(&mut self, data: &[u8]) -> Result<Option<Vec<f32>>> {
        self.decode(data)
    }
    fn finish(&mut self) -> Result<Option<Vec<f32>>> {
        self.finish()
    }
}


pub fn matroska_audio_index<R: Read + Seek>(
    reader: &MatroskaTimelineReader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    let preferred = reader.preferred_track_numbers(
        &reader
            .tracks
            .iter()
            .filter(|t| t.kind == 2)
            .map(|t| t.number)
            .collect::<Vec<_>>(),
    );
    let indices: Vec<_> = reader
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            (t.kind == 2 && (selected.is_some() || preferred.contains(&t.number))).then_some(i)
        })
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
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
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
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let mut limits = crate::owned_webm::Limits::default();
    let memory_packets = options
        .max_controlled_bytes
        .map(|bytes| bytes / std::mem::size_of::<crate::owned_webm::Packet>());
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let reader = MatroskaTimelineReader::open(source, limits).map_err(|error| {
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
    let _ = packet_bytes;
    Ok(reader)
}


pub(crate) fn validate_audio_packet_budget_with_interval<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
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
            if options
                .cancel
                .as_ref()
                .is_some_and(fvid_control::CancelFlag::is_cancelled)
            {
                return Err(invalid("media operation cancelled"));
            }
            crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
            if reader.packet_payload_size(packet_index)? > options.max_packet_bytes {
                return Err(invalid("WebM packet exceeds budget"));
            }
            track_packets += 1;
        }
    }
    reader.restrict_packet_bytes(options.max_packet_bytes);
    Ok(())
}


pub(crate) fn admit_audio_reader_with_interval<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
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
                return Err(invalid("media operation cancelled"));
            }
            crate::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
            let track = reader
                .tracks
                .get(index)
                .ok_or_else(|| invalid("selected audio stream is absent"))?;
            let active_limit =
                matroska_active_track_packets(reader, index, interval, options.max_packets);
            for (packet_index, packet) in reader.packets.iter().enumerate().skip(visited_packets) {
                if packet.track == track.number {
                    if track_packets as usize >= active_limit {
                        continue;
                    }
                    let payload_size = reader.packet_payload_size(packet_index)?;
                    if payload_size > options.max_packet_bytes {
                        return Err(invalid("WebM packet exceeds budget"));
                    }
                    largest_packet = largest_packet.max(payload_size);
                    largest_read_overhead =
                        largest_read_overhead.max(reader.packet_read_overhead(packet_index)?);
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
                .ok_or_else(|| invalid("Matroska memory estimate overflow"))?;
            let cloned_track = track
                .codec
                .len()
                .checked_add(track.name.len())
                .and_then(|bytes| bytes.checked_add(track.language.len()))
                .and_then(|bytes| bytes.checked_add(track.codec_private.len()))
                .ok_or_else(|| invalid("Matroska memory estimate overflow"))?;
            let estimated = reader
                .estimated_index_payload_bytes()?
                .checked_add(decoder)
                .and_then(|bytes| bytes.checked_add(packet))
                .and_then(|bytes| bytes.checked_add(cloned_track))
                .ok_or_else(|| invalid("Matroska memory estimate overflow"))?;
            let wave_meta_bytes =
                if !options.metadata_set.is_empty() || !options.metadata_delete.is_empty() {
                    crate::owned_wave_metadata::wave_metadata_working_bytes(0, options)
                        .map_err(|e| invalid(&e))?
                } else {
                    0
                };
            let estimated = estimated
                .checked_add(wave_meta_bytes)
                .ok_or_else(|| invalid("Matroska memory estimate overflow"))?;
            if options
                .max_controlled_bytes
                .is_some_and(|max| estimated > max)
            {
                return Err(invalid(&format!(
                    "controlled memory budget exceeded: need {estimated} bytes"
                )));
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
    reader.restrict_packet_bytes(options.max_packet_bytes);
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
    let mut control = CopyProgress::new(options);
    control.check()?;
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
        !matches!(
            reader.tracks[index].codec.as_str(),
            "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3"
        )
    } else {
        reader.tracks[index].codec != codec
    } {
        return Err(invalid(
            "selected Matroska audio stream has a different codec",
        ));
    }
    if matches!(
        codec,
        "A_AAC" | "A_FLAC" | "A_ALAC" | "A_OPUS" | "A_MPEG/L3" | "MPEG" | "PCM"
    ) {
        admit_audio_reader_with_interval(&mut reader, index, interval, options)?;
    }
    decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
}

pub(crate) fn decode_matroska_audio_reader_pcm<R: Read + Seek>(
    reader: MatroskaTimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let mut control = CopyProgress::new(options);
    control.check()?;
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
    if let Some(hook) = &options.progress {
        hook.emit(control.event);
    }
    control.check()?;
    decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
}

pub fn decode_matroska_audio_reader_controlled<R: Read + Seek, C: AudioDecodeControl>(
    reader: MatroskaTimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut C,
) -> Result<AudioDecodeStats> {
    decode_matroska_audio_reader_generic::<R, C, MatroskaTimelineDecoder>(
        reader, output, interval, selected, control,
    )
}

pub fn decode_matroska_audio_reader_generic<
    R: Read + Seek,
    C: AudioDecodeControl,
    D: MatroskaSampleDecoder,
>(
    mut reader: MatroskaTimelineReader<R>,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    selected: Option<usize>,
    control: &mut C,
) -> Result<AudioDecodeStats> {
    control.check()?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("audio interval requires from < to"));
    }
    reader.scan_all()?;
    let index = matroska_audio_index(&reader, selected)?;
    let track = reader.tracks[index].clone();
    let max_packets = control.packet_limit();
    let in_band_ps = negotiate_matroska_ps_with_interval(
        &mut reader,
        index,
        interval,
        max_packets,
        || control.check(),
    )?;
    let mut decoder = if in_band_ps {
        D::with_in_band_ps(&track)?
    } else {
        D::from_matroska(&track)?
    };
    let rate = decoder.sample_rate();
    let channels = decoder.channels();
    let exact_clock = matches!(
        track.codec.as_str(),
        "A_AAC"
            | "A_PCM/INT/LIT"
            | "A_PCM/INT/BIG"
            | "A_PCM/FLOAT/IEEE"
            | "A_MPEG/L1"
            | "A_MPEG/L2"
            | "A_MPEG/L3"
            | "A_FLAC"
            | "A_ALAC"
    );
    if track.sample_rate != u64::from(rate)
        || (!in_band_ps && track.channels != u64::from(channels))
    {
        return Err(invalid(
            "Matroska audio geometry disagrees with configuration",
        ));
    }
    let boundary = |ns: u128| -> Result<u64> {
        let samples = ns
            .checked_mul(u128::from(rate))
            .ok_or_else(|| invalid("audio time overflow"))?
            .div_ceil(1_000_000_000);
        u64::try_from(samples).map_err(|_| invalid("audio sample position overflow"))
    };
    let (from, to) = match interval {
        Some((from, to)) => (boundary(from.as_nanos())?, boundary(to.as_nanos())?),
        None => (0, u64::MAX),
    };
    // Muxers express integral sample counts in rounded nanoseconds. Restore
    // the nearest sample for this metadata; user interval boundaries still ceil.
    let trim_samples = |ns: u64| -> Result<u64> {
        u64::try_from((u128::from(ns) * u128::from(rate) + 500_000_000) / 1_000_000_000)
            .map_err(|_| invalid("audio trim length overflow"))
    };
    let total_delay = trim_samples(track.codec_delay_ns)?;
    let mut delay = total_delay;
    let mut leading_padding = 0u64;
    let mut position = 0u64;
    let mut origin = None;
    // Coded audio timestamp quantization applies to the full coded window, before
    // signed padding. Audible position must not be used as that clock.
    let mut sample_epoch: Option<i64> = None;
    let mut coded_samples = 0u64;
    let mut sample_epoch_grid = None;
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut pending_packet = std::collections::VecDeque::new();
    let mut pcm_scratch = Vec::new();
    pcm_scratch
        .try_reserve_exact(4096 * D::SAMPLE_BYTES)
        .map_err(|_| invalid("Matroska audio PCM output buffer allocation failed"))?;
    let mut index = 0;
    loop {
        control.check()?;
        if position >= to {
            break;
        }
        let terminal = index == reader.packets.len() || control.packet_limit_reached();
        let (packet_index, packet, samples) = if terminal {
            let Some(samples) = decoder.finish()? else {
                break;
            };
            let (source_index, packet) = pending_packet
                .pop_front()
                .ok_or_else(|| invalid("delayed Matroska audio has no source packet"))?;
            (source_index, packet, samples)
        } else {
            if reader.packets[index].track != track.number {
                index += 1;
                continue;
            }
            let packet = reader.packets[index].clone();
            let source_index = index;
            let encoded = reader.read_packet(index)?;
            let samples = decoder.decode(&encoded)?;
            control.packet(encoded.len())?;
            index += 1;
            if decoder.delayed() {
                pending_packet.push_back((source_index, packet));
                if pending_packet.len() > 3 {
                    return Err(invalid("delayed Matroska source lookahead exceeded"));
                }
                let Some(samples) = samples else {
                    continue;
                };
                let packet = pending_packet
                    .pop_front()
                    .ok_or_else(|| invalid("delayed Matroska audio source identity mismatch"))?;
                (packet.0, packet.1, samples)
            } else {
                (
                    source_index,
                    packet,
                    samples.ok_or_else(|| invalid("immediate Matroska audio returned no PCM"))?,
                )
            }
        };
        if !samples.len().is_multiple_of(usize::from(channels)) {
            return Err(invalid("invalid Matroska decoded PCM stride"));
        }
        let frames = (samples.len() / usize::from(channels)) as u64;
        let timestamp = if exact_clock {
            let unit = 1_000_000_000i128;
            let grid = reader.packet_timestamp_grid(packet_index);
            let matches = if let (Some(current), Some(epoch)) = (grid, sample_epoch_grid) {
                aac_quantized_grid_matches(
                    current,
                    epoch,
                    coded_samples,
                    rate,
                    reader.timestamp_scale_ns(),
                    reader.track_timestamp_scale(track.number).unwrap_or(1.0),
                )
            } else {
                sample_epoch.is_some_and(|epoch| {
                    aac_quantized_sample_matches(
                        packet.pts_ns,
                        epoch,
                        coded_samples,
                        rate,
                        reader.timestamp_scale_ns(),
                    )
                })
            };
            if !matches {
                sample_epoch = Some(packet.pts_ns);
                sample_epoch_grid = grid;
                coded_samples = 0;
            }
            let rate = i128::from(rate);
            let projected =
                i128::from(sample_epoch.unwrap()) * rate + i128::from(coded_samples) * unit;
            coded_samples = coded_samples
                .checked_add(frames)
                .ok_or_else(|| invalid("audio coded sample clock overflow"))?;
            (projected + unit / 2).div_euclid(unit) - i128::from(total_delay)
        } else {
            let base = *origin.get_or_insert(packet.pts_ns);
            ((i128::from(packet.pts_ns) - i128::from(base)) * i128::from(rate) + 500_000_000)
                .div_euclid(1_000_000_000)
                - i128::from(total_delay)
        };
        // Once coded audio timing has been resolved, explicit trims and gaps
        // are exact sample windows, even when smaller than a container tick.
        let precision = if exact_clock {
            0
        } else {
            (u128::from(reader.timestamp_scale_ns()) * u128::from(rate) / 1_000_000_000) as u64
        };
        let padding = trim_samples(packet.discard_padding_ns.unsigned_abs())?;
        if padding > frames {
            return Err(invalid("Matroska discard padding exceeds audio packet"));
        }
        let head = if packet.discard_padding_ns < 0 {
            padding
        } else {
            0
        };
        let tail = if packet.discard_padding_ns > 0 {
            padding
        } else {
            0
        };
        let skip_delay = if exact_clock {
            timestamp.saturating_neg().max(0).min(i128::from(frames)) as u64
        } else {
            let skipped = delay.min(frames);
            delay -= skipped;
            skipped
        };
        let first = head.max(skip_delay);
        if !exact_clock && position == 0 && stats.sample_frames == 0 {
            // Leading padding can consume several complete laced packets.
            // Accumulate its origin until audible output starts, rather than
            // reinserting the remaining discarded prefix as timestamp silence.
            leading_padding = leading_padding
                .checked_add(head.saturating_sub(skip_delay))
                .ok_or_else(|| invalid("Matroska leading padding overflow"))?;
        }
        let end = frames - tail;
        if first > end {
            return Err(invalid("Matroska audio trimming overlaps"));
        }
        let available = end - first;
        stats.decoded_frames += 1;
        if available == 0 {
            continue;
        }
        let audible_start = timestamp + i128::from(first) - i128::from(leading_padding);
        let audible_start =
            if audible_start < 0 && audible_start.unsigned_abs() <= u128::from(precision) {
                0
            } else {
                u64::try_from(audible_start)
                    .map_err(|_| invalid("negative presented Matroska audio timestamp"))?
            };
        if audible_start < position && position - audible_start > precision {
            return Err(invalid("overlapping presented Matroska audio intervals"));
        }
        if audible_start > position && audible_start - position > precision {
            // A gap is silence in the presentation timeline, not a request to
            // concatenate subsequent sample bytes earlier than their timestamp.
            let gap_begin = from.max(position);
            let gap_end = to.min(audible_start);
            if gap_end > gap_begin {
                let mut frames = gap_end - gap_begin;
                let zero = [0u8; 4096];
                let frame_bytes = usize::from(channels) * D::SAMPLE_BYTES;
                let chunk_frames = zero.len() / frame_bytes;
                while frames > 0 {
                    control.check()?;
                    let count = frames.min(chunk_frames as u64) as usize;
                    output.write_all(&zero[..count * frame_bytes])?;
                    frames -= count as u64;
                    stats.sample_frames += count as u64;
                }
            }
            position = audible_start;
        }
        let begin = from.saturating_sub(position).min(available);
        let end = to.saturating_sub(position).min(available).max(begin);
        let width = usize::from(channels);
        for chunk in
            samples[(first + begin) as usize * width..(first + end) as usize * width].chunks(4096)
        {
            pcm_scratch.clear();
            for &sample in chunk {
                D::push_le_bytes(sample, &mut pcm_scratch);
            }
            output.write_all(&pcm_scratch)?;
        }
        stats.sample_frames += end - begin;
        position = position
            .checked_add(available)
            .ok_or_else(|| invalid("audio timeline overflow"))?;
    }
    if stats.sample_frames == 0 {
        return Err(invalid("Matroska audio interval contains no samples"));
    }
    Ok(stats)
}

pub fn matroska_active_track_packets<R: Read + Seek>(
    reader: &MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
) -> usize {
    let limit = max_packets
        .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX);
    let Some(track) = reader.tracks.get(index) else {
        return 0;
    };
    let track_number = track.number;
    let total_track_packets = reader
        .packets
        .iter()
        .filter(|p| p.track == track_number)
        .count();
    let Some((_, end)) = interval else {
        return total_track_packets.min(limit);
    };
    if reader.zlib_tracks_contains(track_number)
        || reader
            .track_timestamp_scale(track_number)
            .is_some_and(|scale| scale != 1.0)
        || reader
            .packets
            .iter()
            .any(|p| p.track == track_number && p.discard_padding_ns != 0)
    {
        return total_track_packets.min(limit);
    }
    let Some(first_pts_ns) = reader
        .packets
        .iter()
        .find(|p| p.track == track_number)
        .map(|p| p.pts_ns)
    else {
        return 0;
    };
    if first_pts_ns < 0 {
        return total_track_packets.min(limit);
    }
    let mut prev_pts = first_pts_ns;
    for packet in reader.packets.iter().filter(|p| p.track == track_number) {
        if packet.pts_ns < prev_pts {
            return total_track_packets.min(limit);
        }
        prev_pts = packet.pts_ns;
    }
    let end_ns = i128::try_from(end.as_nanos()).unwrap_or(i128::MAX);
    let delay_ns = i128::from(track.codec_delay_ns);
    let scale_ns = i128::from(reader.timestamp_scale_ns().max(1));
    let cutoff_pts_ns = end_ns
        .saturating_add(i128::from(first_pts_ns))
        .saturating_add(delay_ns)
        .saturating_add(scale_ns);
    let extra_lookahead = if track.codec == "A_AAC" { 2usize } else { 0usize };
    let mut count = 0usize;
    for packet in reader.packets.iter().filter(|p| p.track == track_number) {
        if i128::from(packet.pts_ns) < cutoff_pts_ns {
            count += 1;
        } else {
            break;
        }
    }
    count
        .saturating_add(extra_lookahead)
        .min(total_track_packets)
        .min(limit)
}

pub fn matroska_estimated_frames<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    rate: u32,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
) -> u64 {
    let _ = reader.scan_all();
    let Some(track) = reader.tracks.get(index) else {
        return 0;
    };
    let track_number = track.number;
    let active_limit = matroska_active_track_packets(reader, index, interval, max_packets);
    if active_limit == 0 {
        return 0;
    }
    let codec = track.codec.as_str();
    let channels = (track.channels as usize).max(1);
    let bit_depth_bytes = ((track.bit_depth as usize).max(8) / 8).max(1);
    let pcm_block = channels.saturating_mul(bit_depth_bytes).max(1);
    let is_pcm = matches!(
        codec,
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
    );
    let default_pkt_frames = if track.default_duration_ns > 0 {
        (u128::from(track.default_duration_ns) * u128::from(rate)).div_ceil(1_000_000_000) as u64
    } else {
        match codec {
            "A_AAC" => 1024,
            "A_MPEG/L1" => 384,
            "A_MPEG/L2" | "A_MPEG/L3" => 1152,
            "A_OPUS" => (u64::from(rate) / 50).max(1),
            "A_FLAC" | "A_ALAC" => 4096,
            _ => 1024,
        }
    };
    let mut first_pts: Option<i64> = None;
    let mut last_pts: i64 = 0;
    let mut last_pkt_frames: u64 = default_pkt_frames;
    let mut sum_frames: u64 = 0;
    let mut count = 0usize;
    for packet in reader.packets.iter().filter(|p| p.track == track_number) {
        if count >= active_limit {
            break;
        }
        if first_pts.is_none() {
            first_pts = Some(packet.pts_ns);
        }
        last_pts = packet.pts_ns;
        let pkt_frames = if is_pcm {
            (packet.size / pcm_block) as u64
        } else if let Some(dur_ns) = packet.duration_ns.filter(|&d| d > 0) {
            (u128::from(dur_ns) * u128::from(rate)).div_ceil(1_000_000_000) as u64
        } else {
            default_pkt_frames
        };
        last_pkt_frames = pkt_frames;
        sum_frames = sum_frames.saturating_add(pkt_frames);
        count += 1;
    }
    let span_ns = first_pts
        .map(|first| {
            i128::from(last_pts)
                .saturating_sub(i128::from(first))
                .max(0) as u128
        })
        .unwrap_or(0);
    let span_frames = (span_ns * u128::from(rate)).div_ceil(1_000_000_000) as u64;
    let delay_frames = (u128::from(track.codec_delay_ns) * u128::from(rate)) / 1_000_000_000;
    let mut frames = if is_pcm {
        sum_frames.max(span_frames.saturating_add(last_pkt_frames))
    } else if count > 1 && span_frames > 0 {
        let step = (span_frames / (count as u64 - 1)).max(last_pkt_frames);
        span_frames.saturating_add(step)
    } else {
        sum_frames
    };
    frames = frames.saturating_sub(delay_frames as u64);
    if let Some((_, to)) = interval {
        let limit = (to.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as u64;
        frames = frames.min(limit);
    }
    frames.max(1)
}

pub fn negotiate_matroska_ps<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    max_packets: Option<u64>,
    check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    negotiate_matroska_ps_with_interval(reader, index, None, max_packets, check)
}

pub fn negotiate_matroska_ps_with_interval<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    interval: Option<(Duration, Duration)>,
    max_packets: Option<u64>,
    mut check: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let track = reader.tracks[index].clone();
    if track.codec != "A_AAC" {
        return Ok(false);
    }
    let Ok(rate) = u32::try_from(track.sample_rate) else {
        return Ok(false);
    };
    let Ok(mut probe) = InBandPsProbe::new(&track.codec_private, rate) else {
        return Ok(false);
    };
    reader.scan_all()?;
    let active_limit = matroska_active_track_packets(reader, index, interval, max_packets);
    let mut track_packets = 0usize;
    for packet_index in 0..reader.packets.len() {
        check()?;
        if reader.packets[packet_index].track != track.number {
            continue;
        }
        if track_packets >= active_limit {
            break;
        }
        track_packets += 1;
        let packet = reader.read_packet(packet_index)?;
        if probe.read(&packet).is_err() {
            probe.reset();
            continue;
        }
        if probe.ps_detected() {
            return Ok(true);
        }
    }
    Ok(false)
}
