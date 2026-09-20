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
    let estimated =
        estimate_input_decode_controlled_bytes(input, options, scratch_frames, decode)?;
    admit_controlled_budget(options, estimated)
}

/// Current process RSS in bytes when the platform probe is available.
pub(crate) fn process_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut core::ffi::c_void;
            fn K32GetProcessMemoryInfo(
                process: *mut core::ffi::c_void,
                ppsmem_counters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }
        unsafe {
            let mut counters = core::mem::zeroed::<ProcessMemoryCounters>();
            counters.cb = core::mem::size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) == 0 {
                return None;
            }
            Some(counters.working_set_size as u64)
        }
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb.saturating_mul(1024));
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let mut usage = unsafe { core::mem::zeroed::<libc::rusage>() };
        let code = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if code != 0 {
            return None;
        }
        Some(usage.ru_maxrss as u64)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

pub(crate) fn check_rss_budget(options: &CopyOptions) -> Result<()> {
    let Some(max) = options.max_rss_bytes else {
        return Ok(());
    };
    let Some(rss) = process_rss_bytes() else {
        return Err("rss probe unavailable on this platform".into());
    };
    if rss > max {
        return Err(format!(
            "rss budget exceeded: process uses {rss} bytes, limit {max}"
        ));
    }
    Ok(())
}

/// Shared packet-loop budget: cancel, packet count, periodic RSS, and progress.
pub(crate) fn check_budget(options: &CopyOptions, packets: u64) -> Result<()> {
    check_budget_with_bytes(options, packets, 0)
}

pub(crate) fn check_budget_with_bytes(
    options: &CopyOptions,
    packets: u64,
    payload_bytes: u64,
) -> Result<()> {
    if options.cancel.as_ref().is_some_and(crate::CancelFlag::is_cancelled) {
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

#[allow(dead_code)] // exercised via CLI/validate; keep helper for API callers
pub fn mib_to_bytes(mib: u64) -> Result<u64> {
    mib.checked_mul(1024 * 1024)
        .ok_or_else(|| "memory MiB overflow".into())
}

pub fn parse_max_memory_mib(raw: &str) -> Result<usize> {
    let mib: u64 = raw.parse().map_err(|_| "invalid max-memory-mib")?;
    if mib == 0 || mib > 4096 {
        return Err("max-memory-mib must be 1..=4096".into());
    }
    (mib as usize)
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "max-memory-mib overflow".into())
}

pub fn parse_max_rss_mib(raw: &str) -> Result<u64> {
    let mib: u64 = raw.parse().map_err(|_| "invalid max-rss-mib")?;
    if mib == 0 || mib > 65536 {
        return Err("max-rss-mib must be 1..=65536".into());
    }
    mib_to_bytes(mib)
}

/// Convenience for callers that only have a path (probe-open once).
pub fn estimate_path_controlled_bytes(
    source: &Path,
    options: &CopyOptions,
    scratch_frames: usize,
) -> Result<usize> {
    let input = Input::open_fast(source)?;
    estimate_input_decode_controlled_bytes(&input, options, scratch_frames, true)
}
