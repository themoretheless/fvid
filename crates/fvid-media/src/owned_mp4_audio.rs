//! Owned MP4 AAC/ALAC/PCM presentation decoding, including silence and repeated edits.
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
    Pcm(crate::owned_pcm_decoder::PcmDecoder),
}
impl Mp4TimelineDecoder {
    const SAMPLE_BYTES: usize = 4;
    pub(crate) fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
        if let Self::Aac(d) = self {
            Some(d.checkpoint())
        } else {
            None
        }
    }
    pub(crate) fn restore_checkpoint(&mut self, state: &Mp4AacCheckpoint) -> Result<bool> {
        if let Self::Aac(d) = self {
            d.restore(state)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

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
            b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" => Ok(Self::Pcm(
                crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                    .map_err(|e| invalid(&e.to_string()))?,
            )),
            _ => Err(invalid(
                "selected MP4 audio codec is not owned by this export path",
            )),
        }
    }
    pub(crate) fn sample_rate(&self) -> u32 {
        match self {
            Self::Aac(d) => d.sample_rate(),
            Self::Alac(d) => d.sample_rate(),
            Self::Pcm(d) => d.sample_rate(),
        }
    }
    pub(crate) fn channels(&self) -> u16 {
        match self {
            Self::Aac(d) => u16::from(d.channels()),
            Self::Alac(d) => d.channels(),
            Self::Pcm(d) => d.channels(),
        }
    }
    pub(crate) fn channel_mask(&self) -> u32 {
        match self {
            Self::Aac(d) => d.channel_mask(),
            Self::Pcm(d) => crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32,
            Self::Alac(d) => crate::owned_pcm_channels::standard_mask(d.channels()).unwrap_or(0) as u32,
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
            Self::Pcm(d) => d.decode_pcm(bytes).map_err(|e| invalid(&e.to_string())),
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
/// Admit retained MP4 AAC/ALAC/PCM decode payload before cloning the track or creating
/// decoder/checkpoint state. Container parsing has its own bounded limits;
/// parser temporaries and caller-owned I/O are outside this retained estimate.
pub(crate) fn admit_audio_reader<R: Read + Seek>(
    reader: &Mp4TimelineReader<R>,
    index: usize,
    options: &CopyOptions,
) -> Result<()> {
    let Some(limit) = options.max_controlled_bytes else {
        return Ok(());
    };
    let track = reader
        .tracks()
        .get(index)
        .ok_or_else(|| invalid("selected audio stream is absent"))?;
    let decoder = match &track.codec {
        b"mp4a" => {
            let config = crate::owned_aac::config::AacConfig::parse(
                crate::owned_codec_config::aac_specific_config(&track.configuration)?,
            )?;
            // Decoder plus checkpoint and replacement/restore scratch. Immutable
            // tables are shared, but charging full estimates is conservative.
            crate::owned_aac::stream::decode_admission_bytes(u16::from(config.channels))?
                .checked_mul(3)
                .ok_or_else(|| invalid("MP4 audio memory estimate overflow"))?
        }
        b"alac" => crate::owned_alac::AlacDecoder::decode_admission_bytes(
            &track.configuration,
            track.sample_rate,
            track.channels,
        )
        .map_err(|e| invalid(&e.to_string()))?,
        b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" => {
            crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                .map_err(|e| invalid(&e.to_string()))?;
            16 * 1024
        }
        _ => {
            return Err(invalid(
                "selected MP4 audio codec is not owned by this export path",
            ));
        }
    };
    let overflow = || invalid("MP4 audio memory estimate overflow");
    let mut estimated = reader.estimated_index_payload_bytes()?;
    let mut add = |bytes: usize| -> Result<()> {
        estimated = estimated.checked_add(bytes).ok_or_else(overflow)?;
        Ok(())
    };
    add(decoder)?;
    for bytes in [
        track.name.len(),
        track.language.len(),
        track.configuration.len(),
    ] {
        add(bytes)?;
    }
    add(track
        .edits
        .len()
        .checked_mul(std::mem::size_of::<crate::owned_mp4::Edit>())
        .ok_or_else(overflow)?)?;
    add(track
        .edits
        .len()
        .max(1)
        .checked_mul(std::mem::size_of::<crate::owned_mp4_audio_schedule::Segment>())
        .ok_or_else(overflow)?)?;
    let (records, width) = match &track.samples {
        crate::owned_mp4::SampleIndex::Expanded(samples) => (
            samples.len(),
            std::mem::size_of::<crate::owned_mp4::Sample>(),
        ),
        crate::owned_mp4::SampleIndex::Uniform(index) => (
            index.runs.len(),
            std::mem::size_of::<crate::owned_mp4::FrameRun>(),
        ),
    };
    add(records.checked_mul(width).ok_or_else(overflow)?)?;
    let largest_packet = (0..track.samples.len())
        .filter_map(|i| track.samples.get(i))
        .map(|s| s.size as usize)
        .max()
        .unwrap_or(0);
    // The reusable packet Vec may grow geometrically while reading larger packets.
    let pcm = matches!(
        &track.codec,
        b"raw " | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64"
    );
    // PCM output widens each byte into at most one f64 sample; retain room for
    // geometric input buffer growth and output allocation together.
    add(largest_packet
        .checked_mul(if pcm { 10 } else { 2 })
        .ok_or_else(overflow)?)?;
    if estimated > limit {
        return Err(invalid(&format!(
            "controlled memory budget exceeded: need {estimated} bytes, limit {limit}"
        )));
    }
    Ok(())
}
/// Decode normalized float32 PCM. Errors may leave partial caller-owned output;
/// progress never reports publication. Retained allocation admission is checked
/// before decoding; metadata edits remain unsupported.
/// Encoded packet limits include required preroll.
pub fn decode_mp4_audio_pcm<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
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
    let index = mp4_audio_index(&reader, selected)?;
    admit_audio_reader(&reader, index, options)?;
    decode_mp4_audio_reader_controlled(reader, output, interval, selected, &mut control)
}
include!("owned_mp4_audio_timeline_impl.rs");

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn mp4_aac_pcm_admits_index_decoder_and_edit_checkpoint() {
        let source = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/audio/aac-native-edit.m4a"),
        )
        .unwrap();
        let decode = |options: &CopyOptions| {
            let mut pcm = Vec::new();
            let stats =
                decode_mp4_audio_pcm(std::io::Cursor::new(&source), &mut pcm, None, options);
            (stats, pcm)
        };
        let (expected, pcm) = decode(&CopyOptions::default());
        let expected = expected.unwrap();
        let (actual, admitted) = decode(&CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        });
        assert_eq!(actual.unwrap(), expected);
        assert_eq!(admitted, pcm);
        for limit in [
            1,
            crate::owned_aac::stream::decode_admission_bytes(expected.channels).unwrap(),
        ] {
            let (error, output) = decode(&CopyOptions {
                max_controlled_bytes: Some(limit),
                ..Default::default()
            });
            assert!(
                error
                    .unwrap_err()
                    .to_string()
                    .contains("controlled memory budget exceeded")
            );
            assert!(output.is_empty());
        }
    }
}

/// Decode QuickTime PCM without narrowing integer32 or IEEE float64 samples.
/// Encoded timelines and controls match the normalized f32 API.
pub fn decode_mp4_pcm_f64<R: Read + Seek>(
    source: R,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    precise::decode(source, output, interval, options)
}
mod precise {
    use super::*;
    struct Mp4TimelineDecoder(crate::owned_pcm_decoder::PcmDecoder);
    impl Mp4TimelineDecoder {
        const SAMPLE_BYTES: usize = 8;
        fn new(track: &crate::owned_mp4::Track) -> Result<Self> {
            Ok(Self(
                crate::owned_pcm_decoder::PcmDecoder::from_mp4(track)
                    .map_err(|e| invalid(&e.to_string()))?,
            ))
        }
        fn sample_rate(&self) -> u32 {
            self.0.sample_rate()
        }
        fn channels(&self) -> u16 {
            self.0.channels()
        }
        fn reset(&mut self) {}
        fn checkpoint(&self) -> Option<Mp4AacCheckpoint> {
            None
        }
        fn restore_checkpoint(&mut self, _: &Mp4AacCheckpoint) -> Result<bool> {
            Ok(false)
        }
        fn decode(&mut self, bytes: &[u8]) -> Result<Vec<f64>> {
            self.0
                .decode_pcm_f64(bytes)
                .map_err(|e| invalid(&e.to_string()))
        }
    }
    pub(super) fn decode<R: Read + Seek>(
        source: R,
        output: &mut impl Write,
        interval: Option<(Duration, Duration)>,
        options: &CopyOptions,
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
        let index = mp4_audio_index(&reader, selected)?;
        admit_audio_reader(&reader, index, options)?;
        decode_mp4_audio_reader_controlled(reader, output, interval, selected, &mut control)
    }
    include!("owned_mp4_audio_timeline_impl.rs");
}
