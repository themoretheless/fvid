//! Owned MP4 AAC/ALAC presentation decoding, including silence and repeated edits.
use crate::owned_aac::AacCheckpoint as Mp4AacCheckpoint;
use crate::owned_matroska_audio::{invalid, DecodeProgress};
pub use crate::owned_matroska_audio::{AudioDecodeStats, Error};
use crate::owned_mp4::Mp4Reader as Mp4TimelineReader;
use crate::owned_mp4_audio_schedule::AudioTimeline as Mp4AudioSchedule;
use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, Write},
    time::Duration,
};
type Result<T> = std::result::Result<T, Error>;
impl From<crate::owned_mp4::Error> for Error {
    fn from(e: crate::owned_mp4::Error) -> Self {
        invalid(&e.to_string())
    }
}
pub(crate) enum Mp4TimelineDecoder {
    Aac(crate::owned_aac::NativeAacDecoder),
    Alac(crate::owned_alac::AlacDecoder),
}
impl Mp4TimelineDecoder {
    pub(crate) fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
        match &track.codec {
            b"mp4a" => Ok(Self::Aac(crate::owned_aac::NativeAacDecoder::new(
                crate::owned_codec_config::aac_specific_config(&track.configuration)?,
            )?)),
            b"alac" => {
                if track.configuration.len() < 24
                    || u32::from_be_bytes(track.configuration[20..24].try_into().unwrap())
                        != track.sample_rate
                {
                    return Err(invalid("ALAC cookie and track sample rates disagree"));
                }
                Ok(Self::Alac(crate::owned_alac::AlacDecoder::new(
                    &track.configuration,
                    track.sample_rate,
                    track.channels,
                )?))
            }
            _ => Err(invalid(
                "selected MP4 audio codec is not owned by this export path",
            )),
        }
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Aac(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Aac(d) => u16::from(d.channels()),
            Self::Alac(d) => d.channels(),
        }
    }
    pub(crate) fn channel_mask(&self) -> u32 {
        match self {
            Self::Aac(d) => d.channel_mask(),
            Self::Alac(d) => {
                crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32
            }
        }
    }
    fn reset(&mut self) {
        if let Self::Aac(d) = self {
            d.reset();
        }
    }
    fn decode(&mut self, bytes: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::Aac(d) => Ok(d.decode(bytes)?),
            Self::Alac(d) => Ok(d.decode_pcm(bytes)?),
        }
    }
}
pub(crate) fn mp4_audio_index<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    selected: Option<usize>,
) -> Result<usize> {
    let indices: Vec<_> = reader
        .tracks()
        .iter()
        .enumerate()
        .filter_map(|(index, t)| (t.handler == *b"soun").then_some(index))
        .collect();
    match selected {
        Some(index) if indices.contains(&index) => Ok(index),
        None if indices.len() == 1 => Ok(indices[0]),
        _ => Err(invalid("select exactly one audio stream")),
    }
}
/// Decode normalized float32 PCM. Errors may leave partial caller-owned output;
/// progress never reports publication. Aggregate admission and metadata edits
/// are unsupported here. Encoded packet limits include required preroll.
pub fn decode_mp4_audio_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() {
        return Err(invalid(
            "MP4 audio aggregate allocation admission is not yet implemented",
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
        event: fvid_control::ProgressEvent {
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
    let reader = Mp4TimelineReader::open(
        source,
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )?;
    decode_mp4_audio_reader_controlled(reader, output, interval, selected, &mut control)
}
include!("owned_mp4_audio_timeline_impl.rs");
