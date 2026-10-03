//! Decode a file's audio track and report what the playback pipeline sees.
//! Usage: audio_probe [--play] [--pcm FILE] [--packets COUNT] INPUT
//!
//! `--pcm` appends the decoded interleaved f32 bytes to FILE, which is how a
//! decoder's output gets compared byte for byte with a reference renderer of the
//! same track.
//!
//! Without `--play` this only decodes, so it needs no audio device. With it the
//! file runs through the real audio thread and a silent modelled device, which
//! is how the queue pacing gets checked: an unpaced worker drains the whole
//! track into the backend ring in milliseconds instead of taking the track's
//! own duration.
mod harness;

use harness::{Counters, Device};
use std::fs::File;
use std::io::Write;
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
    let playback =
        fvid::audio_thread::AudioPlayback::start(stream, move || Box::new(Device::new(device)));
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

/// Decode the requested packet prefix and validate presented PCM without a device.
fn probe(
    path: &str,
    pcm_to: Option<&str>,
    packet_limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let (found, refused): (Option<Box<dyn fvid::audio::AudioStream>>, Option<String>) =
        if std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| {
                ["mp4", "mov", "m4a"]
                    .iter()
                    .any(|e| s.eq_ignore_ascii_case(e))
            })
        {
            (
                Some(Box::new(fvid::playback_mp4_audio::Mp4AudioReader::open(
                    std::io::BufReader::new(File::open(path)?),
                    fvid::container::mp4::Limits::default(),
                )?)),
                None,
            )
        } else if std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| {
                ["webm", "mkv", "mka"]
                    .iter()
                    .any(|e| s.eq_ignore_ascii_case(e))
            })
        {
            (
                Some(Box::new(fvid::playback_webm_audio::WebmAudioReader::open(
                    std::io::BufReader::new(File::open(path)?),
                    fvid::container::webm::Limits::default(),
                )?)),
                None,
            )
        } else {
            harness::open_stream_reason(path)
        };
    let Some(mut stream) = found else {
        // Which half of the pipeline is missing is the answer, not that one of them
        // is: a container no reader opens and a container whose coding has no arm
        // are two different queues of work.
        return Err(match refused {
            Some(coding) => {
                format!("player read the container, refused its audio coding: {coding}").into()
            }
            None => "no reader recognised this envelope".into(),
        });
    };
    let mut dump = pcm_to.map(File::create).transpose()?;

    let codec = stream.codec().to_owned();
    let sample_rate = stream.sample_rate();
    let channels = u64::from(stream.channels());
    let mut decoder = match fvid::codec::make_audio_decoder(
        &codec,
        stream.extra_data(),
        sample_rate,
        channels as u16,
        stream.bits_per_sample(),
    ) {
        Ok(decoder) => decoder,
        // The container and its track are read at this point, so a refusal here is
        // the dispatch having no arm for the tag the container named. The tag goes
        // out with it, because that is the row this file proves or does not.
        Err(error) => {
            return Err(format!(
                "player read the container and a track coded {codec}, and has no decoder arm for it: {error}"
            )
            .into());
        }
    };

    let mut encoded = 0usize;
    let mut decoded = 0usize;
    let mut frames = 0u64;
    let mut stamps = 0i64;
    let mut first = None;
    let mut last = None;

    while let Some(packet) = stream.next_packet()? {
        encoded += 1;
        stamps += packet.duration.max(0);
        let at = stream.time_of(packet.pts);
        first = first.or(Some(at));
        last = Some(at);
        let Some(mut pcm) = decoder.decode_encoded(
            &packet.data,
            packet.pts.max(0) as u64,
            packet.duration.max(0) as u64,
        )?
        else {
            continue;
        };
        if let Some(limit) = stream.packet_sample_limit(packet.duration.max(0) as u64)? {
            let bytes = limit
                .checked_mul(channels as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or("audio sample window overflow")?;
            if bytes > pcm.data.len() {
                return Err("audio presentation window exceeds decoded PCM".into());
            }
            pcm.data.truncate(bytes);
        }
        let Some(pcm) = stream.present_decoded(pcm, packet.pts)? else {
            continue;
        };
        if channels == 0 || !pcm.data.len().is_multiple_of(channels as usize * 4) {
            return Err("invalid PCM geometry".into());
        }
        if pcm
            .data
            .chunks_exact(4)
            .any(|s| !f32::from_le_bytes(s.try_into().unwrap()).is_finite())
        {
            return Err("non-finite PCM sample".into());
        }
        decoded += 1;
        frames += (pcm.data.len() as u64 / 4) / channels;
        if let Some(file) = &mut dump {
            file.write_all(&pcm.data)?;
        }
        if encoded >= packet_limit {
            break;
        }
    }

    let seconds = frames as f64 / f64::from(sample_rate);
    if frames == 0 {
        return Err("no presented audio samples".into());
    }
    println!("codec={codec} rate={sample_rate} channels={channels}");
    println!("packets={encoded} decoded={decoded} frames={frames}");
    // What the container's own stamps add up to, beside what the decoder handed
    // over: the two agreeing is the track being as long as its timeline claims.
    let timescale = stream.timescale().max(1);
    println!("stamped={:.3}s", stamps as f64 / f64::from(timescale));
    println!(
        "audio={seconds:.3}s first={:?} last={:?}",
        first.unwrap_or_default(),
        last.unwrap_or_default()
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut play = false;
    let mut check_device = false;
    let mut pcm = None;
    let mut path = None;
    let mut packet_limit = usize::MAX;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--play" => play = true,
            "--check-device" => check_device = true,
            "--pcm" => pcm = Some(args.next().ok_or("--pcm needs a path")?),
            "--packets" => packet_limit = args.next().ok_or("--packets needs a count")?.parse()?,
            _ if path.is_none() => path = Some(arg),
            other => return Err(format!("unexpected argument: {other}").into()),
        }
    }
    let path = path.ok_or("usage: audio_probe [--play] [--pcm FILE] [--packets COUNT] INPUT")?;
    if check_device {
        use fvid::audio::AudioBackend;
        let stream = harness::open_stream(&path).ok_or("no decodable audio track")?;
        let mut backend = fvid::audio::PlatformBackend::new();
        backend.start(fvid::audio::AudioSpec {
            sample_rate: stream.sample_rate(),
            channels: stream.channels(),
            format: fvid::audio::SampleFormat::F32,
        })?;
        backend.stop()?;
        println!(
            "device=ready source_channels={} rate={} no PCM sent",
            stream.channels(),
            stream.sample_rate()
        );
        Ok(())
    } else if play {
        play_through_device(&path)
    } else {
        probe(&path, pcm.as_deref(), packet_limit)
    }
}
