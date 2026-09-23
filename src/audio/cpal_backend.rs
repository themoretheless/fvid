//! Cross-platform audio backend using cpal.
//!
//! The backend opens the default output device, configures it for the stream's
//! sample rate and channel count, and runs a callback that fills the audio
//! buffer from a ring of decoded samples. The ring absorbs jitter between the
//! decoder thread and the audio hardware clock.

use super::{AudioBackend, AudioError, AudioPacket, AudioSpec, SampleFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use std::sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}};
use std::time::Duration;

/// Ring buffer of decoded samples. Sized for ~500 ms of 48 kHz stereo f32
/// (about 192 KB) — enough to absorb decoder jitter without noticeable
/// latency.
const RING_FRAMES: usize = 24_000;

struct Ring {
    /// Interleaved f32 samples. Length is `RING_FRAMES * channels`.
    buffer: Vec<f32>,
    /// Number of channels.
    channels: usize,
    /// Write position in frames (decoder advances).
    write_pos: u64,
    /// Read position in frames (audio callback advances).
    read_pos: AtomicU64,
}

impl Ring {
    fn new(channels: usize) -> Self {
        Self {
            buffer: vec![0.0; RING_FRAMES * channels],
            channels,
            write_pos: 0,
            read_pos: AtomicU64::new(0),
        }
    }

    fn readable(&self) -> usize {
        let w = self.write_pos;
        let r = self.read_pos.load(Ordering::Acquire);
        (w.saturating_sub(r) as usize).min(RING_FRAMES)
    }

    fn writable(&self) -> usize {
        RING_FRAMES - self.readable()
    }

    /// Write interleaved f32 samples. Returns frames written.
    fn write(&mut self, samples: &[f32]) -> usize {
        let frames = samples.len() / self.channels;
        let available = self.writable();
        let to_write = frames.min(available);
        if to_write == 0 {
            return 0;
        }
        let write_idx = (self.write_pos as usize % RING_FRAMES) * self.channels;
        let end_idx = write_idx + to_write * self.channels;
        if end_idx <= self.buffer.len() {
            self.buffer[write_idx..end_idx]
                .copy_from_slice(&samples[..to_write * self.channels]);
        } else {
            let split = self.buffer.len() - write_idx;
            self.buffer[write_idx..].copy_from_slice(&samples[..split * self.channels]);
            self.buffer[..(to_write - split) * self.channels]
                .copy_from_slice(&samples[split * self.channels..to_write * self.channels]);
        }
        self.write_pos += to_write as u64;
        to_write
    }

    /// Read interleaved f32 samples. Returns frames read.
    fn read(&self, output: &mut [f32]) -> usize {
        let frames = output.len() / self.channels;
        let available = self.readable();
        let to_read = frames.min(available);
        if to_read == 0 {
            output.fill(0.0);
            return 0;
        }
        let read_idx = (self.read_pos.load(Ordering::Acquire) as usize % RING_FRAMES) * self.channels;
        let end_idx = read_idx + to_read * self.channels;
        if end_idx <= self.buffer.len() {
            output[..to_read * self.channels]
                .copy_from_slice(&self.buffer[read_idx..end_idx]);
        } else {
            let split = self.buffer.len() - read_idx;
            output[..split * self.channels].copy_from_slice(&self.buffer[read_idx..]);
            output[split * self.channels..to_read * self.channels]
                .copy_from_slice(&self.buffer[..(to_read - split) * self.channels]);
        }
        if to_read < frames {
            output[to_read * self.channels..].fill(0.0);
        }
        self.read_pos.fetch_add(to_read as u64, Ordering::Release);
        to_read
    }

    fn flush(&mut self) {
        self.write_pos = 0;
        self.read_pos.store(0, Ordering::Release);
        self.buffer.fill(0.0);
    }

    fn position_frames(&self) -> u64 {
        self.read_pos.load(Ordering::Acquire)
    }
}

struct Shared {
    ring: Mutex<Ring>,
}

pub struct CpalBackend {
    shared: Arc<Shared>,
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
    /// Stream time the ring's first frame corresponds to, set by `flush`.
    base: Duration,
}

impl CpalBackend {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                ring: Mutex::new(Ring::new(2)),
            }),
            stream: None,
            sample_rate: 48_000,
            channels: 2,
            base: Duration::ZERO,
        }
    }

    fn create_stream(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| AudioError::InitFailed("no output device".into()))?;

        let config = StreamConfig {
            channels: spec.channels as cpal::ChannelCount,
            sample_rate: cpal::SampleRate(spec.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let shared = self.shared.clone();
        let err_fn = |err| eprintln!("audio stream error: {err}");

        let stream = match spec.format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let ring = shared.ring.lock().unwrap();
                    ring.read(data);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let ring = shared.ring.lock().unwrap();
                    let mut f32_buf = vec![0.0; data.len()];
                    ring.read(&mut f32_buf);
                    for (i, s) in f32_buf.iter().enumerate() {
                        data[i] = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => device.build_output_stream(
                &config,
                move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                    let ring = shared.ring.lock().unwrap();
                    let mut f32_buf = vec![0.0; data.len()];
                    ring.read(&mut f32_buf);
                    for (i, s) in f32_buf.iter().enumerate() {
                        data[i] = (s.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                },
                err_fn,
                None,
            ),
        }.map_err(|e| AudioError::InitFailed(format!("build stream: {e}")))?;

        self.stream = Some(stream);
        self.sample_rate = spec.sample_rate;
        self.channels = spec.channels;
        Ok(())
    }
}

impl Drop for CpalBackend {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
    }
}

impl AudioBackend for CpalBackend {
    fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
        {
            let mut ring = self.shared.ring.lock().unwrap();
            *ring = Ring::new(spec.channels as usize);
        }
        // Built but left stopped: the device starts clocking at `resume`, so
        // opening a file cannot advance the audio clock before the caller
        // starts presenting pictures against it.
        self.create_stream(spec)
    }

    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
        let samples = match packet.data.len() {
            0 => return Ok(()),
            _ => {
                let f32_samples: Vec<f32> = packet
                    .data
                    .chunks_exact(4)
                    .map(|chunk| {
                        f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                    })
                    .collect();
                f32_samples
            }
        };
        let mut ring = self.shared.ring.lock().unwrap();
        ring.write(&samples);
        Ok(())
    }

    fn position(&self) -> Duration {
        let ring = self.shared.ring.lock().unwrap();
        let frames = ring.position_frames();
        let elapsed = frames as f64 / self.sample_rate as f64;
        self.base + Duration::from_secs_f64(elapsed)
    }

    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        {
            let mut ring = self.shared.ring.lock().unwrap();
            ring.flush();
        }
        self.base = at;
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = &self.stream {
            stream.pause().map_err(|e| {
                AudioError::PlaybackError(format!("pause: {e}"))
            })?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = &self.stream {
            stream.play().map_err(|e| {
                AudioError::PlaybackError(format!("play: {e}"))
            })?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
        Ok(())
    }
}
