//! Owned streaming Matroska packet muxing. The shared writer is also used by
//! the frontend; callers discard partial output on error and own publication.
use fvid_control::{ProgressEvent, CancelFlag, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_matroska_ebml_impl.rs");
include!("owned_matroska_packet_impl.rs");

/// A named point in the file a player can jump to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub start_ns: u64,
    /// Explicit exclusive end in Matroska ticks (nanoseconds), when present.
    pub end_ns: Option<u64>,
    /// The first `ChapterDisplay` string, empty when the atom names no title.
    pub title: String,
}
/// File-wide tags and flat, unordered chapters; times are nanoseconds.
#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub tags: crate::owned_file_tags::FileTags,
    pub chapters: Vec<Chapter>,
}
include!("owned_matroska_file_impl.rs");

/// Container colour codes, independent of a decoder or colour converter.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColourDescription {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chromaticity {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContentLight {
    pub max_cll: f32,
    pub max_fall: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MasteringDisplay {
    pub red: Chromaticity,
    pub green: Chromaticity,
    pub blue: Chromaticity,
    pub white: Chromaticity,
    pub max_luminance: f32,
    pub min_luminance: f32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HdrMetadata {
    pub mastering: Option<MasteringDisplay>,
    pub light: ContentLight,
}
/// Crop in coded pixels and exact pixel aspect, with optional colour/HDR tags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMetadata {
    pub crop: [u32; 4],
    pub pixel_aspect: (u32, u32),
    pub colour: Option<ColourDescription>,
    pub hdr: HdrMetadata,
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
include!("owned_matroska_video_impl.rs");

/// Descriptive track fields preserved independently of file-wide text tags.
#[derive(Clone, Debug)]
pub struct VideoTrackDescription {
    pub name: String,
    pub language: String,
    pub legacy_language: String,
    pub disposition: i32,
}

impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    /// Open one FFV1 v1 video track. Codec configuration lives in its keyframes.
    /// This constructor declares coded geometry without colour/rotation tags.
    pub fn new_ffv1(output: &'a mut W, width: u32, height: u32) -> Result<Self> {
        Self::new_ffv1_with_metadata(output, width, height, None, 0, 0)
    }
    /// Validate colour/HDR, crop, exact aspect and rotation before output.
    pub fn new_ffv1_with_metadata(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
    ) -> Result<Self> {
        Self::new_ffv1_with_file_metadata(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            &FileMetadata::default(),
        )
    }
    /// Validate file tags/chapters and video metadata before emitting any bytes.
    pub fn new_ffv1_with_file_metadata(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
    ) -> Result<Self> {
        Self::new_ffv1_with_text_tags(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            &Default::default(),
        )
    }
    /// Preserve file-wide text tags outside the canonical player tag catalog.
    pub fn new_ffv1_with_text_tags(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_ffv1_with_scoped_tags(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            &Default::default(),
        )
    }
    /// Write file tags and tags scoped to the new video TrackUID (one).
    pub fn new_ffv1_with_scoped_tags(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        Self::new_ffv1_described(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            track_tags,
            None,
        )
    }
    pub fn new_ffv1_with_track_description(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
        description: &VideoTrackDescription,
    ) -> Result<Self> {
        Self::new_ffv1_described(
            output,
            width,
            height,
            metadata,
            rotation,
            default_duration_ns,
            file,
            text_tags,
            track_tags,
            Some(description),
        )
    }
    fn new_ffv1_described(
        output: &'a mut W,
        width: u32,
        height: u32,
        metadata: Option<&VideoMetadata>,
        rotation: u16,
        default_duration_ns: u64,
        file: &FileMetadata,
        text_tags: &std::collections::BTreeMap<String, String>,
        track_tags: &std::collections::BTreeMap<String, String>,
        description: Option<&VideoTrackDescription>,
    ) -> Result<Self> {
        let mut file_elements = file_metadata(file)?;
        file_elements.extend(extra_text_tags(text_tags)?);
        file_elements.extend(text_tags_element(track_tags, Some(1))?);
        let geometry = video_element(width, height, metadata, rotation)?;
        let entries = element(
            0xae,
            &[
                uint(0xd7, 1)?,
                uint(0x73c5, 1)?,
                uint(0x83, 1)?,
                uint(0x9c, 0)?,
                element(0x86, b"V_FFV1")?,
                geometry,
                if default_duration_ns == 0 {
                    Vec::new()
                } else {
                    uint(0x23e383, default_duration_ns)?
                },
                track_description_elements(description)?,
            ]
            .concat(),
        )?;
        Self::new_prepared(output, &entries, &file_elements, vec![0], vec![None])
    }
}

fn track_description_elements(description: Option<&VideoTrackDescription>) -> Result<Vec<u8>> {
    let Some(description) = description else {
        return element(0x22b59c, b"und");
    };
    if description.name.len() > 1024
        || description.language.is_empty()
        || description.language.len() > 128
        || description.name.contains('\0')
        || description.language.contains('\0')
        || description.legacy_language.is_empty()
        || description.legacy_language.len() > 128
        || description.legacy_language.contains('\0')
        || description.disposition & !(1 | 4 | 8 | 64 | 128 | 256 | 131072) != 0
    {
        return Err(invalid("unsupported Matroska track description"));
    }
    let mut data = Vec::new();
    if !description.name.is_empty() {
        data.extend(element(0x536e, description.name.as_bytes())?);
    }
    data.extend(element(0x22b59c, description.legacy_language.as_bytes())?);
    data.extend(element(0x22b59d, description.language.as_bytes())?);
    for (id, bit) in [
        (0x88, 1),
        (0x55aa, 64),
        (0x55ab, 128),
        (0x55ac, 256),
        (0x55ad, 131072),
        (0x55ae, 4),
        (0x55af, 8),
    ] {
        data.extend(uint(id, u64::from(description.disposition & bit != 0))?);
    }
    Ok(data)
}

fn extra_text_tags(tags: &std::collections::BTreeMap<String, String>) -> Result<Vec<u8>> {
    text_tags_element(tags,None)
}
fn text_tags_element(
    tags: &std::collections::BTreeMap<String, String>,
    uid: Option<u64>,
) -> Result<Vec<u8>> {
    if tags.len() > 256 {
        return Err(invalid("Matroska text tag count exceeds limit"));
    }
    let mut entries = Vec::new();
    for (name, value) in tags {
        if name.is_empty()
            || name.len() > 128
            || value.len() > 1024
            || name.contains('\0')
            || value.contains('\0')
        {
            return Err(invalid("invalid Matroska text tag"));
        }
        if uid.is_none() && crate::owned_file_tags::FileTags::supports_key(name) {
            continue;
        }
        let simple = element(
            0x67c8,
            &[
                element(0x45a3, name.as_bytes())?,
                element(0x4487, value.as_bytes())?,
            ]
            .concat(),
        )?;
        let targets = uid
            .map(|uid| element(0x63c0, &uint(0x63c5, uid)?))
            .transpose()?
            .unwrap_or_default();
        entries.extend(element(0x7373, &[targets, simple].concat())?);
    }
    if entries.is_empty() {
        Ok(Vec::new())
    } else {
        element(0x1254c367, &entries)
    }
}

/// Encode and mux transformed Y4M frames into one FFV1/Matroska stream.
/// Preserve exact pixel aspect and colour range; no external codecs or packet index.
/// Output must begin at offset zero. `done` stays false until caller publication.
pub fn write_y4m_ffv1<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    write_y4m_ffv1_controlled(source, output, transform, None, None)
}

/// Writer-level cancellation/progress. Completion is owned by file publication.
pub fn write_y4m_ffv1_controlled<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    write_y4m_ffv1_policy(
        source,
        output,
        transform,
        cancel,
        progress,
        usize::MAX,
        None,
        false,
        &FileMetadata::default(),
        None,
    )
    .map(|(stats, event, _)| (stats, event))
}
// Check cancellation while buffering temporal filters, before they emit packets.
struct CancelReader<'a, R> {
    source: R,
    cancel: Option<&'a fvid_control::CancelFlag>,
}
impl<R: std::io::BufRead> std::io::Read for CancelReader<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        check(self.cancel).map_err(std::io::Error::other)?;
        self.source.read(bytes)
    }
}
impl<R: std::io::BufRead> std::io::BufRead for CancelReader<'_, R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        check(self.cancel).map_err(std::io::Error::other)?;
        self.source.fill_buf()
    }
    fn consume(&mut self, bytes: usize) {
        self.source.consume(bytes);
    }
}

fn write_y4m_ffv1_policy<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    rebase_interval: bool,
    file_metadata: &FileMetadata,
    track_options: Option<&fvid_control::CopyOptions>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    check(cancel)?;
    let mut output = Some(output);
    let mut writer = None;
    let (stats, consumed) = crate::owned_ffv1_encoder::encode_y4m_counted(
        CancelReader { source, cancel },
        transform,
        max_packets,
        |header, packet, pts, duration| {
            check(cancel).map_err(|e| e.to_string())?;
            if packet.len() > max_packet_bytes {
                return Err("encoded FFV1 packet exceeds byte limit".into());
            }
            if writer.is_none() {
                let width = u32::try_from(header.width).map_err(|_| "Matroska width overflow")?;
                let height =
                    u32::try_from(header.height).map_err(|_| "Matroska height overflow")?;
                let mut description = VideoTrackDescription { name: String::new(), language: "und".into(), legacy_language: "und".into(), disposition: 1 };
                let mut track_tags = std::collections::BTreeMap::new();
                let mut text_tags = std::collections::BTreeMap::new();
                if let Some(options) = track_options {
                    for (key, value) in &options.metadata_set {
                        text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
                        if !crate::owned_file_tags::FileTags::supports_key(key) && !value.is_empty() {
                            text_tags.insert(key.to_ascii_uppercase(), value.clone());
                        }
                    }
                    for (_, key, value) in &options.stream_metadata_set {
                        track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
                        if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value) && !value.is_empty() {
                            track_tags.insert(key.to_ascii_uppercase(), value.clone());
                        }
                    }
                }
                writer = Some(
                    PacketWriter::new_ffv1_described(
                        output.take().unwrap(),
                        width,
                        height,
                        Some(&VideoMetadata {
                            pixel_aspect: header.pixel_aspect()?,
                            colour: Some(ColourDescription {
                                primaries: 0,
                                transfer: 0,
                                matrix: 6,
                                full_range: header.full_range()?,
                            }),
                            ..Default::default()
                        }),
                        0,
                        0,
                        file_metadata,
                        &text_tags,
                        &track_tags,
                        track_options.filter(|options| !options.stream_metadata_set.is_empty()).map(|_| &description),
                    )
                    .map_err(|e| e.to_string())?,
                );
            }
            let writer = writer.as_mut().unwrap();
            writer
                .write_packet(
                    0,
                    if rebase_interval {
                        let origin = transform
                            .interval
                            .map_or(0, |(from, _)| from as u128 * 1000);
                        pts.checked_sub(
                            u64::try_from(origin).map_err(|_| "interval timestamp overflow")?,
                        )
                        .ok_or("interval timestamp underflow")?
                    } else {
                        pts
                    },
                    duration,
                    true,
                    packet,
                )
                .map_err(|e| e.to_string())?;
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
            check(cancel).map_err(|e| e.to_string())
        },
    )
    .map_err(Error)?;
    let event = writer
        .ok_or_else(|| invalid("Matroska has no selected frames"))?
        .finish()?;
    Ok((stats, event, consumed))
}

fn check(cancel: Option<&fvid_control::CancelFlag>) -> Result<()> {
    if cancel.is_some_and(fvid_control::CancelFlag::is_cancelled) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Export owned progressive Y4M transforms to FFV1 in `.mkv`. No overwrite:
/// a same-directory temporary file is synced, then linked atomically. Failure
/// or cancellation removes the temporary; done is emitted only after linking.
/// This video-only API preserves Y4M aspect and colour range. The matrix follows
/// the native Y4M converter (BT.601); no primaries/transfer or HDR are inferred.
pub fn export_y4m_ffv1(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    export_y4m_ffv1_policy(
        source,
        destination,
        transform,
        cancel,
        progress,
        usize::MAX,
        None,
        false,
        &FileMetadata::default(),
        None,
    )
    .map(|(stats, event, _)| (stats, event))
}
pub(crate) fn export_y4m_ffv1_policy(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    rebase_interval: bool,
    file_metadata: &FileMetadata,
    track_options: Option<&fvid_control::CopyOptions>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    let input = std::io::BufReader::new(std::fs::File::open(source)?);
    export_atomic(destination, cancel, progress, |file| {
        write_y4m_ffv1_policy(
            input, file, transform, cancel, progress, max_packet_bytes,
            max_packets, rebase_interval, file_metadata, track_options,
        )
    })
}

pub(crate) fn export_atomic(
    destination: &std::path::Path,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    write: impl FnOnce(&mut std::fs::File) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent, u64)> {
    check(cancel)?;
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err(invalid("owned FFV1 export requires .mkv"));
    }
    if destination.symlink_metadata().is_ok() {
        return Err(invalid("output already exists"));
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut reserved = None;
    for _ in 0..100 {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = directory.join(format!(
            ".fvid-matroska-{}-{serial}.tmp",
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                reserved = Some((Temporary(path), file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let (temporary, mut file) =
        reserved.ok_or_else(|| invalid("cannot reserve Matroska output"))?;
    let (stats, mut event, consumed) = write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    check(cancel)?;
    std::fs::hard_link(&temporary.0, destination)?;
    drop(temporary);
    event.done = true;
    if let Some(hook) = progress {
        hook.emit(event);
    }
    Ok((stats, event, consumed))
}

impl MasteringDisplay {
    pub fn from_corners(
        red: (f64, f64),
        green: (f64, f64),
        blue: (f64, f64),
        white: (f64, f64),
        max_luminance: f32,
        min_luminance: f32,
    ) -> Option<Self> {
        let point = |(x, y): (f64, f64)| {
            (x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
                .then_some(Chromaticity { x, y })
        };
        let display = Some(Self {
            red: point(red)?,
            green: point(green)?,
            blue: point(blue)?,
            white: point(white)?,
            max_luminance,
            min_luminance,
        })?;
        (display.max_luminance.is_finite()
            && display.min_luminance.is_finite()
            && display.max_luminance >= 0.0
            && display.min_luminance >= 0.0)
            .then_some(display)
    }

}

impl From<crate::owned_aac::Error> for Error {
    fn from(error: crate::owned_aac::Error) -> Self { Self(error.to_string()) }
}
use crate::owned_codec_config::{AacConfig, AvcConfig, HevcConfig};
use crate::owned_opus_packet as opus_packet;
include!("owned_matroska_track_types_impl.rs");
include!("owned_matroska_tracks_impl.rs");
include!("owned_matroska_constructors_impl.rs");
fn video(width: u32, height: u32, metadata: Option<&VideoMetadata>, rotation: u16) -> Result<Vec<u8>> {
    video_element(width, height, metadata, rotation)
}

use crate::owned_aac::adts;
include!("owned_matroska_adts_impl.rs");
#[cfg(test)]
mod adts_mux_tests {
    use super::*;
    #[test]
    fn synthetic_adts_segments_keep_nanosecond_clock_and_payload() {
        let data = [0xff, 0xf1, 0x50, 0x80, 1, 0x1f, 0xfc, 0xe0];
        let sources = (0..2).map(|_| adts::StreamReader::open(data.as_slice()).unwrap()).collect();
        let mut output = std::io::Cursor::new(Vec::new());
        let event = concat_adts(sources, &mut output, None, None).unwrap();
        assert_eq!(event.packets, 2);
        assert!(!event.done);
        let mut reader = crate::owned_webm::WebmReader::open(output, Default::default()).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[0].sample_rate, 44100);
        assert_eq!(reader.tracks[0].channels, 2);
        assert_eq!(reader.packets[1].pts_ns, 1024 * 1_000_000_000 / 44100);
        assert_eq!(reader.read_packet(0).unwrap(), [0xe0]);
        assert_eq!(reader.read_packet(1).unwrap(), [0xe0]);
    }
}

include!("owned_hdr_payload_impl.rs");

impl From<crate::owned_mp4::Error> for Error {
    fn from(e: crate::owned_mp4::Error) -> Self { Self(e.to_string()) }
}
use crate::owned_mp4::Track as Mp4AacTrack;
use crate::owned_codec_config::aac_specific_config;
include!("owned_mp4_aac_plan_impl.rs");
impl FileMetadata {
    pub fn from_mp4<R: Read + Seek>(input: &crate::owned_mp4::Mp4Reader<R>) -> Self {
        Self { tags: input.tags().clone(), chapters: input.chapters().iter().map(|c| Chapter { start_ns: c.start_ns, end_ns: None, title: c.title.clone() }).collect() }
    }
}


#[cfg(test)]
mod reverse_cancel_tests {
    #[test]
    fn cancellation_stops_reverse_input_before_any_replay_or_packet_write() {
        use std::io::{BufRead, Cursor, Read};
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        struct Input {
            cursor: Cursor<&'static [u8]>,
            flag: fvid_control::CancelFlag,
            at: Arc<AtomicU64>,
            cutoff: u64,
        }
        impl Read for Input {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                let n = self.cursor.read(bytes)?;
                self.at.store(self.cursor.position(), Ordering::Relaxed);
                if self.cursor.position() >= self.cutoff {
                    self.flag.cancel();
                }
                Ok(n)
            }
        }
        impl BufRead for Input {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                self.cursor.fill_buf()
            }
            fn consume(&mut self, n: usize) {
                self.cursor.consume(n);
            }
        }
        let bytes: &'static [u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/reverse-six-frames.y4m");
        let cutoff = (bytes.iter().position(|&b| b == b'\n').unwrap() + 1 + 6 + 24) as u64;
        let flag = fvid_control::CancelFlag::new();
        let at = Arc::new(AtomicU64::new(0));
        let input = Input {
            cursor: Cursor::new(bytes),
            flag: flag.clone(),
            at: at.clone(),
            cutoff,
        };
        let mut output = Cursor::new(Vec::new());
        let transform = fvid_media_info::DecodeTransform {
            reverse: Some(String::new()),
            ..Default::default()
        };
        let error =
            super::write_y4m_ffv1_controlled(input, &mut output, &transform, Some(&flag), None)
                .unwrap_err();
        assert!(
            error.to_string().contains("media operation cancelled"),
            "{error}"
        );
        assert_eq!(at.load(Ordering::Relaxed), cutoff);
        assert!(output.into_inner().is_empty());
    }
}
