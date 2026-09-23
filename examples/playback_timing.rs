//! Measure the player's decode/conversion pipeline, optionally at source cadence.
//! Usage: playback_timing INPUT [FRAMES] [START_SECONDS] [paced]
use fvid::playback_native::NativeReader;
use fvid::playback_thread::{Event, Playback};
use std::{
    fs::File,
    io::BufReader,
    thread,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = args
        .first()
        .ok_or("usage: playback_timing INPUT [FRAMES] [START_SECONDS] [paced]")?;
    let count: usize = args
        .get(1)
        .and_then(|s| s.to_str())
        .unwrap_or("600")
        .parse()?;
    let seek: f64 = args
        .get(2)
        .and_then(|s| s.to_str())
        .unwrap_or("0")
        .parse()?;
    let paced = args.get(3).is_some_and(|s| s == "paced");
    let mut reader = NativeReader::without_memory_limit(BufReader::new(File::open(path)?))?;
    if seek > 0.0 {
        reader.seek(Duration::from_secs_f64(seek))?;
    } else {
        reader.read_frame()?;
    }
    let period = reader.frame_period();
    let playback = Playback::start(reader);
    if paced {
        thread::sleep(fvid::playback_thread::startup_buffer(period));
    }
    let start = Instant::now();
    let mut deadline = start;
    let mut stalls = Vec::new();
    let mut frames = 0;
    while frames < count {
        if paced {
            thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
        let waiting = Instant::now();
        let frame_period = loop {
            match playback.poll() {
                Some(Event::Frame(frame)) => break frame.period,
                Some(Event::Error(error)) => return Err(error.into()),
                Some(Event::Ended(_)) => return Err("unexpected end of video".into()),
                None => thread::sleep(Duration::from_micros(100)),
            }
        };
        let wait = waiting.elapsed();
        if paced && wait > Duration::from_millis(1) {
            stalls.push(wait);
        }
        frames += 1;
        deadline = (deadline + frame_period).max(Instant::now());
    }
    let elapsed = start.elapsed();
    println!(
        "frames={frames} start={seek}s paced={paced} elapsed={elapsed:?} fps={:.2} per_frame={:?} waits_over_1ms={} max_wait={:?}",
        frames as f64 / elapsed.as_secs_f64(),
        elapsed / frames.max(1) as u32,
        stalls.len(),
        stalls.iter().max().copied().unwrap_or_default()
    );
    Ok(())
}
