//! Owned MP4 presentation spool feeding the established FFV1 frame pipeline.
//! This transitional bridge trades temporary disk I/O for bounded frame RAM.
use crate::{owned_matroska as mkv, owned_mp4_video_decode as decode};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
type Result<T> = std::result::Result<T, String>;
static SERIAL: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Temporary {
    pub path: PathBuf,
    file: Option<File>,
}
impl Temporary {
    fn new() -> Result<Self> {
        loop {
            let path = std::env::temp_dir().join(format!(
                "fvid-mp4-presented-{}-{}.mkv",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}
pub(crate) fn recognizes(source: &Path) -> bool {
    let Ok(mut file) = File::open(source) else {
        return false;
    };
    let mut prefix = [0; 8];
    file.read_exact(&mut prefix).is_ok() && crate::owned_mp4::recognizes_prefix(&prefix)
}
pub(crate) fn prepare(
    source: &Path,
    options: Option<&fvid_control::CopyOptions>,
) -> Result<Option<Temporary>> {
    prepare_selected(source, options, false)
}
pub(crate) fn prepare_video(source: &Path) -> Result<Option<Temporary>> {
    prepare_selected(source, None, true)
}
fn prepare_selected(
    source: &Path,
    options: Option<&fvid_control::CopyOptions>,
    video_only: bool,
) -> Result<Option<Temporary>> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    if file.read(&mut prefix).map_err(|e| e.to_string())? != 8
        || !crate::owned_mp4::recognizes_prefix(&prefix)
    {
        return Ok(None);
    }
    if options.is_some_and(|o| o.max_controlled_bytes.is_some() || o.max_rss_bytes.is_some()) {
        return Ok(None);
    }
    let reader = match crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    ) {
        Ok(value) => value,
        Err(e) if e.is_unsupported() => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    // Decode statistics intentionally selects video; exported files must retain
    // every source track until a multitrack bridge is implemented.
    if !video_only && (reader.tracks().len() != 1 || !reader.refused().is_empty()) {
        return Ok(None);
    }
    let Some(track) = reader.tracks().iter().find(|t| t.handler == *b"vide") else {
        return Ok(None);
    };
    if track.rotation != 0 {
        return Ok(None);
    }
    let aspect = track.pixel_aspect;
    let file_metadata = mkv::FileMetadata::from_mp4(&reader);
    let description = mkv::VideoTrackDescription {
        name: track.name.clone(),
        language: if track.language.is_empty() {
            "und".into()
        } else {
            track.language.clone()
        },
        legacy_language: if track.language.is_empty() {
            "und".into()
        } else {
            track.language.clone()
        },
        disposition: 1,
    };
    drop(reader);
    let mut temporary = Temporary::new()?;
    let mut output = Some(temporary.file.as_mut().unwrap());
    let mut writer = None;
    let mut format = None;
    let mut visitor =
        |frame: &decode::FrameMetadata, pixels: &[u8], pts: u64, duration: u64| -> Result<()> {
            if options
                .and_then(|o| o.cancel.as_ref())
                .is_some_and(fvid_control::CancelFlag::is_cancelled)
            {
                return Err("media operation cancelled".into());
            }
            let current = (
                frame.size,
                frame.depth,
                frame.sub,
                frame.mono,
                frame.colour,
                frame.hdr,
            );
            if format.is_some_and(|previous| previous != current) {
                return Err("unsupported MP4 bridge: changing frame format or colour".into());
            }
            format = Some(current);
            let metadata = mkv::VideoMetadata {
                pixel_aspect: aspect,
                colour: Some(frame.colour),
                hdr: frame.hdr,
                ..Default::default()
            };
            if writer.is_none() {
                writer = Some(
                    mkv::PacketWriter::new_ffv1_with_track_description(
                        output.take().unwrap(),
                        frame.size[0],
                        frame.size[1],
                        Some(&metadata),
                        0,
                        0,
                        &file_metadata,
                        &Default::default(),
                        &Default::default(),
                        &description,
                    )
                    .map_err(|e| e.to_string())?,
                );
            }
            let packet = if frame.mono {
                let length = (frame.size[0] as usize)
                    .checked_mul(frame.size[1] as usize)
                    .and_then(|n| n.checked_mul(if frame.depth == 8 { 1 } else { 2 }))
                    .ok_or("monochrome frame size overflow")?;
                crate::owned_ffv1_encoder::encode_gray(
                    frame.size[0] as usize,
                    frame.size[1] as usize,
                    &pixels[..length],
                    frame.depth,
                )?
            } else {
                let mut data = crate::owned_frame::buffer(pixels.len())?;
                data.copy_from_slice(pixels);
                let geometry = crate::owned_frame::GeometryFrame {
                    width: frame.size[0] as usize,
                    height: frame.size[1] as usize,
                    subsampling: Some(frame.sub.map(|v| if v { 2 } else { 1 })),
                    data,
                };
                crate::owned_ffv1_encoder::encode(&geometry, frame.depth)?
            };
            if options.is_some_and(|o| packet.len() > o.max_packet_bytes) {
                return Err("encoded FFV1 packet exceeds byte limit".into());
            }
            writer
                .as_mut()
                .unwrap()
                .write_packet(0, pts, duration, true, &packet)
                .map_err(|e| e.to_string())
        };
    let result = decode::decode_presented(source, &Default::default(), Some(&mut visitor), options);
    drop(visitor);
    match result {
        Ok(None) => return Ok(None),
        Err(e) if e.starts_with("unsupported MP4 bridge:") => return Ok(None),
        Err(e) => return Err(e),
        Ok(Some(_)) => {}
    }
    let writer = writer.ok_or("MP4 has no selected frames")?;
    writer.finish().map_err(|e| e.to_string())?;
    temporary
        .file
        .as_mut()
        .unwrap()
        .flush()
        .map_err(|e| e.to_string())?;
    drop(temporary.file.take());
    Ok(Some(temporary))
}
