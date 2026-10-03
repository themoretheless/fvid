//! Check edited playback without saving pictures or copying the source file.
use fvid::playback_native::NativeReader;
use std::{fs::File, io::BufReader, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).peekable();
    let path = args
        .next()
        .ok_or("usage: playback_edit_probe INPUT [SEEK_SECONDS ...]")?;
    let mut reader = NativeReader::without_memory_limit(BufReader::new(File::open(path)?))?;
    let count = if args.peek().is_some_and(|arg| arg == "--all") {
        args.next();
        usize::MAX
    } else {
        30
    };
    let mut decoded = 0;
    for _ in 0..count {
        if reader.read_frame_raw()?.is_none() {
            break;
        }
        decoded += 1;
    }
    if decoded == 0 {
        return Err("no decoded frames".into());
    }
    println!("initial_frames={decoded} duration={:?}", reader.duration());
    for arg in args {
        let seconds: f64 = arg.to_str().ok_or("non-UTF8 seek")?.parse()?;
        let target = Duration::from_secs_f64(seconds);
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
        println!("seek={seconds} interval={start}..{end}/{scale}");
    }
    reader.rewind()?;
    if reader.read_frame_raw()?.is_none() {
        return Err("rewind returned no picture".into());
    }
    println!("rewind=ok");
    Ok(())
}
