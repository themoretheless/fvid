//! Decode a file's audio track and report what the playback pipeline sees.
//! Usage: audio_probe [--play] INPUT
//!
//! Without `--play` this only decodes, so it needs no audio device. With it the
//! file runs through the real audio thread and a silent modelled device, which
//! is how the queue pacing gets checked: an unpaced worker drains the whole
//! track into the backend ring in milliseconds instead of taking the track's
//! own duration.
mod harness;

use harness::{Counters, Device};
use std::time::{Duration, Instant};

/// Start the audio thread against the modelled device and wait for it to
/// finish, then compare wall time with what the device played and discarded.
fn play_through_device(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let Some(stream) = harness::open_stream(path) else {
        return Err("no audio track this player can decode".into());
    };
    let started = Instant::now();
    let counters = Counters::default();
    let device = counters.clone();
    let playback = fvid::audio_thread::AudioPlayback::start(stream, move || {
        Box::new(Device::new(device))
    });
    // The thread opens the track paused, so nothing runs until commanded —
    // the same hand the player gives it when it starts presenting.
    playback.play();
    loop {
        match playback.poll() {
            Some(fvid::audio_thread::AudioEvent::Ended(_)) => break,
            Some(fvid::audio_thread::AudioEvent::Error(error)) => return Err(error.into()),
            Some(fvid::audio_thread::AudioEvent::Started) => {}
            None => {
                if started.elapsed() > Duration::from_secs(120) {
                    return Err("audio thread made no progress".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    let wall = started.elapsed();
    let fed = counters.fed();
    let dropped = counters.dropped();
    let heard = playback.position();
    println!(
        "wall={:.3}s fed={fed:.3}s heard={:.3}s dropped={dropped:.3}s",
        wall.as_secs_f64(),
        heard.as_secs_f64(),
    );
    // What the thread keeps queued ahead of the device: its pacing target plus
    // one frame, since the frontier is measured at the end of the last packet
    // queued. An unpaced thread runs to the ring's capacity and drops the rest
    // of the track.
    println!(
        "lead={:.3}s {}",
        fed - heard.as_secs_f64(),
        if dropped <= 0.001 {
            "queue never overflowed"
        } else {
            "thread outran the ring"
        }
    );
    Ok(())
}

/// Decode every packet and report the counts, without a device in the loop.
fn probe(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut stream) = harness::open_stream(path) else {
        return Err("no audio track this player can decode".into());
    };

    let codec = stream.codec().to_owned();
    let sample_rate = stream.sample_rate();
    let channels = u64::from(stream.channels());
    let mut decoder =
        fvid::codec::make_audio_decoder(&codec, stream.extra_data(), sample_rate, channels as u16)?;

    let mut encoded = 0usize;
    let mut decoded = 0usize;
    let mut frames = 0u64;
    let mut first = None;
    let mut last = None;

    while let Some(packet) = stream.next_packet()? {
        encoded += 1;
        let at = stream.time_of(packet.pts);
        first = first.or(Some(at));
        last = Some(at);
        let Some(pcm) = decoder.decode_encoded(
            &packet.data,
            packet.pts.max(0) as u64,
            packet.duration.max(0) as u64,
        )?
        else {
            continue;
        };
        decoded += 1;
        frames += (pcm.data.len() as u64 / 4) / channels;
    }

    let seconds = frames as f64 / f64::from(sample_rate);
    println!("codec={codec} rate={sample_rate} channels={channels}");
    println!("packets={encoded} decoded={decoded} frames={frames}");
    println!(
        "audio={seconds:.3}s first={:?} last={:?}",
        first.unwrap_or_default(),
        last.unwrap_or_default()
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut play = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--play" => play = true,
            _ if path.is_none() => path = Some(arg),
            other => return Err(format!("unexpected argument: {other}").into()),
        }
    }
    let path = path.ok_or("usage: audio_probe [--play] INPUT")?;
    if play {
        play_through_device(&path)
    } else {
        probe(&path)
    }
}
