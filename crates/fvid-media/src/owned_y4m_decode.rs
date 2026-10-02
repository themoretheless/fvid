//! Owned raw planar Y4M decode-and-discard, with bounded scratch storage.
use crate::owned_y4m::{Header, PixelFormat, line};
use fvid_media_info::DecodeStats;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

/// Recognize only the progressive planar grammar owned by FVid. Legacy callers
/// retain their existing route for unsupported interlace/chroma configurations.
pub(crate) fn supports(source: &Path) -> bool {
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    line(&mut reader, &mut bytes).is_ok_and(|present| present) && Header::parse(&bytes).is_ok()
}

/// Consume every raw sample byte rather than treating a metadata-only seek as
/// decoding. No RGB conversion, external codec, or frame-sized allocation.
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_reader(BufReader::new(
        File::open(source).map_err(|e| e.to_string())?,
    ))
}
pub fn decode_reader(mut source: impl BufRead) -> Result<DecodeStats> {
    let mut bytes = Vec::new();
    if !line(&mut source, &mut bytes)? {
        return Err("empty Y4M input".into());
    }
    let header = Header::parse(&bytes)?;
    // Keep frame-rate validation identical to owned probing, even though raw
    // decode/discard does not need a presentation clock.
    header.frame_rate()?;
    let width = u32::try_from(header.width).map_err(|_| "Y4M width exceeds decode API range")?;
    let height = u32::try_from(header.height).map_err(|_| "Y4M height exceeds decode API range")?;
    let frame_bytes = header.frame_len()?;
    let mut scratch = [0u8; 8192];
    let mut frames = 0u64;
    while line(&mut source, &mut bytes)? {
        if bytes != b"FRAME\n" && !bytes.starts_with(b"FRAME ") {
            return Err("expected Y4M FRAME marker".into());
        }
        let mut remaining = frame_bytes;
        while remaining != 0 {
            let count = remaining.min(scratch.len());
            source.read_exact(&mut scratch[..count]).map_err(|error| {
                if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    "truncated Y4M frame payload".into()
                } else {
                    error.to_string()
                }
            })?;
            remaining -= count;
        }
        frames = frames.checked_add(1).ok_or("Y4M frame count overflow")?;
    }
    let layout = match header.format {
        PixelFormat::Yuv420 => "420",
        PixelFormat::Yuv422 => "422",
        PixelFormat::Yuv444 => "444",
    };
    let pixel_format = if header.depth() == 8 {
        format!("yuv{layout}p")
    } else {
        format!("yuv{layout}p{}le", header.depth())
    };
    Ok(DecodeStats {
        backend: "owned Y4M raw decode",
        video_frames: frames,
        width,
        height,
        pixel_format,
        decode_errors: 0,
    })
}
