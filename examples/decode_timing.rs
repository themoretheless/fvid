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
    let start = Instant::now();
    let mut frames = 0;
    while frames < limit && reader.read_frame()? {
        frames += 1;
    }
    let elapsed = start.elapsed();
    let [w, h] = reader.dimensions();
    println!(
        "{w}x{h} period={:?} frames={frames} total={:?} per_frame={:?}",
        reader.frame_period(),
        elapsed,
        elapsed / frames.max(1) as u32
    );
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
