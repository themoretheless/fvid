//! Audio playback backend abstraction.
//!
//! The audio pipeline mirrors the video one: a background thread decodes
//! compressed packets into PCM samples, which are handed to a platform
//! backend for playback. The backend is responsible for clocking samples to
//! the audio hardware and keeping the playback position available for A/V
//! sync.

#[cfg(feature = "player")]
mod cpal_backend;

#[cfg(feature = "player")]
pub use cpal_backend::CpalBackend as PlatformBackend;

use std::time::Duration;

/// PCM sample format produced by decoders and consumed by backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Signed 16-bit little-endian interleaved samples.
    I16,
    /// Signed 32-bit little-endian interleaved samples.
    I32,
    /// 32-bit IEEE 754 little-endian interleaved samples.
    F32,
}

/// Description of a decoded audio stream.
#[derive(Clone, Copy, Debug)]
pub struct AudioSpec {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of interleaved channels.
    pub channels: u16,
    /// Sample format.
    pub format: SampleFormat,
}

impl AudioSpec {
    /// Bytes per interleaved frame (one sample per channel).
    pub fn frame_size(&self) -> usize {
        let sample = match self.format {
            SampleFormat::I16 => 2,
            SampleFormat::I32 | SampleFormat::F32 => 4,
        };
        sample * self.channels as usize
    }
}

/// A decoded audio packet ready for playback.
pub struct AudioPacket {
    /// PCM samples, interleaved by channel.
    pub data: Vec<u8>,
    /// Presentation timestamp in stream timebase units.
    pub pts: u64,
    /// Stream timebase: `pts * timebase_num / timebase_den` seconds.
    pub timebase_num: u32,
    pub timebase_den: u32,
}

impl AudioPacket {
    /// Presentation timestamp as a `Duration` from the stream start.
    pub fn presentation_time(&self) -> Duration {
        if self.timebase_den == 0 {
            return Duration::ZERO;
        }
        let nanos = (self.pts as u128)
            .saturating_mul(self.timebase_num as u128)
            .saturating_mul(1_000_000_000)
            / (self.timebase_den as u128);
        Duration::from_nanos(nanos as u64)
    }
}

/// Errors that can occur during audio playback.
#[derive(Debug)]
pub enum AudioError {
    /// The backend failed to initialize (device unavailable, format unsupported).
    InitFailed(String),
    /// The backend encountered an error during playback.
    PlaybackError(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::InitFailed(msg) => write!(f, "audio init failed: {msg}"),
            AudioError::PlaybackError(msg) => write!(f, "audio playback error: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Platform audio backend. Implementations manage the audio device and buffer
/// packets internally; the caller pushes packets as they arrive from the
/// decoder and queries the current playback position for sync.
pub trait AudioBackend {
    /// Open the device for the given stream description. Called once before
    /// any packets are pushed. The device does not have to clock samples yet —
    /// `resume` starts the clock, which is how a caller keeps the audio clock
    /// anchored to the moment it starts presenting.
    fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError>;

    /// Push a decoded packet for playback. The backend takes ownership and
    /// schedules the samples according to the packet's presentation time.
    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError>;

    /// Current playback position on the stream timeline. Used for
    /// A/V sync: the video presentation waits until the audio clock has
    /// reached the frame's PTS.
    fn position(&self) -> Duration;

    /// Discard all buffered packets and re-anchor the clock at `at`. Called
    /// after a seek or rewind, so `position` keeps reporting stream time
    /// rather than the elapsed time since the buffer was drained.
    fn flush(&mut self, at: Duration) -> Result<(), AudioError>;

    /// Pause playback. The backend keeps its buffers but stops clocking
    /// samples.
    fn pause(&mut self) -> Result<(), AudioError>;

    /// Run the device clock from its current position. This is the first start
    /// as well as every restart after a pause: between `start` and here the
    /// clock has not advanced at all.
    fn resume(&mut self) -> Result<(), AudioError>;

    /// Stop playback and release the audio device. The backend may be
    /// restarted with a new `start` call.
    fn stop(&mut self) -> Result<(), AudioError>;
}

/// A no-op backend used when audio is disabled or unsupported. All operations
/// succeed immediately; `position` follows the pushed packets and the flushes
/// exactly, so it mirrors what a real device clock would report.
pub struct NullBackend {
    position: Duration,
}

impl Default for NullBackend {
    fn default() -> Self {
        Self {
            position: Duration::ZERO,
        }
    }
}

impl AudioBackend for NullBackend {
    fn start(&mut self, _: AudioSpec) -> Result<(), AudioError> {
        Ok(())
    }

    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
        self.position = packet.presentation_time();
        Ok(())
    }

    fn position(&self) -> Duration {
        self.position
    }

    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.position = at;
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
}

/// One encoded audio access unit read from a container. Timestamps stay in the
/// source's own timescale, which the stream reports separately.
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub duration: i64,
}

/// Container-side audio source: stream metadata plus a cursor over encoded
/// packets that can be rewound and seeked. Implemented per container so the
/// decode thread stays independent of both the demultiplexer and the codec.
pub trait AudioStream: Send {
    /// Codec tag, e.g. `mp4a` or `A_VORBIS`.
    fn codec(&self) -> &str;
    /// Ticks per second used by packet timestamps.
    fn timescale(&self) -> u32;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Codec setup data, already in the form the decoder expects.
    fn extra_data(&self) -> &[u8];
    /// Next encoded packet, or `None` at end of stream.
    fn next_packet(&mut self) -> crate::Result<Option<EncodedPacket>>;
    fn rewind(&mut self);
    /// Move the cursor to the last packet at or before `pts` and report the
    /// timestamp actually landed on.
    fn seek_to(&mut self, pts: i64) -> i64;

    /// Wall-clock time of a packet timestamp.
    fn time_of(&self, pts: i64) -> Duration {
        let timescale = self.timescale();
        if timescale == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(pts.max(0) as f64 / f64::from(timescale))
    }
}

/// Decoder half of the audio pipeline: turn encoded packets into PCM.
pub trait AudioDecode: Send {
    /// Decode one access unit. `Ok(None)` means the decoder consumed the packet
    /// without producing output, so the caller should feed it another one.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>>;

    /// Drop codec state, e.g. after a seek.
    fn reset(&mut self);
}
