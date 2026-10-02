//! Controlled DPB/scratch admission and optional process RSS probes.
//! Estimates are upper bounds on Fvid-owned + typical decoder/encoder frame pools;
//! they are not a promise of peak OS RSS from libav alone.

use crate::{CopyOptions, Input, Result, check, ffi::*, selection};
use std::path::Path;

/// Probe interval for optional RSS checks during packet loops.
pub(crate) const RSS_CHECK_EVERY: u64 = 256;

/// Bytes for one decoded frame buffer of `format` at `width`×`height`.
pub(crate) fn frame_buffer_bytes(width: i32, height: i32, format: AVPixelFormat) -> Result<usize> {
    if width <= 0 || height <= 0 {
        return Err("invalid frame geometry for memory estimate".into());
    }
    let layout = match format {
        AVPixelFormat_AV_PIX_FMT_YUV420P
        | AVPixelFormat_AV_PIX_FMT_NV12
        | AVPixelFormat_AV_PIX_FMT_NV21 => Some((Some((1, 1)), 1)),
        AVPixelFormat_AV_PIX_FMT_YUV422P => Some((Some((1, 0)), 1)),
        AVPixelFormat_AV_PIX_FMT_YUV444P => Some((Some((0, 0)), 1)),
        AVPixelFormat_AV_PIX_FMT_GRAY8 => Some((None, 1)),
        AVPixelFormat_AV_PIX_FMT_YUV420P10LE
        | AVPixelFormat_AV_PIX_FMT_YUV420P10BE
        | AVPixelFormat_AV_PIX_FMT_YUV420P12LE
        | AVPixelFormat_AV_PIX_FMT_YUV420P12BE
        | AVPixelFormat_AV_PIX_FMT_YUV420P16LE
        | AVPixelFormat_AV_PIX_FMT_YUV420P16BE => Some((Some((1, 1)), 2)),
        AVPixelFormat_AV_PIX_FMT_YUV422P10LE
        | AVPixelFormat_AV_PIX_FMT_YUV422P10BE
        | AVPixelFormat_AV_PIX_FMT_YUV422P12LE
        | AVPixelFormat_AV_PIX_FMT_YUV422P12BE
        | AVPixelFormat_AV_PIX_FMT_YUV422P16LE
        | AVPixelFormat_AV_PIX_FMT_YUV422P16BE => Some((Some((1, 0)), 2)),
        AVPixelFormat_AV_PIX_FMT_YUV444P10LE
        | AVPixelFormat_AV_PIX_FMT_YUV444P10BE
        | AVPixelFormat_AV_PIX_FMT_YUV444P12LE
        | AVPixelFormat_AV_PIX_FMT_YUV444P12BE
        | AVPixelFormat_AV_PIX_FMT_YUV444P16LE
        | AVPixelFormat_AV_PIX_FMT_YUV444P16BE => Some((Some((0, 0)), 2)),
        AVPixelFormat_AV_PIX_FMT_GRAY16LE | AVPixelFormat_AV_PIX_FMT_GRAY16BE => Some((None, 2)),
        AVPixelFormat_AV_PIX_FMT_RGB24 | AVPixelFormat_AV_PIX_FMT_BGR24 => Some((None, 3)),
        AVPixelFormat_AV_PIX_FMT_RGBA | AVPixelFormat_AV_PIX_FMT_BGRA => Some((None, 4)),
        _ => None,
    };
    if let Some((chroma, sample_bytes)) = layout {
        return crate::owned_frame_layout::planar_bytes(
            width as usize,
            height as usize,
            chroma,
            sample_bytes,
        );
    }
    let size = unsafe { av_image_get_buffer_size(format, width, height, 1) };
    check(size, "estimate frame buffer size")?;
    Ok(size as usize)
}

/// Decoder DPB frames implied by `video_delay`, capped for admission.
pub(crate) fn dpb_frame_count(video_delay: i32) -> usize {
    let delay = video_delay.max(0) as usize;
    (delay + 1).clamp(1, 32)
}

/// Conservative controlled-byte estimate for a software decode (+ optional encode scratch).
///
/// Counts decoder DPB (`video_delay+1`), a matching encoder hold, and explicit Fvid
/// scratch frames (decoded + filter outputs + optional hflip ring).
pub(crate) fn estimate_decode_controlled_bytes(
    width: i32,
    height: i32,
    format: AVPixelFormat,
    video_delay: i32,
    scratch_frames: usize,
) -> Result<usize> {
    let frame = frame_buffer_bytes(width, height, format)?;
    let dpb = dpb_frame_count(video_delay);
    // Encoder may retain a similar reorder/lookahead depth for libx264-style codecs;
    // FFV1 is lighter, but admission stays conservative.
    let encoder_hold = dpb;
    let total_frames = dpb
        .checked_add(encoder_hold)
        .and_then(|n| n.checked_add(scratch_frames.max(1)))
        .ok_or("controlled frame count overflow")?;
    total_frames
        .checked_mul(frame)
        .ok_or_else(|| "controlled memory estimate overflow".into())
}

/// Estimate from selected input stream parameters before opening codecs.
///
/// When `decode` is false (remux/trim/concat), only the reusable packet payload
/// budget counts as controlled hold. When true, each selected video stream adds
/// an estimated decoder DPB + encoder hold + Fvid scratch.
pub(crate) fn estimate_input_decode_controlled_bytes(
    input: &Input,
    options: &CopyOptions,
    scratch_frames: usize,
    decode: bool,
) -> Result<usize> {
    if !decode {
        return Ok(options.max_packet_bytes);
    }
    let selected = selection(input, options)?;
    let mut total = 0usize;
    let mut saw_video = false;
    for &index in &selected {
        // SAFETY: selection indices are in-range; codecpar is owned by Input.
        unsafe {
            let p = &*(*input.streams()[index]).codecpar;
            if p.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO {
                continue;
            }
            saw_video = true;
            let format = if p.format == AVPixelFormat_AV_PIX_FMT_NONE {
                // Container may omit format until decode; assume 8-bit 4:2:0 upper bound.
                AVPixelFormat_AV_PIX_FMT_YUV420P
            } else {
                p.format
            };
            let part = estimate_decode_controlled_bytes(
                p.width,
                p.height,
                format,
                p.video_delay,
                scratch_frames,
            )?;
            total = total
                .checked_add(part)
                .ok_or("controlled memory estimate overflow")?;
        }
    }
    if !saw_video {
        return Ok(options.max_packet_bytes);
    }
    Ok(total)
}

pub(crate) fn admit_controlled_budget(options: &CopyOptions, estimated: usize) -> Result<()> {
    if let Some(max) = options.max_controlled_bytes
        && estimated > max
    {
        return Err(format!(
            "controlled memory budget exceeded: need {estimated} bytes, limit {max}"
        ));
    }
    Ok(())
}

pub(crate) fn admit_input_controlled_budget(
    input: &Input,
    options: &CopyOptions,
    scratch_frames: usize,
    decode: bool,
) -> Result<()> {
    if options.max_controlled_bytes.is_none() {
        return Ok(());
    }
    let estimated = estimate_input_decode_controlled_bytes(input, options, scratch_frames, decode)?;
    admit_controlled_budget(options, estimated)
}

pub(crate) use crate::owned_budget::check_rss_budget;

/// Shared packet-loop budget: cancel, packet count, periodic RSS, and progress.
pub(crate) fn check_budget(options: &CopyOptions, packets: u64) -> Result<()> {
    check_budget_with_bytes(options, packets, 0)
}

pub(crate) fn check_budget_with_bytes(
    options: &CopyOptions,
    packets: u64,
    payload_bytes: u64,
) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(crate::CancelFlag::is_cancelled)
    {
        return Err("operation cancelled".into());
    }
    if let Some(max) = options.max_packets
        && packets >= max
    {
        return Err("packet budget exceeded".into());
    }
    if packets % RSS_CHECK_EVERY == 0 {
        check_rss_budget(options)?;
        if let Some(hook) = &options.progress {
            hook.emit(crate::ProgressEvent {
                packets,
                payload_bytes,
                done: false,
            });
        }
    }
    Ok(())
}

pub(crate) fn emit_progress_done(options: &CopyOptions, packets: u64, payload_bytes: u64) {
    if let Some(hook) = &options.progress {
        hook.emit(crate::ProgressEvent {
            packets,
            payload_bytes,
            done: true,
        });
    }
}

pub use crate::owned_budget::{parse_max_memory_mib, parse_max_rss_mib};

/// Convenience for callers that only have a path (probe-open once).
pub fn estimate_path_controlled_bytes(
    source: &Path,
    options: &CopyOptions,
    scratch_frames: usize,
) -> Result<usize> {
    let input = Input::open_fast(source)?;
    estimate_input_decode_controlled_bytes(&input, options, scratch_frames, true)
}
