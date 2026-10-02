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
            assert!(
                check_rss_budget(&unlimited)
                    .unwrap_err()
                    .contains("unavailable")
            );
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
