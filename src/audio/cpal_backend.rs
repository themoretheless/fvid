//! Cross-platform audio backend using cpal.
//!
//! The backend opens the default or explicitly selected output device, configures it for the stream's
//! sample rate and channel count, and runs a callback that fills the audio
//! buffer from a ring of decoded samples. The ring absorbs jitter between the
//! decoder thread and the audio hardware clock.

use super::{
    AudioBackend, AudioError, AudioPacket, AudioSpec, SampleFormat, Stretch, apply_volume_frame,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, AtomicU64, Ordering},
};
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
    /// Pitch-preserving reader of a rate other than the recorded one, made the
    /// first time one is asked for and forgotten again by `flush`.
    stretch: Option<Stretch>,
    /// Ring frame that the stretch counts its own frames from.
    stretch_base: u64,
    /// Contiguous copy of the window last handed to the stretch, reused between
    /// callbacks so the device thread stops allocating.
    window: Vec<f32>,
}

impl Ring {
    fn new(channels: usize) -> Self {
        Self {
            buffer: vec![0.0; RING_FRAMES * channels],
            channels,
            write_pos: 0,
            read_pos: AtomicU64::new(0),
            stretch: None,
            stretch_base: 0,
            window: Vec::new(),
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
            self.buffer[write_idx..end_idx].copy_from_slice(&samples[..to_write * self.channels]);
        } else {
            // The room left at the end is counted in frames, not elements: the
            // ring is interleaved, and an element count would overrun the slice
            // for every stream with more than one channel.
            let head = (self.buffer.len() - write_idx) / self.channels;
            self.buffer[write_idx..].copy_from_slice(&samples[..head * self.channels]);
            let tail = to_write - head;
            self.buffer[..tail * self.channels]
                .copy_from_slice(&samples[head * self.channels..to_write * self.channels]);
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
        let read_idx =
            (self.read_pos.load(Ordering::Acquire) as usize % RING_FRAMES) * self.channels;
        let end_idx = read_idx + to_read * self.channels;
        if end_idx <= self.buffer.len() {
            output[..to_read * self.channels].copy_from_slice(&self.buffer[read_idx..end_idx]);
        } else {
            let head = (self.buffer.len() - read_idx) / self.channels;
            output[..head * self.channels].copy_from_slice(&self.buffer[read_idx..]);
            output[head * self.channels..to_read * self.channels]
                .copy_from_slice(&self.buffer[..(to_read - head) * self.channels]);
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
        // The next packet starts a fresh stream frame, and a stale stretch would
        // fade the last segment of the old position into it.
        self.stretch = None;
        self.stretch_base = 0;
    }

    fn position_frames(&self) -> u64 {
        self.read_pos.load(Ordering::Acquire)
    }

    /// Fill `output` by stretching the stream to `milli`/1000 of its recorded
    /// rate without moving its pitch, and move the read position to what the
    /// stretch retired. Returns the elements written.
    ///
    /// The stretch is given a contiguous window of the ring because a circular
    /// buffer would ask it to search the same correlation across a wrap, and it
    /// is kept between callbacks because the segments have to overlap where the
    /// last one ended.
    fn read_stretched(&mut self, output: &mut [f32], milli: u32) -> usize {
        let channels = self.channels;
        let device_frames = output.len() / channels;
        if self.stretch.is_none() {
            self.stretch_base = self.read_pos.load(Ordering::Acquire);
            self.stretch = Some(Stretch::new(channels, milli));
        }
        let given = {
            let stretch = self.stretch.as_mut().expect("made just above");
            stretch.set_rate(milli);
            // How far the stretch wants to search, capped by what the decoder has
            // handed over: a short window costs the caller the segments it cannot
            // cut, not the ones it can.
            (stretch.lookahead(device_frames) as u64).min(self.readable() as u64)
        };
        if given == 0 {
            output.fill(0.0);
            return 0;
        }
        // The ring is circular and the stretch searches one window for the place
        // the wave fits, which would cross the join twice; hand it a copy.
        let read = self.read_pos.load(Ordering::Acquire);
        let start = (read as usize % RING_FRAMES) * channels;
        let elements = (given as usize) * channels;
        let head = elements.min(self.buffer.len() - start);
        self.window.resize(elements, 0.0);
        self.window[..head].copy_from_slice(&self.buffer[start..start + head]);
        if head < elements {
            self.window[head..].copy_from_slice(&self.buffer[..elements - head]);
        }
        // The ring's frame `read` stands at the stretch's own frame counted from
        // where it first took over, which is where it retired to last time.
        let base = (read - self.stretch_base) as usize;
        let stretch = self.stretch.as_mut().expect("made just above");
        let written = stretch.fill(base, &self.window, output);
        let retired = stretch.retired() as u64;
        output[written..].fill(0.0);
        self.read_pos
            .store(self.stretch_base + retired, Ordering::Release);
        written
    }
}

struct Shared {
    ring: Mutex<Ring>,
    /// Output gain in thousandths, read by the device callback.
    volume: AtomicU32,
    /// Stream frames consumed per device frame, in thousandths.
    rate: AtomicU32,
}

impl Shared {
    /// Pull the ring into `output` at the commanded rate and set the level.
    /// Shared by the three sample-format callbacks.
    fn fill(&self, output: &mut [f32]) {
        let rate = self.rate.load(Ordering::Relaxed);
        let mut ring = self.ring.lock().unwrap();
        if rate == 1_000 {
            ring.read(output);
        } else {
            ring.read_stretched(output, rate);
        }
        drop(ring);
        apply_volume_frame(output, self.volume.load(Ordering::Relaxed));
    }
}

fn select_named_output<T>(
    devices: impl Iterator<Item = (String, T)>,
    wanted: &str,
) -> Result<T, AudioError> {
    let wanted = wanted.trim();
    if !wanted.is_empty() {
        for (name, device) in devices {
            if name.eq_ignore_ascii_case(wanted) {
                return Ok(device);
            }
        }
    }
    Err(AudioError::InitFailed(format!(
        "audio output device not found: {wanted:?}"
    )))
}

pub struct CpalBackend {
    output_device: Option<String>,
    shared: Arc<Shared>,
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
    /// Stream time the ring's first frame corresponds to, set by `flush`.
    base: Duration,
}

impl CpalBackend {
    pub fn new() -> Self {
        Self::with_output_device(None)
    }

    pub fn with_output_device(output_device: Option<String>) -> Self {
        Self {
            output_device,
            shared: Arc::new(Shared {
                ring: Mutex::new(Ring::new(2)),
                volume: AtomicU32::new(1000),
                rate: AtomicU32::new(1000),
            }),
            stream: None,
            sample_rate: 48_000,
            channels: 2,
            base: Duration::ZERO,
        }
    }

    pub fn output_devices() -> Vec<String> {
        cpal::default_host()
            .output_devices()
            .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
            .unwrap_or_default()
    }

    fn create_stream(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
        let host = cpal::default_host();
        let device = if let Some(name) = &self.output_device {
            let devices = host
                .output_devices()
                .map_err(|error| AudioError::InitFailed(error.to_string()))?;
            select_named_output(
                devices.filter_map(|device| device.name().ok().map(|name| (name, device))),
                name,
            )?
        } else {
            host.default_output_device()
                .ok_or_else(|| AudioError::InitFailed("no output device".into()))?
        };

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
                    shared.fill(data);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut f32_buf = vec![0.0; data.len()];
                    shared.fill(&mut f32_buf);
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
                    let mut f32_buf = vec![0.0; data.len()];
                    shared.fill(&mut f32_buf);
                    for (i, s) in f32_buf.iter().enumerate() {
                        data[i] = (s.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                },
                err_fn,
                None,
            ),
        }
        .map_err(|e| AudioError::InitFailed(format!("build stream: {e}")))?;

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
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
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
            stream
                .pause()
                .map_err(|e| AudioError::PlaybackError(format!("pause: {e}")))?;
        }
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = &self.stream {
            stream
                .play()
                .map_err(|e| AudioError::PlaybackError(format!("play: {e}")))?;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
        }
        Ok(())
    }

    fn set_volume(&mut self, milli: u32) -> Result<(), AudioError> {
        self.shared.volume.store(milli, Ordering::Relaxed);
        Ok(())
    }

    fn set_rate(&mut self, milli: u32) -> Result<(), AudioError> {
        self.shared.rate.store(milli, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_output_selection_matches_names_and_never_falls_back() {
        let devices = || vec![("Speakers".to_owned(), 1), ("USB DAC".to_owned(), 2)].into_iter();
        assert_eq!(
            super::select_named_output(devices(), " usb dac ").unwrap(),
            2
        );
        assert!(super::select_named_output(devices(), "missing").is_err());
        assert!(super::select_named_output(devices(), "").is_err());
    }

    use super::Ring;
    use crate::audio::scaled_frames;

    fn ring_with(frames: &[f32]) -> Ring {
        let mut ring = Ring::new(1);
        assert_eq!(ring.write(frames), frames.len());
        ring
    }

    /// A stereo pair wraps in the middle of a frame, not at an element boundary,
    /// so the room at the end of the ring has to be counted in frames by both the
    /// writer and the reader. Mono cannot tell the two apart, which is how long
    /// this went unnoticed.
    #[test]
    fn a_stereo_stream_survives_wrapping_the_ring() {
        let mut ring = Ring::new(2);
        let lead = (super::RING_FRAMES - 10) * 2;
        let first: Vec<f32> = (0..lead).map(|element| element as f32 * 0.25).collect();
        assert_eq!(ring.write(&first), super::RING_FRAMES - 10);
        let mut drain = vec![0.0; lead];
        assert_eq!(ring.read(&mut drain), super::RING_FRAMES - 10);
        assert_eq!(drain, first);

        // Written and read again from ten frames before the end of the buffer.
        let tail: Vec<f32> = (0..2_000).map(|element| 100.0 + element as f32).collect();
        assert_eq!(ring.write(&tail), 1_000);
        let mut back = vec![0.0; 2_000];
        assert_eq!(ring.read(&mut back), 1_000);
        assert_eq!(back, tail);
    }

    /// The clock runs on the ring position, so a stretched read has to move it by
    /// what the rate asks for even though its segments are cut wherever the wave
    /// fits rather than exactly where the rate says.
    #[test]
    fn a_stretched_read_moves_the_clock_by_its_rate() {
        let signal: Vec<f32> = (0..16_384).map(|f| (f as f32 * 0.063).sin()).collect();
        let mut ring = Ring::new(2);
        assert_eq!(ring.write(&signal), signal.len() / 2);
        let mut out = vec![0.0f32; 882];
        let mut given = 0usize;
        loop {
            let written = ring.read_stretched(&mut out, 1_750);
            if written == 0 {
                break;
            }
            given += written / 2;
        }
        assert!(given > 1_000, "the stretch produced {given} frames");
        let (exact, _) = scaled_frames(given, 1_750, 0);
        assert!(
            ring.position_frames().abs_diff(exact as u64)
                <= 2 * super::super::STRETCH_TOLERANCE as u64,
            "clock at {} for a nominal {exact}",
            ring.position_frames(),
        );
    }

    /// The stretch is handed a contiguous copy of the ring because it searches a
    /// window for the place the wave fits, so a window that crosses the end of the
    /// buffer has to be joined there.
    #[test]
    fn a_stretched_read_joins_a_window_that_wraps() {
        let signal: Vec<f32> = (0..4_096).map(|f| (f as f32 * 0.09).sin()).collect();
        let mut ring = Ring::new(1);
        let lead = super::RING_FRAMES - 60;
        let filler = vec![0.0f32; lead];
        assert_eq!(ring.write(&filler), lead);
        let mut drain = vec![0.0; lead];
        assert_eq!(ring.read(&mut drain), lead);
        assert_eq!(ring.position_frames(), lead as u64);

        // Sixty frames sit at the tail of the ring and the rest wraps to its head.
        assert_eq!(ring.write(&signal), signal.len());
        let mut out = vec![0.0f32; 441];
        let written = ring.read_stretched(&mut out, 2_000);
        assert!(written > 0, "the wrap stopped the stretch");
        // The first segment is a plain copy of the window, so the joined one has
        // to start where the ring's head does.
        let copied = 256.min(written);
        assert_eq!(&out[..copied], &signal[..copied]);
        assert!(ring.position_frames() > lead as u64);
    }

    /// A seek hands the ring a new position, and a fade reaching from the old one
    /// into the new would be heard as a splice between the two.
    #[test]
    fn a_flush_forgets_the_half_finished_segment() {
        let first: Vec<f32> = (0..4_096).map(|f| (f as f32 * 0.07).sin()).collect();
        let mut ring = Ring::new(1);
        assert_eq!(ring.write(&first), first.len());
        let mut out = vec![0.0f32; 882];
        assert!(ring.read_stretched(&mut out, 2_000) > 0);

        ring.flush();
        let second: Vec<f32> = first.iter().map(|sample| -sample).collect();
        assert_eq!(ring.write(&second), second.len());
        assert!(ring.read_stretched(&mut out, 2_000) > 0);
        assert_eq!(out[0], second[0], "the old segment faded into the new one");
    }

    #[test]
    fn a_flush_rewinds_both_positions() {
        let mut ring = ring_with(&[1.0, 2.0, 3.0]);
        let mut output = vec![0.0; 2];
        ring.read(&mut output);
        ring.flush();
        assert_eq!(ring.readable(), 0);
        assert_eq!(ring.position_frames(), 0);
    }
}
