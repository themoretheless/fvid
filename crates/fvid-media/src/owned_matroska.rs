//! Owned streaming Matroska packet muxing. The shared writer is also used by
//! the frontend; callers discard partial output on error and own publication.
use fvid_control::ProgressEvent;
use std::io::{Seek, SeekFrom, Write};
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

/// Container colour codes, independent of a decoder or colour converter.
#[derive(Clone, Copy, Debug, PartialEq)]
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
                element(0x22b59c, b"und")?,
            ]
            .concat(),
        )?;
        Self::new_prepared(output, &entries, &[], vec![0], vec![None])
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
    )
}
fn write_y4m_ffv1_policy<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    check(cancel)?;
    let mut output = Some(output);
    let mut writer = None;
    let stats = crate::owned_ffv1_encoder::encode_y4m_limited(
        source,
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
                writer = Some(
                    PacketWriter::new_ffv1_with_metadata(
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
                    )
                    .map_err(|e| e.to_string())?,
                );
            }
            let writer = writer.as_mut().unwrap();
            writer
                .write_packet(0, pts, duration, true, packet)
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
    Ok((stats, event))
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
    )
}
pub(crate) fn export_y4m_ffv1_policy(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: &fvid_media_info::DecodeTransform,
    cancel: Option<&fvid_control::CancelFlag>,
    progress: Option<&fvid_control::ProgressHook>,
    max_packet_bytes: usize,
    max_packets: Option<u64>,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
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
    let input = std::io::BufReader::new(std::fs::File::open(source)?);
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
    let (stats, mut event) = write_y4m_ffv1_policy(
        input,
        &mut file,
        transform,
        cancel,
        progress,
        max_packet_bytes,
        max_packets,
    )?;
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
    Ok((stats, event))
}
