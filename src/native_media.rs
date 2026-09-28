//! Media operations implemented by FVid's own container and codec pipeline.
//! This module is available without the legacy `media` feature or FFmpeg.
use crate::playback_native::{NativeReader, RawFrame};
use crate::{Result, invalid};
use std::{fs::File, io::BufReader, path::Path, time::Duration};

#[derive(Debug)]
pub struct DecodeStats {
    pub backend: &'static str,
    pub video_frames: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    pub decode_errors: u64,
}

/// Decode and discard video frames without converting them to RGB.
/// Errors are propagated, never retried through a foreign decoder. Platform
/// decoder features do not override the owned software codec selection.
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_video_interval(source, None)
}

/// Decode frames whose presentation start belongs to the half-open interval.
/// Reference frames before the interval are decoded but not counted. Timing
/// comparisons retain source-clock precision; no rounded frame rate is used.
pub fn decode_video_interval(
    source: &Path,
    interval: Option<(Duration, Duration)>,
) -> Result<DecodeStats> {
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("decode interval requires from < to"));
    }
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let mut stats = DecodeStats {
        backend: if reader.hardware_accelerated() {
            "videotoolbox"
        } else {
            "fvid"
        },
        video_frames: 0,
        width: 0,
        height: 0,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    while let Some(frame) = reader.read_frame_raw()? {
        if let Some((from, to)) = interval {
            let (start, _, scale) = reader
                .frame_interval()
                .ok_or_else(|| invalid("missing frame timestamp"))?;
            let stamp = start
                .checked_mul(1_000_000_000)
                .ok_or_else(|| invalid("frame timestamp overflow"))?;
            let from = from
                .as_nanos()
                .checked_mul(u128::from(scale))
                .ok_or_else(|| invalid("interval overflow"))?;
            let to = to
                .as_nanos()
                .checked_mul(u128::from(scale))
                .ok_or_else(|| invalid("interval overflow"))?;
            if stamp >= to {
                break;
            }
            if stamp < from {
                continue;
            }
        }
        let [width, height] = reader.dimensions();
        stats.width = u32::try_from(width).map_err(|_| invalid("video width overflow"))?;
        stats.height = u32::try_from(height).map_err(|_| invalid("video height overflow"))?;
        stats.pixel_format = match &frame {
            RawFrame::Rgb(_) => "rgb24".into(),
            RawFrame::Avc { picture, .. } => match picture.bit_depth {
                8 => "yuv420p".into(),
                depth => format!("yuv420p{depth}le"),
            },
            RawFrame::Planar8(p) => planar_format(width, height, p.chroma_width, p.chroma_height)?,
            RawFrame::Yuv { sx, sy, .. } => planar_format(width, height, width / sx, height / sy)?,
        };
        stats.video_frames = stats
            .video_frames
            .checked_add(1)
            .ok_or_else(|| invalid("frame count overflow"))?;
    }
    if stats.video_frames == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    Ok(stats)
}
fn planar_format(w: usize, h: usize, cw: usize, ch: usize) -> Result<String> {
    Ok(if cw == w && ch == h {
        "yuv444p"
    } else if cw == w.div_ceil(2) && ch == h {
        "yuv422p"
    } else if cw == w.div_ceil(2) && ch == h.div_ceil(2) {
        "yuv420p"
    } else {
        return Err(invalid("unrecognized decoded plane geometry"));
    }
    .into())
}
