//! Owned AVC/HEVC/AAC/FFV1 Matroska muxing. No external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use crate::codec::config::{AacConfig, AvcConfig, HevcConfig};
use super::opus_packet;
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
include!("../../crates/fvid-media/src/owned_matroska_ebml_impl.rs");

/// File-level metadata. Chapter timestamps are nanoseconds on the presentation
/// timeline; editions are flat and unordered. No implicit chapter ends are added.
#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub tags: super::FileTags,
    pub chapters: Vec<super::webm::Chapter>,
}

impl FileMetadata {
    pub fn from_mp4<R: Read + Seek>(input: &super::mp4::Mp4Reader<R>) -> Self {
        Self {
            tags: input.tags().clone(),
            chapters: input
                .chapters()
                .iter()
                .map(|c| super::webm::Chapter {
                    start_ns: c.start_ns,
                    end_ns: None,
                    title: c.title.clone(),
                })
                .collect(),
        }
    }
}

include!("../../crates/fvid-media/src/owned_matroska_file_impl.rs");

include!("../../crates/fvid-media/src/owned_matroska_track_types_impl.rs");

/// Container-level video properties, independent of codec configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMetadata {
    /// Left, top, right, bottom in coded pixels.
    pub crop: [u32; 4],
    /// Width/height of one visible pixel; must be nonzero.
    pub pixel_aspect: (u32, u32),
    /// None leaves the container colour declaration absent.
    pub colour: Option<crate::color::hdr::ColourDescription>,
    pub hdr: crate::color::hdr::HdrMetadata,
}
impl Default for VideoMetadata {
    fn default() -> Self {
        Self {
            crop: [0; 4],
            pixel_aspect: (1, 1),
            colour: None,
            hdr: Default::default(),
        }
    }
}

include!("../../crates/fvid-media/src/owned_matroska_tracks_impl.rs");
fn video(
    width: u32,
    height: u32,
    metadata: Option<&VideoMetadata>,
    rotation: u16,
) -> Result<Vec<u8>> {
    use fvid_media::owned_matroska as owned;
    let metadata = metadata.map(|m| owned::VideoMetadata {
        crop: m.crop,
        pixel_aspect: m.pixel_aspect,
        colour: m.colour.map(|c| owned::ColourDescription {
            matrix: c.matrix,
            transfer: c.transfer,
            primaries: c.primaries,
            full_range: c.full_range,
        }),
        hdr: owned::HdrMetadata {
            light: owned::ContentLight {
                max_cll: m.hdr.light.max_cll,
                max_fall: m.hdr.light.max_fall,
            },
            mastering: m.hdr.mastering.map(|d| owned::MasteringDisplay {
                red: owned::Chromaticity {
                    x: d.red.x,
                    y: d.red.y,
                },
                green: owned::Chromaticity {
                    x: d.green.x,
                    y: d.green.y,
                },
                blue: owned::Chromaticity {
                    x: d.blue.x,
                    y: d.blue.y,
                },
                white: owned::Chromaticity {
                    x: d.white.x,
                    y: d.white.y,
                },
                min_luminance: d.min_luminance,
                max_luminance: d.max_luminance,
            }),
        },
    });
    owned::video_element(width, height, metadata.as_ref(), rotation)
        .map_err(|e| invalid(&e.to_string()))
}

include!("../../crates/fvid-media/src/owned_matroska_packet_impl.rs");

include!("../../crates/fvid-media/src/owned_matroska_constructors_impl.rs");

/// Stream strict ADTS packets into Matroska. Per-packet clusters avoid signed
/// block timestamp overflow at a 1 ns clock. No cue table/index is accumulated.
/// Caller must discard partial output after error and publish only on success.
pub fn write_adts<R: Read, W: Write + Seek>(
    mut input: super::adts::StreamReader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let asc = input.audio_specific_config().to_vec();
    write_aac_packets(
        input.configuration(),
        &asc,
        || input.next_packet(),
        output,
        cancel,
        progress,
    )
}

/// Append compatible ADTS segments to one Matroska track without decoding.
pub fn concat_adts<R: Read, W: Write + Seek>(
    readers: Vec<super::adts::StreamReader<R>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let mut sequence = super::adts::SequenceReader::new(readers)?;
    let asc = sequence.audio_specific_config().to_vec();
    write_aac_packets(
        sequence.configuration(),
        &asc,
        || sequence.next_packet(),
        output,
        cancel,
        progress,
    )
}

fn write_aac_packets<W: Write + Seek>(
    config: super::adts::Header,
    asc: &[u8],
    mut next_packet: impl FnMut() -> Result<Option<Vec<u8>>>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let samples = u64::from(crate::codec::config::AacConfig::parse(asc)?.frame_samples);
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: asc,
            sample_rate: config.sample_rate,
            channels: config.channels.into(),
        },
        name: "",
        language: "",
    };
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    let time = |packet: u64| -> Result<u64> {
        u64::try_from(
            u128::from(packet) * u128::from(samples) * 1_000_000_000
                / u128::from(config.sample_rate),
        )
        .map_err(|_| invalid("Matroska timestamp overflow"))
    };
    check()?;
    let mut writer = PacketWriter::new(output, &[spec])?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    loop {
        check()?;
        let Some(packet) = next_packet()? else {
            break;
        };
        let index = writer.event().packets;
        let next = index
            .checked_add(1)
            .ok_or_else(|| invalid("packet count overflow"))?;
        let start = time(index)?;
        writer.write_packet(0, start, time(next)? - start, true, &packet)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    let event = writer.finish()?;
    check()?;
    Ok(event)
}

/// Copy one MP4 AAC track into Matroska, retaining decoder pre-roll and the
/// audible boundaries of a single media edit. This is a track-level operation:
/// file tags, chapters and other tracks belong to the caller's remux policy.
/// Empty/repeated edits require timeline reconstruction and are rejected here.
/// The caller must discard partial output on failure.
pub fn write_mp4_aac<R: Read + Seek, W: Write + Seek>(
    input: &mut super::mp4::Mp4Reader<R>,
    track_index: usize,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    write_mp4_aac_metadata(
        input,
        track_index,
        output,
        &FileMetadata::default(),
        cancel,
        progress,
    )
}

/// Remux a single-track AAC MP4 file, including all represented tags and chapters.
/// Other tracks are rejected rather than silently discarded.
pub fn write_mp4_aac_file<R: Read + Seek, W: Write + Seek>(
    input: &mut super::mp4::Mp4Reader<R>,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    if input.tracks().len() != 1 || !input.refused().is_empty() {
        return Err(invalid("AAC Matroska remux requires a single audio track"));
    }
    let metadata = FileMetadata::from_mp4(input);
    write_mp4_aac_metadata(input, 0, output, &metadata, cancel, progress)
}

fn write_mp4_aac_metadata<R: Read + Seek, W: Write + Seek>(
    input: &mut super::mp4::Mp4Reader<R>,
    track_index: usize,
    output: &mut W,
    metadata: &FileMetadata,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<ProgressEvent> {
    let check = || {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            Err(invalid("media operation cancelled"))
        } else {
            Ok(())
        }
    };
    check()?;
    let track = input
        .tracks()
        .get(track_index)
        .ok_or_else(|| invalid("MP4 AAC track index out of range"))?
        .clone();
    let plan = aac_packet_plan(&track, input.movie_timescale(), cancel)?;
    let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
    let mut writer = PacketWriter::new_with_metadata(
        output,
        &[TrackSpec {
            encoding: Encoding::Aac {
                configuration: asc,
                sample_rate: plan.rate,
                channels: track.channels,
            },
            name: &track.name,
            language: &track.language,
        }],
        &[TrackOptions {
            codec_delay_ns: plan.delay,
            ..Default::default()
        }],
        metadata,
    )?;
    if let Some(h) = progress {
        h.emit(writer.event());
    }
    let mut packet = Vec::new();
    for i in 0..plan.count {
        check()?;
        input.read_packet(track_index, i, &mut packet)?;
        let (begin, duration, padding) = plan.packet(i)?;
        writer.write_packet_with_padding(0, begin, duration, true, &packet, padding)?;
        if let Some(h) = progress {
            h.emit(writer.event());
        }
    }
    check()?;
    let event = writer.finish()?;
    check()?;
    Ok(event)
}

fn check_cancel(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
fn sample_ns(samples: u64, rate: u32) -> Result<u64> {
    u64::try_from((u128::from(samples) * 1_000_000_000 + u128::from(rate) / 2) / u128::from(rate))
        .map_err(|_| invalid("AAC nanosecond timestamp overflow"))
}
pub(crate) struct AacPacketPlan {
    pub count: usize,
    pub delay: u64,
    padding: i64,
    frame: u64,
    pub rate: u32,
}
impl AacPacketPlan {
    pub fn packet(&self, index: usize) -> Result<(u64, u64, i64)> {
        let begin = sample_ns(index as u64 * self.frame, self.rate)?;
        let end = sample_ns((index as u64 + 1) * self.frame, self.rate)?;
        Ok((
            begin,
            end - begin,
            if index + 1 == self.count {
                self.padding
            } else {
                0
            },
        ))
    }
}
pub(crate) fn aac_packet_plan(
    track: &super::mp4::Track,
    movie_scale: u32,
    cancel: Option<&CancelFlag>,
) -> Result<AacPacketPlan> {
    if track.handler != *b"soun" || track.codec != *b"mp4a" {
        return Err(invalid("MP4 track is not AAC"));
    }
    let asc = crate::codec::config::aac_specific_config(&track.configuration)?;
    let config = crate::codec::config::AacConfig::parse(asc)?;
    if track.timescale == 0
        || track.sample_rate != config.sample_rate
        || track.channels != u16::from(config.channels)
    {
        return Err(invalid("MP4 AAC clock or geometry mismatch"));
    }
    let rate = u128::from(config.sample_rate);
    let position = |ticks: u64| -> Result<u64> {
        let n = u128::from(ticks) * rate;
        let d = u128::from(track.timescale);
        if n % d != 0 {
            return Err(invalid("AAC time is not sample aligned"));
        }
        u64::try_from(n / d).map_err(|_| invalid("AAC sample position overflow"))
    };
    let media_end = position(track.duration)?;
    let (start, end) = match track.edits.as_slice() {
        [] => (0, media_end),
        [edit] if edit.media_time >= 0 && movie_scale != 0 => {
            let start = position(edit.media_time as u64)?;
            let length =
                u64::try_from((u128::from(edit.duration) * rate).div_ceil(u128::from(movie_scale)))
                    .map_err(|_| invalid("AAC edit duration overflow"))?;
            (
                start,
                start
                    .checked_add(length)
                    .ok_or_else(|| invalid("AAC edit endpoint overflow"))?,
            )
        }
        _ => {
            return Err(invalid(
                "AAC packet remux requires one contiguous media edit",
            ));
        }
    };
    if start >= end || end > media_end {
        return Err(invalid("AAC edit exceeds media samples"));
    }
    let frame = u64::from(config.frame_samples);
    let count =
        usize::try_from(end.div_ceil(frame)).map_err(|_| invalid("AAC packet count overflow"))?;
    if count > track.samples.len() {
        return Err(invalid("AAC edit exceeds packet index"));
    }
    // Validate before producing a header. The AAC frame clock, rather than a
    // shortened final stts duration, determines the actual decoded frame size.
    for i in 0..track.samples.len() {
        check_cancel(cancel)?;
        let sample = track
            .samples
            .get(i)
            .ok_or_else(|| invalid("missing AAC sample"))?;
        let expected = (i as u64)
            .checked_mul(frame)
            .ok_or_else(|| invalid("AAC timeline overflow"))?;
        let duration = position(u64::from(sample.duration))?;
        if sample.pts < 0
            || sample.pts as u64 != sample.dts
            || position(sample.dts)? != expected
            || duration == 0
            || duration > frame
            || (i + 1 != track.samples.len() && duration != frame)
        {
            return Err(invalid(
                "AAC packet remux requires a contiguous frame clock",
            ));
        }
        if i + 1 == track.samples.len() && expected.checked_add(duration) != Some(media_end) {
            return Err(invalid("AAC media duration disagrees with packet timeline"));
        }
    }
    let delay = sample_ns(start, config.sample_rate)?;
    let coded_end = (count as u64)
        .checked_mul(frame)
        .ok_or_else(|| invalid("AAC timeline overflow"))?;
    let padding = i64::try_from(sample_ns(coded_end - end, config.sample_rate)?)
        .map_err(|_| invalid("AAC padding overflow"))?;
    Ok(AacPacketPlan {
        count,
        delay,
        padding,
        frame,
        rate: config.sample_rate,
    })
}
