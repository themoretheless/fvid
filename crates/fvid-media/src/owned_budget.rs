//! Shared memory-policy helpers, without demuxer or codec dependencies.
use fvid_control::CopyOptions;
type Result<T> = std::result::Result<T, String>;
/// Process memory probe in bytes. Linux/Windows report current RSS;
/// macOS retains the existing getrusage high-water-mark policy.
pub fn process_rss_bytes() -> Option<u64> {
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

pub fn check_rss_budget(options: &CopyOptions) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_rss_and_memory_parsers_keep_the_existing_policy() {
        check_rss_budget(&CopyOptions::default()).unwrap();
        let zero = CopyOptions {
            max_rss_bytes: Some(0),
            ..Default::default()
        };
        assert!(check_rss_budget(&zero).is_err());
        let unlimited = CopyOptions {
            max_rss_bytes: Some(u64::MAX),
            ..Default::default()
        };
        if let Some(rss) = process_rss_bytes() {
            assert!(rss > 0);
            check_rss_budget(&unlimited).unwrap();
        } else {
            assert!(check_rss_budget(&unlimited)
                .unwrap_err()
                .contains("unavailable"));
        }
        assert_eq!(parse_max_memory_mib("1").unwrap(), 1024 * 1024);
        assert_eq!(parse_max_rss_mib("65536").unwrap(), 65536 * 1024 * 1024);
        for invalid in ["0", "4097", "bad"] {
            assert!(parse_max_memory_mib(invalid).is_err());
        }
        for invalid in ["0", "65537", "bad"] {
            assert!(parse_max_rss_mib(invalid).is_err());
        }
    }
}

/// Conservative admission estimate for the current owned WAVE export pipeline.
/// Counts retained input, output vector growth/quantization, sinc history,
/// INFO rewrite buffers and fixed I/O/stack scratch. This is not process RSS.
pub fn estimate_float_wave_export_bytes(
    info: &crate::owned_wave_inspect::WaveInfo,
    input_bytes: usize,
    metadata_bytes: usize,
    transform: fvid_media_info::AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<usize> {
    if !(if info.float {
        matches!(info.bits_per_sample, 32 | 64)
    } else {
        matches!(info.bits_per_sample, 8 | 16 | 24 | 32)
    }) || !(1..=64).contains(&info.channels)
        || info.sample_rate == 0
    {
        return Err("invalid WAVE memory geometry".into());
    }
    let width = u128::from(info.bits_per_sample / 8);
    let frame_bytes = u128::from(info.channels) * width;
    if u128::from(info.block) != frame_bytes || input_bytes as u128 % frame_bytes != 0 {
        return Err("partial WAVE frame in memory estimate".into());
    }
    let rate = transform.sample_rate.unwrap_or(info.sample_rate as i32);
    let channels = transform.channels.unwrap_or(i32::from(info.channels));
    if rate <= 0 || !(1..=64).contains(&channels) {
        return Err("invalid output memory geometry".into());
    }
    let frames = input_bytes as u128 / frame_bytes;
    let output_frames = (frames * rate as u128).div_ceil(u128::from(info.sample_rate));
    let dsp_width = if info.float { width } else { 8 };
    let output_bytes = output_frames * channels as u128 * dsp_width;
    let history = if rate as u32 == info.sample_rate {
        0
    } else {
        // +1 radius protects against the floating-point ceil in the filter;
        // the deque grows geometrically and may transiently retain old storage.
        let radius = (32 * u128::from(info.sample_rate))
            .div_ceil(rate as u128)
            .max(32)
            + 1;
        let step = u128::from(info.sample_rate).div_ceil(rate as u128);
        let held = frames.min(2 * radius + step + 2);
        let slots = held
            .checked_next_power_of_two()
            .ok_or("sinc history estimate overflow")?
            .max(4);
        2 * slots * 64 * dsp_width
    };
    let retains_metadata = !info.float
        || info.bits_per_sample == 64
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty();
    let metadata = if retains_metadata {
        let assignments = options
            .metadata_set
            .iter()
            .try_fold(0u128, |sum, (_, value)| {
                sum.checked_add(value.len() as u128 + 10)
                    .ok_or("metadata estimate overflow")
            })?;
        8 * (metadata_bytes as u128 + assignments + 12)
    } else {
        0
    };
    let normalization_scratch = if info.float {
        0
    } else {
        frames.min(4096) * u128::from(info.channels) * 8
    };
    let total = input_bytes as u128
        + 3 * output_bytes
        + history
        + metadata
        + normalization_scratch
        + 16 * 1024;
    usize::try_from(total).map_err(|_| "controlled memory estimate exceeds address space".into())
}

#[cfg(test)]
mod wave_estimate_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn admission_covers_observed_sinc_history_and_output_capacities() {
        for bits in [32u16, 64] {
            for (rate, out) in [
                (48000, 48000),
                (48000, 16000),
                (44100, 48000),
                (96000, 8000),
            ] {
                let width = usize::from(bits / 8);
                let frames = 257;
                let info = crate::owned_wave_inspect::WaveInfo {
                    sample_rate: rate,
                    channels: 2,
                    bits_per_sample: bits,
                    float: true,
                    valid_bits: bits,
                    channel_mask: 3,
                    data_offset: 44,
                    sample_frames: frames,
                    block: (2 * width) as u16,
                    data_bytes: (frames as usize * 2 * width) as u32,
                    end: 44 + frames * 2 * width as u64,
                };
                let input_bytes = frames as usize * 2 * width;
                let transform = fvid_media_info::AudioDecodeTransform {
                    sample_rate: Some(out as i32),
                    ..Default::default()
                };
                let estimate = estimate_float_wave_export_bytes(
                    &info,
                    input_bytes,
                    0,
                    transform,
                    &CopyOptions::default(),
                )
                .unwrap();
                if bits == 32 {
                    let mut engine =
                        crate::owned_resample::Resampler::new(Vec::new(), rate, out, 2).unwrap();
                    for _ in 0..frames {
                        engine
                            .write_all(&[0.25f32.to_le_bytes(), 0.5f32.to_le_bytes()].concat())
                            .unwrap();
                        assert!(input_bytes + engine.retained_storage_bytes() <= estimate);
                    }
                    engine.finish().unwrap();
                    assert!(input_bytes + engine.retained_storage_bytes() <= estimate);
                } else {
                    let mut engine =
                        crate::owned_resample_f64::Resampler::new(Vec::new(), rate, out, 2)
                            .unwrap();
                    for _ in 0..frames {
                        engine
                            .write_all(&[0.25f64.to_le_bytes(), 0.5f64.to_le_bytes()].concat())
                            .unwrap();
                        assert!(input_bytes + engine.retained_storage_bytes() <= estimate);
                    }
                    engine.finish().unwrap();
                    assert!(input_bytes + engine.retained_storage_bytes() <= estimate);
                }
                let with_tags = estimate_float_wave_export_bytes(
                    &info,
                    input_bytes,
                    100,
                    transform,
                    &CopyOptions {
                        metadata_set: vec![("title".into(), "new".into())],
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(with_tags > estimate);
            }
        }
    }
}
