//! Owned Matroska ALAC presentation timeline to caller-owned float32 PCM.
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
type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
struct MatroskaTimelineDecoder(crate::owned_alac::AlacDecoder);
impl MatroskaTimelineDecoder {
    fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        Ok(Self(crate::owned_alac::AlacDecoder::from_matroska(track)?))
    }
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.0.channels()
    }
    fn decode(&mut self, data: &[u8]) -> Result<Vec<f32>> {
        Ok(self.0.decode_pcm(data)?)
    }
}
struct DecodeProgress<'a> {
    options: &'a CopyOptions,
    event: ProgressEvent,
}
impl DecodeProgress<'_> {
    fn check(&self) -> Result<()> {
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
    fn packet_limit_reached(&self) -> bool {
        self.options
            .max_packets
            .is_some_and(|max| self.event.packets >= max)
    }
    fn packet(&mut self, bytes: usize) -> Result<()> {
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
fn matroska_audio_index<R: Read + Seek>(
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
    if reader.tracks[index].codec != "A_ALAC" {
        return Err(invalid("selected Matroska audio stream is not ALAC"));
    }
    Ok(index)
}
/// Decode the presentation timeline, including delay, signed padding, gaps and
/// ceil-rounded interval boundaries. An error may leave partial caller-owned PCM.
/// Progress never reports publication/completion. Aggregate allocation admission
/// and metadata mutation are explicitly unsupported by this raw-stream API.
pub fn decode_matroska_alac_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() {
        return Err(invalid(
            "Matroska ALAC aggregate allocation admission is not yet implemented",
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
    let reader = MatroskaTimelineReader::open(
        source,
        crate::owned_webm::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )?;
    decode_matroska_audio_reader_controlled(reader, output, interval, selected, &mut control)
}
include!("owned_matroska_audio_timeline_impl.rs");
