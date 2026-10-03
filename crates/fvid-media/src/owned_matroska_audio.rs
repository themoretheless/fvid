//! Owned Matroska AAC/ALAC/PCM presentation timeline to caller-owned float32 PCM.
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
enum MatroskaTimelineDecoder {
    Pcm(crate::owned_pcm_decoder::PcmDecoder),
    Alac(crate::owned_alac::AlacDecoder),
    Aac(crate::owned_aac::NativeAacDecoder),
}
impl MatroskaTimelineDecoder {
    const SAMPLE_BYTES: usize = 4;
    fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        match track.codec.as_str() {
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_matroska(track)?,
            )),
            "A_ALAC" => Ok(Self::Alac(crate::owned_alac::AlacDecoder::from_matroska(
                track,
            )?)),
            "A_AAC" => Ok(Self::Aac(crate::owned_aac::NativeAacDecoder::new(
                &track.codec_private,
            )?)),
            _ => Err(invalid(
                "selected Matroska audio codec is not owned by the export path",
            )),
        }
    }
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Pcm(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
            Self::Aac(d) => d.sample_rate(),
        }
    }
    fn channels(&self) -> u16 {
        match self {
            Self::Pcm(d) => d.channels(),
            Self::Alac(d) => d.channels(),
            Self::Aac(d) => u16::from(d.channels()),
        }
    }
    fn decode(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Pcm(d) => Ok(d.decode_pcm(data)?),
            Self::Alac(d) => Ok(d.decode_pcm(data)?),
            Self::Aac(d) => Ok(d.decode(data)?),
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
    let indices: Vec<_> = reader
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| (t.kind == 2).then_some(i))
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
    let mut limits = crate::owned_webm::Limits {
        packet_bytes,
        ..Default::default()
    };
    let memory_packets = options
        .max_controlled_bytes
        .map(|bytes| bytes / std::mem::size_of::<crate::owned_webm::Packet>());
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    MatroskaTimelineReader::open(source, limits).map_err(|error| {
        if memory_packets.is_some_and(|max| max < crate::owned_webm::Limits::default().packets)
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            invalid("controlled memory budget exceeded: Matroska packet index limit")
        } else {
            error.into()
        }
    })
}
pub(crate) fn admit_audio_reader<R: Read + Seek>(
    reader: &mut MatroskaTimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    if options.max_controlled_bytes.is_none() {
        return Ok(());
    }
    let track = reader
        .tracks
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let decoder = match track.codec.as_str() {
        "A_AAC" => {
            let config = crate::owned_aac::config::AacConfig::parse(&track.codec_private)?;
            crate::owned_aac::stream::decode_admission_bytes(u16::from(config.channels))?
        }
        "A_ALAC" => crate::owned_alac::AlacDecoder::decode_admission_bytes(
            &track.codec_private,
            u32::try_from(track.sample_rate)
                .map_err(|_| invalid("ALAC sample rate exceeds decoder geometry"))?,
            u16::try_from(track.channels)
                .map_err(|_| invalid("ALAC channel count exceeds decoder geometry"))?,
        )
        .map_err(|e| invalid(&e.to_string()))?,
        _ => {
            return Err(invalid(
                "Matroska audio aggregate allocation admission is not yet implemented",
            ));
        }
    };
    let mut visited_packets = 0;
    let mut largest_packet = 0;
    reader
        .scan_all_with_admission(|reader| {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(crate::owned_ebml::Error("media operation cancelled".into()));
            }
            crate::owned_budget::check_rss_budget(options).map_err(crate::owned_ebml::Error)?;
            let track = reader.tracks.get(index).ok_or_else(|| {
                crate::owned_ebml::Error("selected audio stream is absent".into())
            })?;
            for packet in &reader.packets[visited_packets..] {
                if packet.track == track.number {
                    largest_packet = largest_packet.max(packet.size);
                }
            }
            visited_packets = reader.packets.len();
            let packet = largest_packet;
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
    Ok(())
}
/// Decode the presentation timeline, including delay, signed padding, gaps and
/// ceil-rounded interval boundaries. An error may leave partial caller-owned PCM.
/// Progress never reports publication/completion. AAC/ALAC controlled admission is
/// checked before decoder construction; other codec admission and metadata
/// mutation remain unsupported by this raw-stream API.
pub(crate) fn decode_matroska_audio_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    codec: &str,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() && !matches!(codec, "A_AAC" | "A_ALAC") {
        return Err(invalid(
            "Matroska audio aggregate allocation admission is not yet implemented",
        ));
    }
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
    } else if codec == "A_ALAC" {
        open_audio_reader(source, options, options.max_packet_bytes)?
    } else {
        MatroskaTimelineReader::open(
            source,
            crate::owned_webm::Limits {
                packet_bytes: options.max_packet_bytes,
                ..Default::default()
            },
        )?
    };
    let index = matroska_audio_index(&reader, selected)?;
    if if codec == "PCM" {
        !matches!(
            reader.tracks[index].codec.as_str(),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
        )
    } else {
        reader.tracks[index].codec != codec
    } {
        return Err(invalid(
            "selected Matroska audio stream has a different codec",
        ));
    }
    if matches!(codec, "A_AAC" | "A_ALAC") {
        admit_audio_reader(&mut reader, index, options)?;
    }
    decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
}
include!("owned_matroska_audio_timeline_impl.rs");
