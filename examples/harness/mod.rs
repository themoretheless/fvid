//! Shared scaffolding for the playback examples.
//!
//! [`open_stream`] gets an audio cursor out of whichever container the file
//! uses, and [`Device`] stands in for an output device that plays nothing. It
//! keeps the two properties that decide whether the decode thread behaves: it
//! clocks samples at wall-clock rate while it runs, and it buffers only a
//! ring's worth, refusing what does not fit. So an unpaced thread shows up as
//! discarded audio rather than needing headphones.
use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec, AudioStream};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Open the first audio track the player can decode, from MP4 or WebM.
pub fn open_stream(path: &str) -> Option<Box<dyn AudioStream>> {
    let mp4 = File::open(path)
        .ok()
        .and_then(|file| {
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                BufReader::new(file),
                fvid::container::mp4::Limits::default(),
            )
            .ok()
        })
        .map(|reader| Box::new(reader) as Box<dyn AudioStream>);
    mp4.or_else(|| {
        File::open(path)
            .ok()
            .and_then(|file| {
                fvid::playback_webm_audio::WebmAudioReader::open(
                    BufReader::new(file),
                    fvid::container::webm::Limits::default(),
                )
                .ok()
            })
            .map(|reader| Box::new(reader) as Box<dyn AudioStream>)
    })
}

/// Frame counts the device accumulates. Shared with the caller because the
/// device is built on, and never leaves, the audio thread.
#[derive(Clone, Default)]
pub struct Counters {
    fed: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
}

impl Counters {
    /// Seconds of audio the device accepted.
    pub fn fed(&self) -> f64 {
        self.seconds(self.fed.load(Ordering::Relaxed))
    }

    /// Seconds of audio it refused because the ring was already full.
    pub fn dropped(&self) -> f64 {
        self.seconds(self.dropped.load(Ordering::Relaxed))
    }

    fn seconds(&self, frames: u64) -> f64 {
        let rate = self.sample_rate.load(Ordering::Relaxed).max(1);
        frames as f64 / f64::from(rate)
    }
}

/// A device that plays nothing: samples leave it only as fast as the clock
/// advances, and no more than `queue_frames` of them wait inside. Like the cpal
/// backend it opens stopped and only clocks once `resume` runs it, so a thread
/// that never gets a play command shows up as audio that never advances rather
/// than as a device that ran ahead of the picture.
pub struct Device {
    /// When `played` and `queued` were last reconciled with the wall clock.
    checkpoint: Instant,
    running: bool,
    /// Stream time the clock was last re-anchored at, plus the frames played
    /// since: together they are what a real device reports as its position.
    base: Duration,
    sample_rate: u32,
    frame_size: usize,
    /// Frames played since `base`.
    played: u64,
    /// Frames accepted but not yet played.
    queued: u64,
    queue_frames: u64,
    counters: Counters,
}

impl Device {
    pub fn new(counters: Counters) -> Self {
        Self {
            checkpoint: Instant::now(),
            running: false,
            base: Duration::ZERO,
            sample_rate: 48_000,
            frame_size: 4,
            played: 0,
            queued: 0,
            // The cpal backend's ring: about half a second.
            queue_frames: 24_000,
            counters,
        }
    }

    /// Frames the device would have played given `runnable` frames worth of
    /// running time since the last reconciliation, and what its ring would hold.
    /// Consumption stops at what it was given, so a starved run shifts every
    /// later sample instead of being made up.
    fn consumed(&self, runnable: u64) -> (u64, u64) {
        let play = runnable.min(self.queued);
        (self.played + play, self.queued - play)
    }

    /// Frames worth of running time since the last reconciliation.
    fn runnable(&self) -> u64 {
        if !self.running {
            return 0;
        }
        (self.checkpoint.elapsed().as_secs_f64() * f64::from(self.sample_rate)) as u64
    }

    /// Let the device play whatever running time has allowed.
    fn advance(&mut self) {
        let (played, queued) = self.consumed(self.runnable());
        self.played = played;
        self.queued = queued;
        self.checkpoint = Instant::now();
    }
}

impl AudioBackend for Device {
    fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
        self.sample_rate = spec.sample_rate;
        self.frame_size = spec.frame_size();
        self.queue_frames = u64::from(spec.sample_rate) / 2;
        self.checkpoint = Instant::now();
        self.running = false;
        self.base = Duration::ZERO;
        self.played = 0;
        self.queued = 0;
        self.counters
            .sample_rate
            .store(spec.sample_rate, Ordering::Relaxed);
        Ok(())
    }

    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
        self.advance();
        let frames = u64::try_from(packet.data.len() / self.frame_size.max(1)).unwrap_or(0);
        let room = self.queue_frames.saturating_sub(self.queued);
        let kept = frames.min(room);
        self.queued += kept;
        self.counters.fed.fetch_add(kept, Ordering::Relaxed);
        self.counters
            .dropped
            .fetch_add(frames - kept, Ordering::Relaxed);
        Ok(())
    }

    /// The clock runs on its own, like a device's callback thread, so reading it
    /// reconciles without waiting for the decode thread to push again. Otherwise
    /// the last packets of a track would freeze the reported position short of
    /// where the device really is.
    fn position(&self) -> Duration {
        let (played, _) = self.consumed(self.runnable());
        let seconds = played as f64 / f64::from(self.sample_rate.max(1));
        self.base.checked_add(Duration::from_secs_f64(seconds))
            .unwrap_or(self.base)
    }

    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.queued = 0;
        self.played = 0;
        self.base = at;
        self.checkpoint = Instant::now();
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        if self.running {
            self.advance();
            self.running = false;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        if !self.running {
            self.checkpoint = Instant::now();
            self.running = true;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
}
