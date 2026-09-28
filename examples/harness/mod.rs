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

/// One reader of the chain: it either hands over a track or says why this file is
/// not the kind it reads.
type Opener = fn(&str) -> fvid::Result<Box<dyn AudioStream>>;

fn track_of<R: AudioStream + 'static>(reader: R) -> Box<dyn AudioStream> {
    Box::new(reader)
}

fn open_mp4(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_mp4_audio::Mp4AudioReader::open(
        BufReader::new(file),
        fvid::container::mp4::Limits::default(),
    )?))
}

fn open_webm(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_webm_audio::WebmAudioReader::open(
        BufReader::new(file),
        fvid::container::webm::Limits::default(),
    )?))
}

/// An AVI file states one stream's geometry in two headers and repeats it again in
/// an index beside the run, so its reader cross-checks all three and refuses a file
/// whose own accounts disagree; it frames records for the codings whose counting it
/// has measured, and names the Wave format number of the rest.
fn open_avi(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_avi_audio::AviAudioReader::open(
        BufReader::new(file),
        fvid::container::avi::Limits::default(),
    )?))
}

fn open_smf(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_smf::SmfAudioReader::open(
        BufReader::new(file),
        fvid::container::smf::Limits::default(),
    )?))
}

fn open_xm(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_xm::XmAudioReader::open(
        BufReader::new(file),
        fvid::container::xm::Limits::default(),
    )?))
}

/// An Ogg file states nothing about its codec in its pages, so this reader walks
/// the framing and reads the coding out of the first bytes of the first packet:
/// Vorbis has an arm, and every other bitstream is refused by the name it opened
/// with.
fn open_ogg(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_ogg_audio::OggAudioReader::open(
        BufReader::new(file),
        fvid::container::ogg::Limits::default(),
    )?))
}

/// A Wave file first of the three that state nothing but a geometry: its parser is
/// the strictest, since a header whose redundant fields disagree is refused rather
/// than guessed at.
fn open_wav(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_wav::WavAudioReader::open(
        BufReader::new(file),
        fvid::playback_wav::Limits::default(),
    )?))
}

/// An AIFF file states each number of its geometry once, in big-endian, and spells
/// its rate as an 80-bit extended number.
fn open_aiff(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_aiff::AiffAudioReader::open(
        BufReader::new(file),
        fvid::playback_aiff::Limits::default(),
    )?))
}

/// A Sun header last of the readers that state nothing but a geometry: six words and
/// a run, and an encoding table whose numbers above 3 the readers of the format
/// disagree about, so only what this one names plays out of it.
fn open_au(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_au::AuAudioReader::open(
        BufReader::new(file),
        fvid::playback_au::Limits::default(),
    )?))
}

/// A `.flac` file states its geometry in a block and its frames carry no length, so
/// every packet boundary is proved by the frame's own CRC-16.
fn open_flac(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_flac::FlacAudioReader::open(
        BufReader::new(file),
        fvid::playback_flac::Limits::default(),
    )?))
}

/// A bare MPEG audio file states nothing up front at all: its geometry comes from the
/// first frame that carries audio, one frame after an `Info` header the demuxer eats,
/// and every packet is a whole frame with its own header. Having no magic to look for,
/// it goes last, after every reader that begins with something that says what the
/// file is.
fn open_mp3(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_mp3::Mp3AudioReader::open(
        BufReader::new(file),
        fvid::playback_mp3::Limits::default(),
    )?))
}

/// A bare AC-3 file states nothing before its frames: each syncframe carries its own
/// geometry, so the reader walks the stream the way the MPEG walker does.
fn open_ac3(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_ac3::Ac3AudioReader::open(
        BufReader::new(file),
        fvid::playback_ac3::Limits::default(),
    )?))
}

/// A bare ADTS AAC file: the frame headers state geometry and no setup block exists,
/// so the reader builds the `esds` the container-shaped decoder asks for. Last,
/// because its syncword is the loosest of the walkers.
fn open_aac(path: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let file = File::open(path)?;
    Ok(track_of(fvid::playback_aac::AacAudioReader::open(
        BufReader::new(file),
        fvid::playback_aac::Limits::default(),
    )?))
}

/// Every audio reader the player has, in the order it tries them.
const OPENERS: &[Opener] = &[
    open_mp4, open_webm, open_avi, open_smf, open_xm, open_ogg, open_wav, open_aiff, open_au,
    open_flac, open_mp3, open_ac3, open_aac,
];

/// Open the first audio track the player can decode, from MP4, WebM or AVI, a MIDI
/// performance, a tracker module, an Ogg file, a Wave, AIFF or Sun file, a bare
/// FLAC file, or a bare MPEG audio file.
pub fn open_stream(path: &str) -> Option<Box<dyn AudioStream>> {
    open_stream_reason(path).0
}

/// The same walk, and when it finds no track the half of the pipeline that is
/// missing: `Some(coding)` is a reader that understood the envelope and refused the
/// coding it names for want of a decode arm, `None` that no reader recognised the
/// envelope at all. The two are different queues of work, so a caller that reports a
/// refusal has to say which one it hit.
pub fn open_stream_reason(path: &str) -> (Option<Box<dyn AudioStream>>, Option<String>) {
    let mut refused = None;
    for open in OPENERS {
        match open(path) {
            Ok(stream) => return (Some(stream), None),
            Err(fvid::Error::Unsupported(coding)) => {
                if refused.is_none() {
                    refused = Some(coding);
                }
            }
            Err(_) => {}
        }
    }
    (None, refused)
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
        self.base
            .checked_add(Duration::from_secs_f64(seconds))
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
