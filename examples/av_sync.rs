//! Measure how closely the audio clock tracks video presentation, window-free.
//! Usage: av_sync INPUT [FRAMES] [START_SECONDS]
//!
//! Runs the real video decode thread and the real audio decode thread over the
//! same file, presenting frames on the wall-clock cadence the player uses while
//! the audio thread feeds a silent modelled device. For each presented frame it
//! records `audio clock - frame PTS`, which is exactly the quantity the
//! player's sync gate compares: how far inside the slack the two tracks stay,
//! and whether they drift apart over a long run.
mod harness;

use fvid::playback_native::NativeReader;
use fvid::playback_thread::{Event, Frame, Playback, startup_buffer};
use harness::{Counters, Device};
use std::fs::File;
use std::io::BufReader;
use std::time::{Duration, Instant};

/// Frame presentation time in seconds, from its track timescale.
fn seconds(pts: Option<(i64, u32)>) -> Option<f64> {
    let (value, scale) = pts?;
    (scale > 0).then(|| value as f64 / f64::from(scale))
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let path = args
        .first()
        .and_then(|p| p.to_str())
        .ok_or("usage: av_sync INPUT [FRAMES] [START_SECONDS]")?;
    let count: usize = args
        .get(1)
        .and_then(|s| s.to_str())
        .unwrap_or("0")
        .parse()?;
    let seek: f64 = args
        .get(2)
        .and_then(|s| s.to_str())
        .unwrap_or("0")
        .parse()?;

    let mut reader = NativeReader::without_memory_limit(BufReader::new(File::open(path)?))?;
    if seek > 0.0 {
        reader.seek(Duration::from_secs_f64(seek))?;
    } else {
        reader.read_frame()?;
    }
    let period = reader.frame_period();
    let video = Playback::start(reader, None);

    let Some(stream) = harness::open_stream(path) else {
        return Err("no audio track this player can decode".into());
    };
    let counters = Counters::default();
    let device = counters.clone();
    let mut audio =
        fvid::audio_thread::AudioPlayback::start(stream, move || Box::new(Device::new(device)));
    if seek > 0.0 {
        audio.seek(Duration::from_secs_f64(seek));
    }

    // The player holds back a short run of frames before it starts presenting;
    // without it the first frames all look like video running ahead.
    std::thread::sleep(startup_buffer(period));
    // ...and it starts the audio clock at that same instant, which is the
    // anchoring this measures: a clock that started with the video thread
    // instead runs ahead by the preroll for the whole file.
    audio.play();

    let mut measured = Measured::default();
    let mut deadline = Instant::now();
    let slack = fvid::player::AUDIO_SYNC_SLACK;
    'present: loop {
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        let frame = loop {
            match video.poll() {
                Some(Event::Frame(frame)) => break frame,
                Some(Event::Ended(_)) => break 'present,
                Some(Event::Error(error)) => return Err(error.into()),
                None => std::thread::sleep(Duration::from_micros(200)),
            }
        };
        deadline = (deadline + frame.period).max(Instant::now());
        present(frame, &audio, slack, &mut measured);
        if count > 0 && measured.skew.len() >= count {
            break;
        }
    }

    let skew = &measured.skew;
    let frames = skew.len();
    if frames == 0 {
        return Err("no frame met an advancing audio clock".into());
    }
    let heard = audio.position().as_secs_f64();
    let (lag, lead) = (
        skew.iter().fold(f64::INFINITY, |a, &b| a.min(b)),
        skew.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b)),
    );
    println!("frames={frames} seek={seek}s period={period:?} slack={slack:.3}s");
    println!(
        "skew mean={:+.3}s min={:+.3}s max={:+.3}s",
        mean(skew),
        lag,
        lead,
    );
    // Drift is what a fixed offset cannot hide: the first and last thirds of the
    // run sit on different clocks if the two tracks disagree about rate.
    let third = (frames / 3).max(1);
    println!(
        "drift={:+.4}s gated={} frozen={} heard={heard:.3}s fed={:.3}s dropped={:.3}s",
        mean(&skew[frames - third..]) - mean(&skew[..third]),
        measured.gated,
        measured.frozen,
        counters.fed(),
        counters.dropped(),
    );
    // The whole run has to sit inside the budget the player's gate allows, or
    // the gate is either freezing pictures or excusing a visible offset.
    println!(
        "verdict={}",
        if lag >= -slack && lead <= slack {
            "every frame inside the slack"
        } else {
            "outside the slack"
        }
    );
    Ok(())
}

/// What a run measured: per-frame skew plus the readings that were dropped.
#[derive(Default)]
struct Measured {
    skew: Vec<f64>,
    /// Frames the player's gate would have held back.
    gated: usize,
    /// Readings taken while the audio clock stood still. The worker stops
    /// sampling the device once the track runs out, so the tail of a run reports
    /// where the last queued samples left off while video still has frames to
    /// show. Those are not disagreements between the tracks, and including them
    /// would make the end of a file look like an offset.
    frozen: usize,
    /// Clock reading behind the last sample, to spot one that has stopped.
    last: Option<Duration>,
}

/// Compare one presented frame with the audio clock and count the frames the
/// player's gate would have held back.
fn present(
    frame: Frame,
    audio: &fvid::audio_thread::AudioPlayback,
    slack: f64,
    measured: &mut Measured,
) {
    let Some(at) = seconds(frame.pts) else {
        return;
    };
    let heard = audio.position();
    if measured.last.is_some_and(|previous| heard <= previous) {
        measured.frozen += 1;
        return;
    }
    measured.last = Some(heard);
    let error = heard.as_secs_f64() - at;
    if error < -slack {
        measured.gated += 1;
    }
    measured.skew.push(error);
}
