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

impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    /// Open one FFV1 v1 video track. Codec configuration lives in its keyframes.
    /// This constructor declares coded geometry without colour/rotation tags.
    pub fn new_ffv1(output: &'a mut W, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(invalid("empty Matroska video dimensions"));
        }
        let geometry = element(
            0xe0,
            &[
                uint(0xb0, u64::from(width))?,
                uint(0xba, u64::from(height))?,
            ]
            .concat(),
        )?;
        let entries = element(
            0xae,
            &[
                uint(0xd7, 1)?,
                uint(0x73c5, 1)?,
                uint(0x83, 1)?,
                uint(0x9c, 0)?,
                element(0x86, b"V_FFV1")?,
                geometry,
                element(0x22b59c, b"und")?,
            ]
            .concat(),
        )?;
        Self::new_prepared(output, &entries, &[], vec![0], vec![None])
    }
}

/// Encode and mux transformed Y4M frames into one FFV1/Matroska stream.
/// No audio, metadata synthesis, external codecs or retained packet index.
/// Output must begin at offset zero. `done` stays false until caller publication.
pub fn write_y4m_ffv1<W: Write + Seek>(
    source: impl std::io::BufRead,
    output: &mut W,
    transform: &fvid_media_info::DecodeTransform,
) -> Result<(fvid_media_info::DecodeStats, ProgressEvent)> {
    let mut output = Some(output);
    let mut writer = None;
    let stats = crate::owned_ffv1_encoder::encode_y4m(
        source,
        transform,
        |header, packet, pts, duration| {
            if writer.is_none() {
                let width = u32::try_from(header.width).map_err(|_| "Matroska width overflow")?;
                let height =
                    u32::try_from(header.height).map_err(|_| "Matroska height overflow")?;
                writer = Some(
                    PacketWriter::new_ffv1(output.take().unwrap(), width, height)
                        .map_err(|e| e.to_string())?,
                );
            }
            writer
                .as_mut()
                .unwrap()
                .write_packet(0, pts, duration, true, packet)
                .map_err(|e| e.to_string())
        },
    )
    .map_err(Error)?;
    let event = writer
        .ok_or_else(|| invalid("Matroska has no selected frames"))?
        .finish()?;
    Ok((stats, event))
}
