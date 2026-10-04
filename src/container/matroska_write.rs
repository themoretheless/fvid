//! Owned AVC/HEVC/AAC/FFV1 Matroska muxing. No external muxer is used.
//! Mapping: https://www.matroska.org/technical/codec_specs.html#a_aac
use crate::{Result, invalid};
use super::FileTags;
use crate::codec::config::{AacConfig, AvcConfig, HevcConfig};
use super::{opus_packet, adts};
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

include!("../../crates/fvid-media/src/owned_matroska_adts_impl.rs");

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

use super::mp4::Track as Mp4AacTrack;
use crate::codec::config::aac_specific_config;
include!("../../crates/fvid-media/src/owned_mp4_aac_plan_impl.rs");
