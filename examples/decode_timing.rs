//! Time native decoding of the first frames of a file, without a window.
use std::{fs::File, io::BufReader, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        return Err("usage: decode_timing INPUT [FRAMES]".into());
    }
    let limit: usize = args
        .get(1)
        .and_then(|a| a.to_str())
        .and_then(|a| a.parse().ok())
        .unwrap_or(60);
    let mut reader =
        fvid::playback_native::NativeReader::without_memory_limit(BufReader::new(File::open(&args[0])?))?;
    // Two passes: the first also pays for cold file reads, the second is warm.
    for pass in 0..2 {
        if pass == 1 {
            reader.rewind()?;
        }
        let start = Instant::now();
        let mut frames = 0;
        // FNV-1a over every RGB byte, to compare decoder changes for exactness.
        let mut checksum = 0xcbf2_9ce4_8422_2325u64;
        // FVID_CHECKSUM=1 hashes every frame (and prints each on pass 0); it
        // costs several ms per frame, so timing runs leave it unset.
        let hashing = std::env::var_os("FVID_CHECKSUM").is_some();
        while frames < limit && reader.read_frame()? {
            frames += 1;
            if !hashing {
                continue;
            }
            let mut frame_sum = 0xcbf2_9ce4_8422_2325u64;
            for &byte in reader.rgb() {
                frame_sum = (frame_sum ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
            checksum = (checksum ^ frame_sum).wrapping_mul(0x0100_0000_01b3);
            if pass == 0 {
                println!("frame {frames}: {frame_sum:016x}");
            }
        }
        let elapsed = start.elapsed();
        let [w, h] = reader.dimensions();
        println!(
            "pass {pass}: {w}x{h} period={:?} frames={frames} total={:?} per_frame={:?} checksum={checksum:016x}",
            reader.frame_period(),
            elapsed,
            elapsed / frames.max(1) as u32
        );
    }
    // Optional third argument: seek to that many seconds and report where we land.
    if let Some(secs) = args.get(2).and_then(|a| a.to_str()).and_then(|a| a.parse::<f64>().ok()) {
        let start = Instant::now();
        reader.seek(std::time::Duration::from_secs_f64(secs))?;
        let (begin, end, scale) = reader.frame_interval().ok_or("no frame after seek")?;
        println!(
            "seek to {secs}s landed on [{:.3}s, {:.3}s) in {:?}",
            begin as f64 / f64::from(scale),
            end as f64 / f64::from(scale),
            start.elapsed()
        );
    }
    // The player's pipeline (decode thread + conversion thread): frames per
    // second as the window would receive them.
    {
        let mut reader = fvid::playback_native::NativeReader::without_memory_limit(
            BufReader::new(File::open(&args[0])?),
        )?;
        reader.read_frame()?;
        let playback = fvid::playback_thread::Playback::start(reader);
        let start = Instant::now();
        let mut frames = 0;
        while frames < limit {
            match playback.poll() {
                Some(fvid::playback_thread::Event::Frame(_)) => frames += 1,
                Some(fvid::playback_thread::Event::Ended(_)) => break,
                Some(fvid::playback_thread::Event::Error(error)) => return Err(error.into()),
                None => std::thread::sleep(std::time::Duration::from_micros(200)),
            }
        }
        let elapsed = start.elapsed();
        println!(
            "pipeline: frames={frames} total={:?} per_frame={:?}",
            elapsed,
            elapsed / frames.max(1) as u32
        );
    }
    // Raw AVC decode only, when the input is MP4.
    if let Ok(mut source) = fvid::playback_mp4::Mp4AvcReader::open(
        BufReader::new(File::open(&args[0])?),
        fvid::container::mp4::Limits::default(),
        usize::MAX,
    ) {
        let start = Instant::now();
        let mut frames = 0;
        while frames < limit && source.read_frame()?.is_some() {
            frames += 1;
        }
        let elapsed = start.elapsed();
        println!(
            "avc only: frames={frames} total={:?} per_frame={:?}",
            elapsed,
            elapsed / frames.max(1) as u32
        );
    }
    Ok(())
}
