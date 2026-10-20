//! Check edited playback without saving pictures or copying the source file.
use fvid::playback_native::NativeReader;
use std::{io::BufReader, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).peekable();
    let path = args
        .next()
        .ok_or("usage: playback_edit_probe INPUT [SEEK_SECONDS ...]")?;
    let started = std::time::Instant::now();
    let spool = fvid::playback_spool::Spool::adaptive(std::path::Path::new(&path))?;
    let source_cache = spool.handle();
    eprintln!("source_open_ms={} source_cache_ram={}", started.elapsed().as_millis(), source_cache.is_memory());
    let mut reader = NativeReader::without_memory_limit(BufReader::with_capacity(
        fvid::playback_spool::READ_AHEAD, spool))?;
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    reader.enable_shared_surfaces()?;
    eprintln!("index_open_ms={}", started.elapsed().as_millis());
    let verify_continuation = args.peek().is_some_and(|arg| arg == "--warm-seek-cache");
    if verify_continuation {
        args.next();
        reader.warm_seeking(std::path::Path::new(&path));
        let mut reported = 0;
        while !reader.seek_frame_cache_complete() {
            if started.elapsed() > Duration::from_secs(600) { return Err("seek cache preparation timed out".into()); }
            let frames = reader.seek_frame_cache_count();
            if frames / 300 > reported {
                reported = frames / 300;
                eprintln!("prepared_frames={frames} elapsed={:?}", started.elapsed());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("seek_cache_ready frames={} cache={:?} elapsed={:?}", reader.seek_frame_cache_count(), reader.seek_cache_stats(), started.elapsed());
    }
    let count = if args.peek().is_some_and(|arg| arg == "--all") {
        args.next();
        usize::MAX
    } else {
        30
    };
    let mut decoded = 0;
    let mut last_interval = None;
    let mut max_frame_wait = Duration::ZERO;
    let mut waits_over_second = 0;
    for _ in 0..count {
        let frame_started = std::time::Instant::now();
        if reader.read_frame_raw()?.is_none() {
            break;
        }
        let wait = frame_started.elapsed();
        max_frame_wait = max_frame_wait.max(wait);
        waits_over_second += usize::from(wait > Duration::from_secs(1));
        last_interval = reader.frame_interval();
        decoded += 1;
        if decoded % 300 == 0 { eprintln!("decoded_frames={decoded} elapsed_seconds={:.3}", started.elapsed().as_secs_f64()); }
    }
    if decoded == 0 {
        return Err("no decoded frames".into());
    }
    println!("initial_frames={decoded} duration={:?} last_interval={last_interval:?} max_frame_wait={max_frame_wait:?} waits_over_second={waits_over_second} total_elapsed={:?}", reader.duration(), started.elapsed());
    for arg in args {
        let seconds: f64 = arg.to_str().ok_or("non-UTF8 seek")?.parse()?;
        let target = Duration::from_secs_f64(seconds);
        let seek_started = std::time::Instant::now();
        eprintln!("seek_start={seconds}");
        if reader.seek_raw(target)?.is_none() {
            return Err("seek returned no picture".into());
        }
        let (start, end, scale) = reader.frame_interval().ok_or("missing interval")?;
        let ticks = target.as_nanos() * u128::from(scale);
        // A source can legitimately start video after time zero (audio already
        // runs). Seeking zero returns its first picture rather than an error.
        if (start * 1_000_000_000 > ticks && !target.is_zero()) || ticks >= end * 1_000_000_000 {
            return Err(format!(
                "seek {seconds}: picture interval {start}..{end}/{scale} does not contain target"
            )
            .into());
        }
        println!("seek={seconds} interval={start}..{end}/{scale} elapsed_ms={:.3} cache={:?} cached_frames={}", seek_started.elapsed().as_secs_f64() * 1000.0, reader.seek_cache_stats(), reader.seek_frame_cache_count());
        if let Some(status) = reader.seek_cache_status() {
            if let Some(total) = reader.duration() {
                let ranges = status.ranges(total);
                if verify_continuation && ranges != vec![[0.0, 1.0]] {
                    return Err("prepared RAM coverage changed after seek".into());
                }
                println!("line_ranges_after={seconds} memory={ranges:?} source_ram={} source_byte_ranges={:?} disk_line_filled={}",
                    source_cache.is_memory(), source_cache.ranges(), !source_cache.is_memory() && !source_cache.ranges().is_empty());
            }
        }
        if verify_continuation {
            let mut previous = start;
            let mut worst = Duration::ZERO;
            for _ in 0..10 {
                let began = std::time::Instant::now();
                if reader.read_frame_raw()?.is_none() { return Err("cached continuation ended early".into()); }
                worst = worst.max(began.elapsed());
                let (at, _, _) = reader.frame_interval().ok_or("continuation has no timestamp")?;
                if at <= previous { return Err("cached continuation repeated or reordered a picture".into()); }
                previous = at;
            }
            println!("continuation_after={seconds} frames=10 max_ms={:.3}", worst.as_secs_f64() * 1000.0);
        }
    }
    reader.rewind()?;
    if reader.read_frame_raw()?.is_none() {
        return Err("rewind returned no picture".into());
    }
    println!("rewind=ok");
    Ok(())
}
