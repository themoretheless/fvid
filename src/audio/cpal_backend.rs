//! Cross-platform audio backend using cpal.
//!
//! The backend opens the default or explicitly selected output device, configures it for the stream's
//! sample rate and channel count, and runs a callback that fills the audio
//! buffer from a ring of decoded samples. The ring absorbs jitter between the
//! decoder thread and the audio hardware clock.

use super::{
    AudioBackend, AudioError, AudioPacket, AudioSpec, STRETCH_SEGMENT, STRETCH_TOLERANCE,
    SampleFormat, Stretch, apply_volume_frame,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Initial decoded ring capacity (~500 ms at48kHz). Larger access units
/// grow it fallibly; the output queue has an explicit retained-byte ceiling.
const RING_FRAMES: usize = 24_000;
const MAX_RING_BYTES: usize = 32 * 1024 * 1024;

struct Ring {
    /// Interleaved f32 samples; capacity grows within the retained-byte limit.
    buffer: Vec<f32>,
    /// Number of channels.
    channels: usize,
    /// Write position in frames (decoder advances).
    write_pos: u64,
    /// Read position in frames (audio callback advances).
    read_pos: AtomicU64,
    /// Nominal source progress actually rendered, independent of search history.
    rendered_pos: AtomicU64,
    /// Pitch-preserving reader of a rate other than the recorded one, made the
    /// first time one is asked for and forgotten again by `flush`.
    stretch: Option<Stretch>,
    /// Ring frame that the stretch counts its own frames from.
    stretch_base: u64,
    /// Contiguous copy of the window last handed to the stretch, reused between
    /// callbacks so the device thread stops allocating.
    window: Vec<f32>,
    draining: bool,
}

impl Ring {
    fn new(channels: usize) -> Self {
        Self::try_new(channels).expect("valid bounded audio ring")
    }
    fn try_new(channels: usize) -> Result<Self, AudioError> {
        let elements = RING_FRAMES
            .checked_mul(channels)
            .filter(|_| channels != 0)
            .ok_or_else(|| AudioError::PlaybackError("invalid audio buffer geometry".into()))?;
        let bytes = elements
            .checked_mul(size_of::<f32>())
            .filter(|&n| n <= MAX_RING_BYTES)
            .ok_or_else(|| {
                AudioError::PlaybackError("audio buffer exceeds 32 MiB memory limit".into())
            })?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(bytes / size_of::<f32>())
            .map_err(|e| AudioError::PlaybackError(format!("audio buffer allocation: {e}")))?;
        buffer.resize(elements, 0.0);
        Ok(Self {
            buffer,
            channels,
            write_pos: 0,
            read_pos: AtomicU64::new(0),
            rendered_pos: AtomicU64::new(0),
            stretch: None,
            stretch_base: 0,
            window: Vec::new(),
            draining: false,
        })
    }
    fn capacity_frames(&self) -> usize {
        self.buffer.len() / self.channels
    }
    fn reserve_frames(&mut self, additional: usize) -> Result<(), AudioError> {
        let queued = self.readable();
        let needed = queued
            .checked_add(additional)
            .ok_or_else(|| AudioError::PlaybackError("audio buffer size overflow".into()))?;
        let max_frames = MAX_RING_BYTES / (self.channels * size_of::<f32>());
        if needed > max_frames {
            return Err(AudioError::PlaybackError(
                "audio output queue exceeds 32 MiB memory limit".into(),
            ));
        }
        let old_capacity = self.capacity_frames();
        if needed <= old_capacity {
            return Ok(());
        }
        let capacity = needed
            .checked_next_power_of_two()
            .unwrap_or(max_frames)
            .min(max_frames);
        let elements = capacity * self.channels;
        let mut grown = Vec::new();
        grown
            .try_reserve_exact(elements)
            .map_err(|e| AudioError::PlaybackError(format!("audio buffer allocation: {e}")))?;
        grown.resize(elements, 0.0);
        let read = self.read_pos.load(Ordering::Acquire);
        let mut offset = 0;
        while offset < queued {
            let frame = read.wrapping_add(offset as u64);
            let old = (frame % old_capacity as u64) as usize;
            let new = (frame % capacity as u64) as usize;
            let frames = (queued - offset)
                .min(old_capacity - old)
                .min(capacity - new);
            let elements = frames * self.channels;
            let old = old * self.channels;
            let new = new * self.channels;
            grown[new..new + elements].copy_from_slice(&self.buffer[old..old + elements]);
            offset += frames;
        }
        self.buffer = grown;
        Ok(())
    }
    fn write_all(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(AudioError::PlaybackError(
                "partial decoded audio frame".into(),
            ));
        }
        let frames = samples.len() / self.channels;
        self.reserve_frames(frames)?;
        if self.write(samples) != frames {
            return Err(AudioError::PlaybackError(
                "incomplete audio buffer write".into(),
            ));
        }
        if frames != 0 {
            self.draining = false;
        }
        Ok(())
    }

    /// Caller reserves the complete gap and payload before publishing either.
    fn write_silence(&mut self, frames: usize) {
        let index = (self.write_pos % self.capacity_frames() as u64) as usize;
        let head = frames.min(self.capacity_frames() - index);
        self.buffer[index * self.channels..(index + head) * self.channels].fill(0.0);
        self.buffer[..(frames - head) * self.channels].fill(0.0);
        self.write_pos += frames as u64;
    }

    fn readable(&self) -> usize {
        let w = self.write_pos;
        let r = self.read_pos.load(Ordering::Acquire);
        (w.saturating_sub(r) as usize).min(self.capacity_frames())
    }

    fn writable(&self) -> usize {
        self.capacity_frames() - self.readable()
    }

    /// Write interleaved f32 samples. Returns frames written.
    fn write(&mut self, samples: &[f32]) -> usize {
        let frames = samples.len() / self.channels;
        let available = self.writable();
        let to_write = frames.min(available);
        if to_write == 0 {
            return 0;
        }
        let write_idx = (self.write_pos % self.capacity_frames() as u64) as usize * self.channels;
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
        let read_idx = (self.read_pos.load(Ordering::Acquire) % self.capacity_frames() as u64)
            as usize
            * self.channels;
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
        let end = self
            .read_pos
            .fetch_add(to_read as u64, Ordering::Release)
            .wrapping_add(to_read as u64);
        self.rendered_pos.store(end, Ordering::Release);
        to_read
    }

    fn flush(&mut self) {
        self.write_pos = 0;
        self.read_pos.store(0, Ordering::Release);
        self.rendered_pos.store(0, Ordering::Release);
        self.buffer.fill(0.0);
        // The next packet starts a fresh stream frame, and a stale stretch would
        // fade the last segment of the old position into it.
        self.stretch = None;
        self.stretch_base = 0;
        self.draining = false;
    }

    fn position_frames(&self) -> u64 {
        self.rendered_pos.load(Ordering::Acquire)
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
        if given == 0
            && !self
                .stretch
                .as_ref()
                .is_some_and(Stretch::has_pending_output)
        {
            output.fill(0.0);
            return 0;
        }
        // The ring is circular and the stretch searches one window for the place
        // the wave fits, which would cross the join twice; hand it a copy.
        let read = self.read_pos.load(Ordering::Acquire);
        let start = (read % self.capacity_frames() as u64) as usize * channels;
        let actual_elements = (given as usize) * channels;
        let padding = if self.draining {
            (STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE) * channels
        } else {
            0
        };
        let head = actual_elements.min(self.buffer.len() - start);
        self.window.resize(actual_elements + padding, 0.0);
        self.window[..head].copy_from_slice(&self.buffer[start..start + head]);
        if head < actual_elements {
            self.window[head..actual_elements]
                .copy_from_slice(&self.buffer[..actual_elements - head]);
        }
        self.window[actual_elements..].fill(0.0);
        // The ring's frame `read` stands at the stretch's own frame counted from
        // where it first took over, which is where it retired to last time.
        let base = (read - self.stretch_base) as usize;
        let stretch = self.stretch.as_mut().expect("made just above");
        let written = if self.draining {
            let end = self
                .write_pos
                .saturating_sub(self.stretch_base)
                .min(usize::MAX as u64) as usize;
            stretch.fill_final(base, &self.window, end, output)
        } else {
            stretch.fill(base, &self.window, output)
        };
        let retired = stretch.retired() as u64;
        let rendered = stretch.rendered() as u64;
        output[written..].fill(0.0);
        self.read_pos.store(
            self.stretch_base
                .saturating_add(retired)
                .min(self.write_pos),
            Ordering::Release,
        );
        self.rendered_pos.store(
            self.stretch_base
                .saturating_add(rendered)
                .min(self.write_pos),
            Ordering::Release,
        );
        written
    }
}

#[derive(Default)]
struct OutputTail {
    due: Option<Instant>,
    remaining: Option<Duration>,
    paused: bool,
}

struct ActiveCallback<'a>(&'a AtomicU64);
impl Drop for ActiveCallback<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

struct Shared {
    ring: Mutex<Ring>,
    /// Output gain in thousandths, read by the device callback.
    volume: AtomicU32,
    /// Stream frames consumed per device frame, in thousandths.
    rate: AtomicU32,
    output_tail: Mutex<OutputTail>,
    output_epoch: AtomicU64,
    active_callbacks: AtomicU64,
    stream_generation: AtomicU64,
    output_error: Mutex<Option<String>>,
}

impl Shared {
    fn callback_guard(&self) -> ActiveCallback<'_> {
        self.active_callbacks.fetch_add(1, Ordering::AcqRel);
        ActiveCallback(&self.active_callbacks)
    }
    fn record_error(&self, error: String, generation: u64) -> bool {
        let mut current = self.output_error.lock().unwrap();
        if generation != self.stream_generation.load(Ordering::Acquire) {
            return false;
        }
        *current = Some(error);
        true
    }
    /// Pull production ring samples; tests may consume them synchronously.
    #[cfg(test)]
    fn fill(&self, output: &mut [f32]) {
        let _callback = self.callback_guard();
        let _ = self.render(output);
    }

    fn render(&self, output: &mut [f32]) -> (usize, u64) {
        let rate = self.rate.load(Ordering::Relaxed);
        let mut ring = self.ring.lock().unwrap();
        let epoch = self.output_epoch.load(Ordering::Acquire);
        let channels = ring.channels;
        let produced = if rate == 1_000 && ring.stretch.is_none() {
            ring.read(output)
        } else {
            ring.read_stretched(output, rate) / channels
        };
        drop(ring);
        apply_volume_frame(output, self.volume.load(Ordering::Relaxed));
        (produced, epoch)
    }
    fn fill_timed(&self, output: &mut [f32], sample_rate: u32, latency: Duration) {
        let _callback = self.callback_guard();
        let (produced, epoch) = self.render(output);
        self.record_tail(produced, sample_rate, latency, epoch);
    }
    fn record_tail(&self, frames: usize, sample_rate: u32, latency: Duration, epoch: u64) {
        if frames == 0 || sample_rate == 0 {
            return;
        }
        let ns = (frames as u128 * 1_000_000_000).div_ceil(u128::from(sample_rate));
        let duration = Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX));
        let now = Instant::now();
        let lead = latency.saturating_add(duration);
        let lead = if now.checked_add(lead).is_some() {
            lead
        } else {
            duration
        };
        let mut tail = self.output_tail.lock().unwrap();
        if epoch != self.output_epoch.load(Ordering::Acquire) {
            return;
        }
        if tail.paused {
            tail.remaining = Some(tail.remaining.map_or(lead, |old| old.max(lead)));
        } else if let Some(due) = now.checked_add(lead) {
            tail.due = Some(tail.due.map_or(due, |old| old.max(due)));
        }
    }
    // Called while holding the ring lock, so an in-flight callback can never
    // publish an old tail into a new navigation generation.
    fn reset_output(&self, new_stream: bool) {
        self.output_epoch.fetch_add(1, Ordering::AcqRel);
        let mut tail = self.output_tail.lock().unwrap();
        tail.due = None;
        tail.remaining = None;
        if new_stream {
            tail.paused = false;
            self.stream_generation.fetch_add(1, Ordering::AcqRel);
            *self.output_error.lock().unwrap() = None;
        }
    }
    fn pause_tail(&self) {
        let mut tail = self.output_tail.lock().unwrap();
        let now = Instant::now();
        if let Some(due) = tail.due.take() {
            let left = due.saturating_duration_since(now);
            if !left.is_zero() {
                tail.remaining = Some(tail.remaining.map_or(left, |old| old.max(left)));
            }
        }
        tail.paused = true;
    }
    fn resume_tail(&self) {
        let mut tail = self.output_tail.lock().unwrap();
        tail.paused = false;
        if let Some(left) = tail.remaining.take() {
            if let Some(due) = Instant::now().checked_add(left) {
                tail.due = Some(tail.due.map_or(due, |old| old.max(due)));
            }
        }
    }
}

fn callback_latency(info: &cpal::OutputCallbackInfo) -> Duration {
    let stamp = info.timestamp();
    stamp
        .playback
        .duration_since(&stamp.callback)
        .unwrap_or_default()
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
    stereo_router:Option<fvid_media::owned_stereo_router::StereoRouter>,
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
            stereo_router:None,
            output_device,
            shared: Arc::new(Shared {
                ring: Mutex::new(Ring::new(2)),
                volume: AtomicU32::new(1000),
                rate: AtomicU32::new(1000),
                output_tail: Mutex::new(OutputTail::default()),
                output_epoch: AtomicU64::new(0),
                active_callbacks: AtomicU64::new(0),
                stream_generation: AtomicU64::new(0),
                output_error: Mutex::new(None),
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

    fn check_output(&self) -> Result<(), AudioError> {
        if let Some(error) = self.shared.output_error.lock().unwrap().as_ref() {
            return Err(AudioError::PlaybackError(format!("audio device: {error}")));
        }
        Ok(())
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
        let error_shared = self.shared.clone();
        let stream_generation = self.shared.stream_generation.load(Ordering::Acquire);
        let err_fn = move |err: cpal::StreamError| {
            if error_shared.record_error(err.to_string(), stream_generation) {
                eprintln!("audio stream error: {err}");
            }
        };
        let sample_rate = spec.sample_rate;

        let stream = match spec.format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    shared.fill_timed(data, sample_rate, callback_latency(info));
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                    let _callback = shared.callback_guard();
                    let mut f32_buf = vec![0.0; data.len()];
                    let (produced, epoch) = shared.render(&mut f32_buf);
                    for (i, s) in f32_buf.iter().enumerate() {
                        data[i] = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                    shared.record_tail(produced, sample_rate, callback_latency(info), epoch);
                },
                err_fn,
                None,
            ),
            SampleFormat::I32 => device.build_output_stream(
                &config,
                move |data: &mut [i32], info: &cpal::OutputCallbackInfo| {
                    let _callback = shared.callback_guard();
                    let mut f32_buf = vec![0.0; data.len()];
                    let (produced, epoch) = shared.render(&mut f32_buf);
                    for (i, s) in f32_buf.iter().enumerate() {
                        data[i] = (s.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                    shared.record_tail(produced, sample_rate, callback_latency(info), epoch);
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
        self.stereo_router=None;
        {
            let mut ring = self.shared.ring.lock().unwrap();
            *ring = Ring::try_new(spec.channels as usize)?;
            self.shared.reset_output(true);
        }
        // Built but left stopped: the device starts clocking at `resume`, so
        // opening a file cannot advance the audio clock before the caller
        // starts presenting pictures against it.
        self.create_stream(spec)
    }

    fn start_with_layout(&mut self,spec:AudioSpec,mask:Option<u32>)->Result<(),AudioError>{
        let router=mask.map(|mask|fvid_media::owned_stereo_router::StereoRouter::new(spec.channels,mask).map_err(AudioError::InitFailed)).transpose()?;
        self.start_with_router(spec,router)
    }
    fn start_with_router(&mut self,spec:AudioSpec,router:Option<fvid_media::owned_stereo_router::StereoRouter>)->Result<(),AudioError>{
        if let Some(router)=router {
            if router.input_channels()!=spec.channels{return Err(AudioError::InitFailed("speaker routing disagrees with source channels".into()));}
            self.start(AudioSpec{channels:2,..spec})?;self.stereo_router=Some(router);Ok(())
        }else{self.start(spec)}
    }
    fn end_drain(&mut self) -> Result<(), AudioError> {
        self.check_output()?;
        self.shared.ring.lock().unwrap().draining = false;
        Ok(())
    }
    fn reconfigure(&mut self, spec: AudioSpec, router: Option<fvid_media::owned_stereo_router::StereoRouter>, at: Duration, playing: bool) -> Result<(), AudioError> {
        self.stop()?;
        self.start_with_router(spec, router)?;
        self.flush(at)?;
        if playing { self.resume()?; }
        Ok(())
    }
    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
        self.check_output()?;
        if packet.data.len() > MAX_RING_BYTES {
            return Err(AudioError::PlaybackError(
                "decoded audio packet exceeds 32 MiB output limit".into(),
            ));
        }
        let source_channels=self.stereo_router.as_ref().map_or(self.channels,|router|router.input_channels());
        let stride = usize::from(source_channels) * size_of::<f32>();
        if stride == 0 || !packet.data.len().is_multiple_of(stride) {
            return Err(AudioError::PlaybackError(
                "partial decoded audio frame".into(),
            ));
        }
        let samples = if let Some(router)=&self.stereo_router {router.route(&packet.data,MAX_RING_BYTES).map_err(AudioError::PlaybackError)?}else{match packet.data.len() {
            0 => return Ok(()),
            _ => {
                let f32_samples: Vec<f32> = packet
                    .data
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
                f32_samples
            }
        }};
        if packet.timebase_den == 0 || self.sample_rate == 0 {
            return Err(AudioError::PlaybackError("invalid decoded audio timebase".into()));
        }
        let rate = u128::from(self.sample_rate);
        let denominator = u128::from(packet.timebase_den);
        // Nearest sample resolves sub-sample timestamp quantization without
        // accumulating rounding errors between successive packets.
        let packet_frame = (u128::from(packet.pts) * u128::from(packet.timebase_num) * rate
            + denominator / 2) / denominator;
        let base_frame = u128::from(self.base.as_secs()) * rate
            + (u128::from(self.base.subsec_nanos()) * rate + 500_000_000) / 1_000_000_000;
        let mut ring = self.shared.ring.lock().unwrap();
        let queued_end = base_frame + u128::from(ring.write_pos);
        let gap = usize::try_from(packet_frame.saturating_sub(queued_end))
            .map_err(|_| AudioError::PlaybackError("audio timestamp gap exceeds output limit".into()))?;
        let skipped = queued_end.saturating_sub(packet_frame).min((samples.len() / ring.channels) as u128) as usize;
        let samples = &samples[skipped * ring.channels..];
        let frames = samples.len() / ring.channels;
        let added = gap.checked_add(frames)
            .ok_or_else(|| AudioError::PlaybackError("audio timestamp gap exceeds output limit".into()))?;
        ring.write_pos.checked_add(added as u64)
            .ok_or_else(|| AudioError::PlaybackError("audio output timeline overflow".into()))?;
        ring.reserve_frames(added)?;
        ring.write_silence(gap);
        ring.write_all(samples)
    }

    fn begin_drain(&mut self) -> Result<(), AudioError> {
        self.check_output()?;
        self.shared.ring.lock().unwrap().draining = true;
        Ok(())
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        self.check_output()?;
        let empty = {
            let ring = self.shared.ring.lock().unwrap();
            ring.readable() == 0
                && !ring
                    .stretch
                    .as_ref()
                    .is_some_and(Stretch::has_pending_output)
        };
        if !empty {
            return Ok(false);
        }
        let tail = self.shared.output_tail.lock().unwrap();
        Ok(tail.remaining.is_none_or(|remaining| remaining.is_zero())
            && tail.due.is_none_or(|due| Instant::now() >= due)
            && self.shared.active_callbacks.load(Ordering::Acquire) == 0)
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
            self.shared.reset_output(false);
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
        self.shared.pause_tail();
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        if let Some(stream) = &self.stream {
            stream
                .play()
                .map_err(|e| AudioError::PlaybackError(format!("play: {e}")))?;
        }
        self.shared.resume_tail();
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

#[cfg(test)]
mod packet_conservation_regressions {
    use super::*;
    use crate::audio::AudioStream;
    use std::io::Cursor;
    #[test]
    fn one_large_decoded_access_unit_reaches_the_output_ring_without_loss() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-ring-capacity");
        let bytes = std::fs::read(base.join("large-pcm.mkv")).unwrap();
        let mut video =
            crate::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut pictures = 0;
        while video.read_frame_raw().unwrap().is_some() {
            pictures += 1;
        }
        assert_eq!(pictures, 35);
        let expected: Vec<f32> = std::fs::read(base.join("expected.f32le"))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut source = crate::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let encoded = source.next_packet().unwrap().unwrap();
        let frame = decoder
            .decode_packet(&encoded.data, encoded.pts, encoded.duration as u64)
            .unwrap()
            .unwrap();
        let packet = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
            .unwrap();
        assert_eq!(packet.data.len() / 4, 32768);
        assert!(source.next_packet().unwrap().is_none());
        source.validate_eof().unwrap();
        let mut backend = CpalBackend::new();
        backend.channels = 1;
        backend.sample_rate = 48000;
        *backend.shared.ring.lock().unwrap() = Ring::new(1);
        backend.push(packet).unwrap();
        let ring = backend.shared.ring.lock().unwrap();
        assert_eq!(
            ring.readable(),
            32768,
            "successful push must retain every decoded frame"
        );
        drop(ring);
        let mut actual = vec![0.0; 32768];
        backend.shared.fill(&mut actual);
        assert_eq!(actual, expected);
        assert_eq!(backend.shared.ring.lock().unwrap().readable(), 0);
        assert_eq!(
            backend.position(),
            Duration::from_secs_f64(32768.0 / 48000.0)
        );
    }
}

#[cfg(test)]
mod ring_growth_admission_regressions {
    use super::*;
    fn packet(samples: &[f32]) -> AudioPacket {
        AudioPacket {
            data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
            pts: 0,
            timebase_num: 1,
            timebase_den: 48000,
        }
    }
    fn packet_at_end(backend: &CpalBackend, samples: &[f32]) -> AudioPacket {
        let mut packet = packet(samples);
        packet.pts = backend.base.as_secs() * 48000
            + (u64::from(backend.base.subsec_nanos()) * 48000 + 500_000_000) / 1_000_000_000
            + backend.shared.ring.lock().unwrap().write_pos;
        packet
    }
    #[test]
    fn growth_preserves_wrapped_stereo_tail_and_consumed_position() {
        let mut backend = CpalBackend::new();
        let prefix: Vec<f32> = (0..(RING_FRAMES - 8) * 2)
            .map(|i| (i % 512) as f32 / 1024.0 - 0.25)
            .collect();
        backend.push(packet_at_end(&backend, &prefix)).unwrap();
        let mut consumed = vec![0.0; (RING_FRAMES - 16) * 2];
        backend.shared.fill(&mut consumed);
        assert_eq!(consumed, prefix[..consumed.len()]);
        let tail: Vec<f32> = (0..32).flat_map(|_| [0.375, -0.375]).collect();
        backend.push(packet_at_end(&backend, &tail)).unwrap();
        let at = backend.position();
        let large: Vec<f32> = (0..32768)
            .flat_map(|i| {
                [
                    (i % 128) as f32 / 256.0 - 0.25,
                    0.25 - (i % 128) as f32 / 256.0,
                ]
            })
            .collect();
        backend.push(packet_at_end(&backend, &large)).unwrap();
        assert_eq!(backend.position(), at);
        let expected: Vec<f32> = prefix[consumed.len()..]
            .iter()
            .chain(&tail)
            .chain(&large)
            .copied()
            .collect();
        assert_eq!(
            backend.shared.ring.lock().unwrap().readable(),
            expected.len() / 2
        );
        let mut actual = vec![0.0; expected.len()];
        backend.shared.fill(&mut actual);
        assert_eq!(actual, expected);
        assert_eq!(backend.shared.ring.lock().unwrap().readable(), 0);
        assert_eq!(
            backend.position(),
            Duration::from_secs_f64(((RING_FRAMES - 16) + expected.len() / 2) as f64 / 48000.0)
        );
    }
    #[test]
    fn overflow_budget_and_partial_frame_refusals_preserve_queued_audio() {
        let mut ring = Ring::new(2);
        let expected = [0.125, -0.125, 0.25, -0.25];
        ring.write_all(&expected).unwrap();
        let capacity = ring.capacity_frames();
        assert!(ring.reserve_frames(usize::MAX).is_err());
        assert!(
            ring.reserve_frames(MAX_RING_BYTES / (2 * size_of::<f32>()) + 1)
                .is_err()
        );
        assert!(ring.write_all(&[0.5]).is_err());
        assert_eq!(ring.capacity_frames(), capacity);
        assert_eq!(ring.position_frames(), 0);
        assert_eq!(ring.readable(), 2);
        let mut actual = [0.0; 4];
        assert_eq!(ring.read(&mut actual), 2);
        assert_eq!(actual, expected);
        assert!(Ring::try_new(0).is_err());
        assert!(Ring::try_new(usize::MAX).is_err());
        assert!(Ring::try_new(MAX_RING_BYTES / (RING_FRAMES * size_of::<f32>()) + 1).is_err());
    }
    #[test]
    fn flush_after_growth_drops_old_audio_and_reanchors_output_clock() {
        let mut backend = CpalBackend::new();
        let large = vec![0.25; 32768 * 2];
        backend.push(packet_at_end(&backend, &large)).unwrap();
        let mut initial = vec![0.0; 1000 * 2];
        backend.shared.fill(&mut initial);
        assert_eq!(initial, vec![0.25; 2000]);
        backend.flush(Duration::from_secs(2)).unwrap();
        assert_eq!(backend.position(), Duration::from_secs(2));
        let mut empty = [1.0; 16];
        backend.shared.fill(&mut empty);
        assert_eq!(empty, [0.0; 16]);
        assert_eq!(backend.position(), Duration::from_secs(2));
        let next = [0.125, -0.125, 0.375, -0.375];
        backend.push(packet_at_end(&backend, &next)).unwrap();
        let mut actual = [0.0; 4];
        backend.shared.fill(&mut actual);
        assert_eq!(actual, next);
        assert_eq!(
            backend.position(),
            Duration::from_secs(2) + Duration::from_secs_f64(2.0 / 48000.0)
        );
    }
    #[test]
    fn grown_ring_stretch_matches_contiguous_fixture_samples() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-ring-capacity/expected.f32le");
        let signal: Vec<f32> = std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut ring = Ring::new(1);
        ring.write_all(&signal).unwrap();
        let mut reference = Stretch::new(1, 1250);
        let mut passed_old_boundary = false;
        for _ in 0..200 {
            let base = reference.retired();
            let remaining = signal.len().saturating_sub(base);
            let given = reference.lookahead(512).min(remaining);
            let mut expected = vec![0.0; 512];
            let wanted = if given == 0 {
                0
            } else {
                reference.fill(base, &signal[base..base + given], &mut expected)
            };
            expected[wanted..].fill(0.0);
            let mut actual = vec![0.0; 512];
            let written = ring.read_stretched(&mut actual, 1250);
            assert_eq!(written, wanted);
            assert_eq!(actual, expected, "grown ring window at{base}");
            assert_eq!(
                ring.position_frames(),
                (reference.rendered() as u64).min(signal.len() as u64)
            );
            assert_eq!(
                ring.read_pos.load(Ordering::Acquire),
                (reference.retired() as u64).min(signal.len() as u64)
            );
            passed_old_boundary |= base > RING_FRAMES;
            if written == 0 {
                assert!(passed_old_boundary);
                return;
            }
        }
        panic!("stretched fixture did not reach EOF");
    }
    #[test]
    fn partial_fixture_pcm_refuses_without_publishing_any_samples() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-ring-capacity/expected.f32le");
        let mut bytes = std::fs::read(path).unwrap();
        let mut backend = CpalBackend::new();
        backend.channels = 1;
        *backend.shared.ring.lock().unwrap() = Ring::new(1);
        let first = f32::from_le_bytes(bytes[..4].try_into().unwrap());
        backend.push(packet_at_end(&backend, &[first])).unwrap();
        bytes.pop();
        let invalid = AudioPacket {
            data: bytes,
            pts: 0,
            timebase_num: 1,
            timebase_den: 48000,
        };
        assert!(
            backend.push(invalid).is_err(),
            "partial decoded sample must not be accepted"
        );
        assert_eq!(backend.shared.ring.lock().unwrap().readable(), 1);
        let mut actual = [0.0; 4];
        backend.shared.fill(&mut actual);
        assert_eq!(actual, [first, 0.0, 0.0, 0.0]);
    }
    struct CallbackDevice {
        backend: CpalBackend,
        samples: Arc<Mutex<Vec<f32>>>,
    }
    impl AudioBackend for CallbackDevice {
        fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
            *self.backend.shared.ring.lock().unwrap() = Ring::try_new(usize::from(spec.channels))?;
            self.backend.sample_rate = spec.sample_rate;
            self.backend.channels = spec.channels;
            Ok(())
        }
        fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
            let elements = packet.data.len() / 4;
            self.backend.push(packet)?;
            let mut output = vec![0.0; elements];
            self.backend.shared.fill(&mut output);
            self.samples.lock().unwrap().extend(output);
            Ok(())
        }
        fn position(&self) -> Duration {
            self.backend.position()
        }
        fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
            self.samples.lock().unwrap().clear();
            self.backend.flush(at)
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            self.backend.pause()
        }
        fn resume(&mut self) -> Result<(), AudioError> {
            self.backend.resume()
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            self.backend.stop()
        }
    }
    #[test]
    fn actual_audio_worker_preserves_large_packet_through_device_callback_and_navigation() {
        use crate::audio::AudioStream;
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-ring-capacity");
        let bytes = std::fs::read(base.join("large-pcm.mkv")).unwrap();
        let expected: Vec<f32> = std::fs::read(base.join("expected.f32le"))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.sample_rate(), 48000);
        let samples = Arc::new(Mutex::new(Vec::new()));
        let captured = samples.clone();
        let mut playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            Box::new(CallbackDevice {
                backend: CpalBackend::new(),
                samples: captured,
            })
        });
        for generation in 0..3 {
            if generation == 1 {
                playback.rewind();
            }
            if generation == 2 {
                playback.seek(Duration::ZERO);
            }
            playback.play();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                assert!(
                    std::time::Instant::now() < deadline,
                    "large PCM worker did not reach EOF"
                );
                match playback.poll() {
                    Some(crate::audio_thread::AudioEvent::Ended(g)) if g == generation => break,
                    Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(*samples.lock().unwrap(), expected);
            assert_eq!(
                playback.position(),
                Duration::from_secs_f64(32768.0 / 48000.0)
            );
        }
    }
}

#[cfg(test)]
mod eof_drain_regressions {
    use super::*;
    struct SlowDevice {
        backend: CpalBackend,
    }
    impl AudioBackend for SlowDevice {
        fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
            *self.backend.shared.ring.lock().unwrap() = Ring::try_new(usize::from(spec.channels))?;
            self.backend.sample_rate = spec.sample_rate;
            self.backend.channels = spec.channels;
            Ok(())
        }
        fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
            self.backend.push(packet)
        }
        fn begin_drain(&mut self) -> Result<(), AudioError> {
            self.backend.begin_drain()
        }
        fn drain_status(&self) -> Result<bool, AudioError> {
            self.backend.drain_status()
        }
        fn set_rate(&mut self, milli: u32) -> Result<(), AudioError> {
            self.backend.set_rate(milli)
        }
        fn position(&self) -> Duration {
            self.backend.position()
        }
        fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
            self.backend.flush(at)
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            self.backend.pause()
        }
        fn resume(&mut self) -> Result<(), AudioError> {
            self.backend.resume()
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            self.backend.stop()
        }
    }
    #[test]
    fn public_position_refreshes_during_long_packet_pacing() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-pacing-clock");
        let media = std::fs::read(base.join("pacing.mkv")).unwrap();
        let expected: Vec<f32> = std::fs::read(base.join("expected-packet.f32le")).unwrap()
            .chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut video = crate::playback_native::NativeReader::software(
            std::io::Cursor::new(media.clone()), 32 << 20).unwrap();
        for _ in 0..70 { assert!(video.read_frame_raw().unwrap().is_some()); }
        assert!(video.read_frame_raw().unwrap().is_none());
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(media), Default::default()).unwrap();
        let shared = CpalBackend::new().shared.clone();
        let device_shared = shared.clone();
        let playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            let mut backend = CpalBackend::new();
            backend.shared = device_shared;
            Box::new(SlowDevice { backend })
        });
        playback.play();
        wait_until(|| shared.ring.lock().unwrap().readable() == 32768, "first PCM packet not queued");
        std::thread::sleep(Duration::from_millis(20));
        let mut output = vec![0.0;960];
        shared.fill(&mut output);
        assert_eq!(output, expected[..960]);
        assert_eq!(shared.ring.lock().unwrap().position_frames(), 960);
        let deadline = Instant::now() + Duration::from_millis(100);
        while playback.position() != Duration::from_millis(20) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(playback.position(), Duration::from_millis(20),
            "public audio clock stayed stale during the large-packet pacing wait");
    }
    #[test]
    fn pcm_frontier_includes_decoded_packet_duration() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-frontier");
        let media = std::fs::read(base.join("frontier.mkv")).unwrap();
        let expected: Vec<f32> = std::fs::read(base.join("expected-packet.f32le")).unwrap()
            .chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut video = crate::playback_native::NativeReader::software(
            std::io::Cursor::new(media.clone()), 32 << 20).unwrap();
        for _ in 0..70 { assert!(video.read_frame_raw().unwrap().is_some()); }
        assert!(video.read_frame_raw().unwrap().is_none());
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(media), Default::default()).unwrap();
        let shared = CpalBackend::new().shared.clone();
        let device_shared = shared.clone();
        let playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            let mut backend = CpalBackend::new();
            backend.shared = device_shared;
            Box::new(SlowDevice { backend })
        });
        playback.play();
        wait_until(|| shared.ring.lock().unwrap().readable() == 32768, "first PCM packet not queued");
        std::thread::sleep(Duration::from_millis(100));
        let ring = shared.ring.lock().unwrap();
        assert_eq!(&ring.buffer[..32768], expected.as_slice());
        assert_eq!(ring.position_frames(),0);
        assert_eq!(ring.write_pos,32768,
            "second large PCM packet queued before the first packet's source lead was consumed");
    }
    #[test]
    fn accelerated_device_refills_when_source_lead_is_exhausted() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-refill-clock");
        let media = std::fs::read(base.join("refill.mkv")).unwrap();
        let expected: Vec<f32> = std::fs::read(base.join("expected-packet.f32le")).unwrap()
            .chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut video = crate::playback_native::NativeReader::software(
            std::io::Cursor::new(media.clone()), 32 << 20).unwrap();
        for _ in 0..105 { assert!(video.read_frame_raw().unwrap().is_some()); }
        assert!(video.read_frame_raw().unwrap().is_none());
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(media), Default::default()).unwrap();
        let shared = CpalBackend::new().shared.clone();
        let device_shared = shared.clone();
        let playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            let mut backend = CpalBackend::new();
            backend.shared = device_shared;
            Box::new(SlowDevice { backend })
        });
        playback.set_rate(2000);
        playback.play();
        wait_until(|| shared.ring.lock().unwrap().readable() == 32768, "first PCM packet not queued");
        std::thread::sleep(Duration::from_millis(20));
        let mut output = vec![0.0;12000];
        shared.fill(&mut output);
        assert!(output.iter().any(|sample| *sample != 0.0));
        assert_eq!(expected.len(),32768);
        assert_eq!(shared.ring.lock().unwrap().position_frames(),24000);
        let deadline = Instant::now() + Duration::from_millis(100);
        while shared.ring.lock().unwrap().write_pos < 65536 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let ring=shared.ring.lock().unwrap();
        // Contiguous native sample clocks do not insert the old 16-sample
        // timestamp-rounding silence between the first two PCM packets.
        assert_eq!(ring.write_pos,65536,
            "decoder failed to refill after accelerated device consumed its200ms source lead");
        assert_eq!(&ring.buffer[32768..65536],expected.as_slice(),
            "refill must contain the exact next packet, without inserted silence");
    }
    fn fixture() -> (Vec<u8>, Vec<f32>) {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-eof-drain");
        let data = std::fs::read(base.join("queued-tail.mkv")).unwrap();
        let expected = std::fs::read(base.join("expected.f32le"))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        (data, expected)
    }
    #[test]
    fn source_eof_waits_until_the_device_consumes_queued_pcm() {
        let (data, expected) = fixture();
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(data),
            Default::default(),
        )
        .unwrap();
        let shared = CpalBackend::new().shared.clone();
        let device_shared = shared.clone();
        let playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            let mut backend = CpalBackend::new();
            backend.shared = device_shared;
            Box::new(SlowDevice { backend })
        });
        playback.play();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while shared.ring.lock().unwrap().readable() != 480 {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture PCM was not queued"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let hold = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < hold {
            match playback.poll() {
                Some(crate::audio_thread::AudioEvent::Ended(_)) => {
                    panic!("source EOF emitted with480 unread device frames")
                }
                Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut output = vec![0.0; 480];
        shared.fill(&mut output);
        assert_eq!(output, expected);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "drained device did not finish"
            );
            match playback.poll() {
                Some(crate::audio_thread::AudioEvent::Ended(0)) => break,
                Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(playback.position(), Duration::from_millis(60));
    }
    fn wait_until(mut ready: impl FnMut() -> bool, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "{message}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn start_pending() -> (crate::audio_thread::AudioPlayback, Arc<Shared>, Vec<f32>) {
        let (data, expected) = fixture();
        let source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(data),
            Default::default(),
        )
        .unwrap();
        let shared = CpalBackend::new().shared.clone();
        let target = shared.clone();
        let playback = crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
            let mut backend = CpalBackend::new();
            backend.shared = target;
            Box::new(SlowDevice { backend })
        });
        playback.play();
        wait_until(
            || {
                let ring = shared.ring.lock().unwrap();
                ring.readable() == 480 && ring.draining
            },
            "producer did not enter device drain",
        );
        (playback, shared, expected)
    }
    fn hold_unfinished(playback: &crate::audio_thread::AudioPlayback) {
        let deadline = Instant::now() + Duration::from_millis(50);
        loop {
            match playback.poll() {
                Some(crate::audio_thread::AudioEvent::Ended(g)) => {
                    panic!("premature EOF generation{g}")
                }
                Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn wait_end(playback: &crate::audio_thread::AudioPlayback, generation: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "device drain did not finish generation{generation}"
            );
            match playback.poll() {
                Some(crate::audio_thread::AudioEvent::Ended(g)) => {
                    assert_eq!(g, generation);
                    return;
                }
                Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn pause_and_resume_keep_the_pending_tail_alive() {
        let (playback, shared, expected) = start_pending();
        playback.pause();
        wait_until(
            || shared.output_tail.lock().unwrap().paused,
            "pause was not handled during drain",
        );
        hold_unfinished(&playback);
        playback.play();
        wait_until(
            || !shared.output_tail.lock().unwrap().paused,
            "resume was not handled during drain",
        );
        let mut output = vec![0.0; 480];
        shared.fill(&mut output);
        assert_eq!(output, expected);
        wait_end(&playback, 0);
        assert_eq!(playback.position(), Duration::from_millis(60));
    }
    #[test]
    fn seek_cancels_old_drain_and_rejects_a_late_callback_tail() {
        let (mut playback, shared, expected) = start_pending();
        let old_epoch = shared.output_epoch.load(Ordering::Acquire);
        playback.seek(Duration::from_millis(20));
        wait_until(
            || {
                shared.output_epoch.load(Ordering::Acquire) != old_epoch
                    && shared.ring.lock().unwrap().readable() == 320
            },
            "seek did not replace pending PCM",
        );
        shared.record_tail(480, 8000, Duration::from_secs(10), old_epoch);
        hold_unfinished(&playback);
        let mut output = vec![0.0; 320];
        shared.fill(&mut output);
        assert_eq!(output, expected[160..]);
        wait_end(&playback, 1);
        assert_eq!(playback.position(), Duration::from_millis(60));
    }
    #[test]
    fn stop_does_not_wait_for_an_unconsumed_output_queue() {
        let (playback, shared, _) = start_pending();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(playback);
            let _ = done.send(());
        });
        finished
            .recv_timeout(Duration::from_secs(2))
            .expect("Stop waited for device drain");
        assert_eq!(shared.ring.lock().unwrap().position_frames(), 0);
    }
    #[test]
    fn device_failure_terminates_drain_as_error_without_false_eof() {
        let (playback, shared, _) = start_pending();
        let generation = shared.stream_generation.load(Ordering::Acquire);
        assert!(shared.record_error("synthetic device disconnect".into(), generation));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "failed device left drain waiting forever"
            );
            match playback.poll() {
                Some(crate::audio_thread::AudioEvent::Error(e)) => {
                    assert!(e.contains("synthetic device disconnect"), "{e}");
                    break;
                }
                Some(crate::audio_thread::AudioEvent::Ended(_)) => {
                    panic!("failed device reported successful EOF")
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(shared.ring.lock().unwrap().readable(), 480);
    }
    #[test]
    fn rendered_callback_tail_waits_for_latency_and_survives_pause_until_flush() {
        let (_, signal) = fixture();
        let mut backend = CpalBackend::new();
        backend.channels = 1;
        backend.sample_rate = 8000;
        *backend.shared.ring.lock().unwrap() = Ring::new(1);
        backend
            .push(AudioPacket {
                data: signal.iter().flat_map(|s| s.to_le_bytes()).collect(),
                pts: 0,
                timebase_num: 1,
                timebase_den: 8000,
            })
            .unwrap();
        backend.begin_drain().unwrap();
        let shared = backend.shared.clone();
        let callback = shared.callback_guard();
        let mut output = vec![0.0; 480];
        let (frames, epoch) = shared.render(&mut output);
        assert_eq!(output, signal);
        assert!(
            !backend.drain_status().unwrap(),
            "in-flight conversion must not look drained"
        );
        shared.record_tail(frames, 8000, Duration::from_secs(10), epoch);
        drop(callback);
        assert!(!backend.drain_status().unwrap());
        backend.pause().unwrap();
        assert!(shared.output_tail.lock().unwrap().remaining.is_some());
        assert!(!backend.drain_status().unwrap());
        backend.resume().unwrap();
        assert!(!backend.drain_status().unwrap());
        shared.output_tail.lock().unwrap().due = Some(Instant::now() - Duration::from_secs(1));
        assert!(backend.drain_status().unwrap());
        let mut silence = [1.0; 16];
        shared.fill_timed(&mut silence, 8000, Duration::from_secs(10));
        assert_eq!(silence, [0.0; 16]);
        assert!(
            backend.drain_status().unwrap(),
            "empty callbacks must not extend the tail"
        );
        backend.flush(Duration::from_secs(2)).unwrap();
        shared.record_tail(480, 8000, Duration::from_secs(10), epoch);
        assert!(backend.drain_status().unwrap());
        assert_eq!(backend.position(), Duration::from_secs(2));
    }
    #[test]
    fn device_errors_survive_seek_but_old_stream_errors_cannot_poison_restart() {
        let mut backend = CpalBackend::new();
        let shared = backend.shared.clone();
        let old = shared.stream_generation.load(Ordering::Acquire);
        assert!(shared.record_error("old failure".into(), old));
        backend.flush(Duration::ZERO).unwrap();
        assert!(backend.drain_status().is_err());
        {
            let _ring = shared.ring.lock().unwrap();
            shared.reset_output(true);
        }
        assert!(backend.drain_status().unwrap());
        assert!(!shared.record_error("late old failure".into(), old));
        assert!(backend.drain_status().unwrap());
        let new = shared.stream_generation.load(Ordering::Acquire);
        assert!(shared.record_error("new failure".into(), new));
        assert!(backend.drain_status().is_err());
    }
    #[test]
    fn actual_worker_finishes_short_stretched_audio_and_device_eof_at_all_rates() {
        for rate in [500, 875, 1000, 1234, 1250, 1750, 2000] {
            let (playback, shared, signal) = start_pending();
            playback.set_rate(rate);
            wait_until(
                || shared.rate.load(Ordering::Relaxed) == rate,
                "tempo command was not handled during drain",
            );
            let wanted = (480usize * 1000).div_ceil(rate as usize);
            let mut expected = vec![0.0; wanted];
            if rate == 1000 {
                expected.clone_from(&signal);
            } else {
                let mut padded = signal.clone();
                padded.resize(480 + STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE, 0.0);
                assert_eq!(
                    Stretch::new(1, rate).fill(0, &padded, &mut expected),
                    wanted
                );
            }
            let mut actual = Vec::new();
            for _ in 0..wanted.div_ceil(64) + 8 {
                let callback = shared.callback_guard();
                let mut output = [0.0; 64];
                let (frames, _) = shared.render(&mut output);
                drop(callback);
                actual.extend_from_slice(&output[..frames]);
                if frames == 0 {
                    break;
                }
            }
            assert_eq!(actual, expected, "worker output at rate{rate}");
            wait_end(&playback, 0);
            assert_eq!(playback.position(), Duration::from_millis(60));
        }
    }
}

#[cfg(test)]
mod finite_stretch_regressions {
    use super::*;
    #[test]
    fn finite_short_audio_emits_its_rate_adjusted_duration_and_drains() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-eof/expected-source.f32le");
        let signal: Vec<f32> = std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(signal.len(), 480);
        use crate::audio::AudioStream;
        let media = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/audio-stretch-eof/short-finite.mkv"),
        )
        .unwrap();
        let mut video = crate::playback_native::NativeReader::software(
            std::io::Cursor::new(media.clone()),
            32 << 20,
        )
        .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut audio = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(media),
            Default::default(),
        )
        .unwrap();
        let mut decoder = audio.make_decoder().unwrap();
        let mut decoded = Vec::new();
        while let Some(source) = audio.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&source.data, source.pts, source.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(packet) = audio
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                decoded.extend(
                    packet
                        .data
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes(b.try_into().unwrap())),
                );
            }
        }
        audio.validate_eof().unwrap();
        assert_eq!(decoded, signal);
        let mut ring = Ring::new(1);
        ring.write_all(&signal).unwrap();
        ring.draining = true;
        let mut produced = 0;
        for _ in 0..32 {
            let mut output = [0.0; 64];
            let written = ring.read_stretched(&mut output, 1250);
            produced += written;
            if written == 0 {
                break;
            }
        }
        assert_eq!(
            produced,
            (480usize * 1000).div_ceil(1250),
            "finite PCM disappeared below the stretch lookahead"
        );
        assert_eq!(ring.readable(), 0, "finite DSP tail was not retired");
    }
    fn finish_against_contiguous(
        signal: &[f32],
        channels: usize,
        rate: u32,
        chunk: usize,
        consumed: usize,
    ) {
        let remaining = signal.len() / channels - consumed;
        let wanted = (remaining as u128 * 1000).div_ceil(u128::from(rate)) as usize;
        let mut padded = signal[consumed * channels..].to_vec();
        padded.resize(
            padded.len() + (STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE) * channels,
            0.0,
        );
        let mut reference = Stretch::new(channels, rate);
        let mut expected = vec![0.0; wanted * channels];
        assert_eq!(reference.fill(0, &padded, &mut expected), expected.len());
        let mut ring = Ring::new(channels);
        ring.write_all(signal).unwrap();
        if consumed != 0 {
            let mut prefix = vec![0.0; consumed * channels];
            assert_eq!(ring.read(&mut prefix), consumed);
            assert_eq!(prefix, signal[..consumed * channels]);
        }
        ring.draining = true;
        let mut actual = Vec::new();
        for _ in 0..(wanted.div_ceil(chunk) + 8) * 2 {
            let mut output = vec![0.0; chunk * channels];
            let written = ring.read_stretched(&mut output, rate);
            actual.extend_from_slice(&output[..written]);
            if written == 0 {
                break;
            }
        }
        assert_eq!(actual.len(), wanted * channels, "rate{rate}, chunk{chunk}");
        assert_eq!(actual, expected, "finite wave at rate{rate}, chunk{chunk}");
        assert_eq!(ring.readable(), 0);
        assert_eq!(
            ring.position_frames(),
            signal.len() as u64 / channels as u64
        );
        assert!(!ring.stretch.as_ref().unwrap().has_pending_output());
    }
    #[test]
    fn finite_rates_fractional_carry_stereo_and_callback_boundaries_match_reference() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-eof/expected-source.f32le");
        let signal: Vec<f32> = std::fs::read(base)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        for rate in [500, 875, 1000, 1001, 1234, 1250, 1750, 2000] {
            finish_against_contiguous(&signal, 1, rate, 64, 0);
        }
        for chunk in [1, 63, 513] {
            finish_against_contiguous(&signal, 1, 1250, chunk, 0);
        }
        let stereo: Vec<f32> = signal.iter().flat_map(|&v| [v, -v]).collect();
        finish_against_contiguous(&stereo, 2, 1234, 63, 17);
    }
    #[test]
    fn grown_finite_access_unit_retires_the_last_partial_processing_segment() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-ring-capacity/expected.f32le");
        let signal: Vec<f32> = std::fs::read(base)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        finish_against_contiguous(&signal, 1, 1250, 512, 0);
    }
    #[test]
    fn zero_rate_preserves_finite_input_and_resumes_without_a_stuck_segment() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-eof/expected-source.f32le");
        let signal: Vec<f32> = std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut ring = Ring::new(1);
        ring.write_all(&signal).unwrap();
        ring.draining = true;
        let mut paused = [1.0; 64];
        assert_eq!(ring.read_stretched(&mut paused, 0), 0);
        assert_eq!(paused, [0.0; 64]);
        assert_eq!(ring.position_frames(), 0);
        assert_eq!(ring.readable(), 480);
        let mut actual = Vec::new();
        for _ in 0..16 {
            let mut output = [0.0; 64];
            let written = ring.read_stretched(&mut output, 1250);
            actual.extend_from_slice(&output[..written]);
            if written == 0 {
                break;
            }
        }
        let mut padded = signal.clone();
        padded.resize(480 + STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE, 0.0);
        let mut expected = vec![0.0; 384];
        assert_eq!(Stretch::new(1, 1250).fill(0, &padded, &mut expected), 384);
        assert_eq!(actual, expected);
        assert_eq!(ring.position_frames(), 480);
        assert_eq!(ring.readable(), 0);
    }
    #[test]
    fn returning_to_normal_rate_finishes_the_inflight_segment_and_finite_tail() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-eof/expected-source.f32le");
        let signal: Vec<f32> = std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut backend = CpalBackend::new();
        backend.channels = 1;
        backend.sample_rate = 8000;
        *backend.shared.ring.lock().unwrap() = Ring::new(1);
        backend
            .push(AudioPacket {
                data: signal.iter().flat_map(|s| s.to_le_bytes()).collect(),
                pts: 0,
                timebase_num: 1,
                timebase_den: 8000,
            })
            .unwrap();
        backend.begin_drain().unwrap();
        backend.set_rate(1250).unwrap();
        let mut first = [0.0; 64];
        assert_eq!(backend.shared.render(&mut first).0, 64);
        let mut actual = first.to_vec();
        backend.set_rate(1000).unwrap();
        for _ in 0..16 {
            let mut output = [0.0; 63];
            let frames = backend.shared.render(&mut output).0;
            actual.extend_from_slice(&output[..frames]);
            if frames == 0 {
                break;
            }
        }
        // The first256-frame processing segment retains its1.25rate. It spans
        //320 source frames; the remaining160 finish at the normal rate.
        assert_eq!(actual.len(), 416);
        let mut padded = signal.clone();
        padded.resize(480 + STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE, 0.0);
        let mut reference = Stretch::new(1, 1250);
        let mut expected = vec![0.0; 416];
        assert_eq!(reference.fill(0, &padded, &mut expected[..64]), 64);
        reference.set_rate(1000);
        assert_eq!(reference.fill(0, &padded, &mut expected[64..]), 352);
        assert_eq!(actual, expected);
        assert!(backend.drain_status().unwrap());
        assert_eq!(backend.position(), Duration::from_millis(60));
    }
}

#[cfg(test)]
mod clock_progress_regressions {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn stretched_master_clock_tracks_rendered_frames_not_search_retirement() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-clock");
        let media = std::fs::read(base.join("progress.mkv")).unwrap();
        let expected: Vec<f32> = std::fs::read(base.join("expected-source.f32le"))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut video = crate::playback_native::NativeReader::software(
            std::io::Cursor::new(media.clone()),
            32 << 20,
        )
        .unwrap();
        for _ in 0..7 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source = crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(media),
            Default::default(),
        )
        .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let encoded = source.next_packet().unwrap().unwrap();
        let frame = decoder
            .decode_packet(&encoded.data, encoded.pts, encoded.duration as u64)
            .unwrap()
            .unwrap();
        let packet = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
            .unwrap();
        assert_eq!(
            packet
                .data
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(expected.len(), 1120);
        assert!(source.next_packet().unwrap().is_none());
        source.validate_eof().unwrap();
        let mut backend = CpalBackend::new();
        backend.channels = 1;
        backend.sample_rate = 8000;
        *backend.shared.ring.lock().unwrap() = Ring::new(1);
        backend.set_rate(1750).unwrap();
        backend.push(packet).unwrap();
        backend.begin_drain().unwrap();
        let mut output = [0.0; 64];
        backend.shared.fill(&mut output);
        assert_eq!(output, expected[..64]);
        assert_eq!(
            backend.position(),
            Duration::from_millis(14),
            "64 device frames must advance112 source frames at1.75rate"
        );
        assert!(
            backend.position() < Duration::from_millis(20),
            "20ms video frame must still wait after8ms of device output"
        );
    }
    fn signal() -> Vec<f32> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-stretch-clock/expected-source.f32le");
        std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()
    }
    fn configured(channels: u16) -> CpalBackend {
        let mut backend = CpalBackend::new();
        backend.channels = channels;
        backend.sample_rate = 8000;
        *backend.shared.ring.lock().unwrap() = Ring::new(usize::from(channels));
        backend
    }
    fn packet(signal: &[f32]) -> AudioPacket {
        AudioPacket {
            data: signal.iter().flat_map(|s| s.to_le_bytes()).collect(),
            pts: 0,
            timebase_num: 1,
            timebase_den: 8000,
        }
    }
    #[test]
    fn callback_clock_is_sample_exact_across_rates_channels_and_fractional_phase() {
        let mono = signal();
        for channels in [1, 2] {
            for rate in [500, 875, 1000, 1001, 1234, 1250, 1750, 2000] {
                let samples: Vec<f32> = mono
                    .iter()
                    .flat_map(|&s| if channels == 1 { vec![s] } else { vec![s, -s] })
                    .collect();
                let mut backend = configured(channels);
                backend.push(packet(&samples)).unwrap();
                backend.set_rate(rate).unwrap();
                backend.begin_drain().unwrap();
                let shared = backend.shared.clone();
                let mut rendered = 0usize;
                let mut previous = Duration::ZERO;
                for call in 0..256 {
                    let callback = shared.callback_guard();
                    let mut output = vec![0.0; [1, 17, 64, 257][call % 4] * usize::from(channels)];
                    let (frames, _) = shared.render(&mut output);
                    drop(callback);
                    rendered += frames;
                    let source = (rendered as u128 * u128::from(rate) / 1000).min(1120) as u64;
                    let expected = Duration::from_nanos(source * 125_000);
                    let at = backend.position();
                    assert_eq!(
                        at, expected,
                        "rate{rate}, channels{channels}, callback{call}"
                    );
                    assert!(at >= previous);
                    previous = at;
                    if frames == 0 {
                        break;
                    }
                }
                assert_eq!(rendered, (1120usize * 1000).div_ceil(rate as usize));
                assert!(backend.drain_status().unwrap());
                assert_eq!(backend.position(), Duration::from_millis(140));
            }
        }
    }
    #[test]
    fn rate_switch_zero_suspend_and_seek_keep_the_rendered_clock_continuous() {
        let signal = signal();
        let mut backend = configured(1);
        backend.push(packet(&signal)).unwrap();
        let mut prefix = [0.0; 17];
        backend.shared.fill(&mut prefix);
        assert_eq!(prefix, signal[..17]);
        assert_eq!(backend.position(), Duration::from_nanos(17 * 125_000));
        backend.set_rate(1250).unwrap();
        backend.begin_drain().unwrap();
        let mut output = [0.0; 64];
        backend.shared.fill(&mut output);
        assert_eq!(
            backend.position(),
            Duration::from_nanos((17 + 80) * 125_000)
        );
        backend.set_rate(0).unwrap();
        backend.shared.fill(&mut output);
        assert_eq!(output, [0.0; 64]);
        assert_eq!(
            backend.position(),
            Duration::from_nanos((17 + 80) * 125_000)
        );
        backend.set_rate(1000).unwrap();
        backend.shared.fill(&mut output);
        assert_eq!(
            backend.position(),
            Duration::from_nanos((17 + 160) * 125_000)
        );
        backend.flush(Duration::from_secs(2)).unwrap();
        assert_eq!(backend.position(), Duration::from_secs(2));
        let mut after_seek = packet(&signal[..64]);
        after_seek.pts = 16_000;
        backend.push(after_seek).unwrap();
        backend.begin_drain().unwrap();
        backend.shared.fill(&mut output);
        assert_eq!(output, signal[..64]);
        assert_eq!(
            backend.position(),
            Duration::from_secs(2) + Duration::from_millis(8)
        );
        assert!(backend.drain_status().unwrap());
    }
}

#[cfg(test)]
mod packet_timeline_regressions {
    use super::*;
    use crate::audio::AudioStream;
    fn packets(name: &str, video_frames: usize, stamps: &[i64]) -> (Vec<AudioPacket>, Vec<f32>) {
        let base=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-packet-timeline");
        let media=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let expected_packet=std::fs::read(base.join("expected-packet.f32le")).unwrap();
        let expected=std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap()
            .chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut video=crate::playback_native::NativeReader::software(std::io::Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..video_frames {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(media),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();
        let mut packets=Vec::new();
        for &stamp in stamps {
            let encoded=source.next_packet().unwrap().unwrap();assert_eq!(encoded.pts,stamp);
            let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
            let packet=source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap();
            assert_eq!(packet.data,expected_packet);packets.push(packet);
        }
        assert!(source.next_packet().unwrap().is_none());
        (packets,expected)
    }
    fn virtual_backend() -> CpalBackend {
        let mut backend=CpalBackend::new();backend.sample_rate=8000;backend.channels=1;
        *backend.shared.ring.lock().unwrap()=Ring::new(1);backend
    }
    #[test]
    fn overlapping_and_duplicate_packets_do_not_repeat_already_queued_samples() {
        for (name,frames,stamps) in [("overlap",2,vec![0,10_000_000]),("duplicate",1,vec![0,0])] {
            let (packets,expected)=packets(name,frames,&stamps);
            let mut backend=virtual_backend();
            for packet in packets {backend.push(packet).unwrap();}
            let mut output=vec![0.0;expected.len()];backend.shared.fill(&mut output);
            assert_eq!(output,expected,"{name}");
            assert_eq!(backend.shared.ring.lock().unwrap().readable(),0);
        }
    }
    #[test]
    fn leading_gap_and_seek_anchor_preserve_the_declared_packet_start() {
        for anchor in [Duration::ZERO,Duration::from_millis(90),Duration::from_millis(110)] {
            let (mut packets,expected)=packets("leading",6,&[100_000_000]);
            let mut backend=virtual_backend();backend.flush(anchor).unwrap();
            backend.push(packets.remove(0)).unwrap();
            let skipped=(anchor.as_millis()*8) as usize;
            let mut output=vec![0.0;expected.len()-skipped];backend.shared.fill(&mut output);
            assert_eq!(output,&expected[skipped..]);
            assert_eq!(backend.position(),Duration::from_millis(120));
        }
    }
    #[test]
    fn oversized_gap_refuses_atomically_before_publishing_silence_or_payload() {
        let (mut packets,_) = packets("leading",6,&[100_000_000]);
        let mut backend=virtual_backend();
        let packet=packets.remove(0);
        let mut huge=AudioPacket{data:packet.data.clone(),pts:u64::MAX,timebase_num:1,timebase_den:8000};
        assert!(backend.push(huge).is_err());
        assert_eq!(backend.shared.ring.lock().unwrap().write_pos,0);
        huge=AudioPacket{data:packet.data.clone(),pts:0,timebase_num:1,timebase_den:0};
        assert!(backend.push(huge).is_err());
        assert_eq!(backend.shared.ring.lock().unwrap().write_pos,0);
        backend.push(packet).unwrap();
        backend.begin_drain().unwrap();
        let mut output=vec![0.0;960];backend.shared.fill(&mut output);
        assert!(backend.drain_status().unwrap());
    }
    #[test]
    fn stereo_gap_wraps_without_reusing_old_buffer_samples() {
        let (mut packets,_) = packets("leading",6,&[100_000_000]);
        let mono:Vec<f32>=packets.remove(0).data.chunks_exact(4)
            .map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        let stereo:Vec<f32>=mono.iter().flat_map(|&v|[v,-v]).collect();
        let mut backend=CpalBackend::new();
        let prefix=vec![0.375;(RING_FRAMES-4)*2];
        backend.shared.ring.lock().unwrap().write_all(&prefix).unwrap();
        let mut consumed=vec![0.0;prefix.len()];backend.shared.fill(&mut consumed);
        assert_eq!(consumed,prefix);
        backend.push(AudioPacket{data:stereo.iter().flat_map(|v|v.to_le_bytes()).collect(),
            pts:(RING_FRAMES+4) as u64,timebase_num:1,timebase_den:48000}).unwrap();
        let mut output=vec![1.0;16+stereo.len()];backend.shared.fill(&mut output);
        assert_eq!(&output[..16],&[0.0;16]);assert_eq!(&output[16..],stereo);
        assert_eq!(backend.shared.ring.lock().unwrap().readable(),0);
    }
    #[test]
    fn packet_timestamp_gap_preserves_silence_and_following_audio() {
        let base=std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-packet-timeline");
        let media=std::fs::read(base.join("gap.mkv")).unwrap();
        let expected_packet=std::fs::read(base.join("expected-packet.f32le")).unwrap();
        let expected:Vec<f32>=std::fs::read(base.join("expected-output.f32le")).unwrap()
            .chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut video=crate::playback_native::NativeReader::software(std::io::Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..6 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(media),Default::default()).unwrap();
        let mut decoder=source.make_decoder().unwrap();
        let mut backend=CpalBackend::new();backend.sample_rate=8000;backend.channels=1;
        *backend.shared.ring.lock().unwrap()=Ring::new(1);
        for expected_pts in [0,100_000_000] {
            let encoded=source.next_packet().unwrap().unwrap();
            assert_eq!(encoded.pts,expected_pts);
            let frame=decoder.decode_packet(&encoded.data,encoded.pts,encoded.duration as u64).unwrap().unwrap();
            let packet=source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap();
            assert_eq!(packet.data,expected_packet);
            assert_eq!(packet.presentation_time(),Duration::from_nanos(expected_pts as u64));
            backend.push(packet).unwrap();
        }
        assert!(source.next_packet().unwrap().is_none());
        let mut output=vec![0.0;960];backend.shared.fill(&mut output);
        assert_eq!(output,expected,"packet gap was collapsed rather than rendered as80ms silence");
        assert_eq!(backend.position(),Duration::from_millis(120));
    }
}
#[cfg(test)]
mod explicit_flac_speaker_route_regressions {
    use super::*;
    #[test]
    fn actual_cpal_push_ring_and_callback_follow_authored_flac_speaker_oracles() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("flac-layout/manifest.json")).unwrap())
                .unwrap();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let weights = [
            [1., 0.],
            [0., 1.],
            [k, k],
            [0., 0.],
            [k, 0.],
            [0., k],
            [k, 0.],
            [0., k],
            [k, k],
            [k, 0.],
            [0., k],
            [k, k],
            [k, 0.],
            [k, k],
            [0., k],
            [k, 0.],
            [k, k],
            [0., k],
        ];
        for case in manifest["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let stem = case["source"].as_str().unwrap();
            let mask = case["mask"].as_u64().unwrap() as u32;
            let raw = std::fs::read(root.join(format!("flac-native/{stem}.flac"))).unwrap();
            let index = fvid_media::owned_flac::Flac::parse(&raw, &Default::default()).unwrap();
            let channels = usize::from(index.channels);
            let scale = (1u64 << (index.bits_per_sample - 1)) as f64;
            let integer = std::fs::read(root.join(format!("flac-native/{stem}.s32le"))).unwrap();
            let golden: Vec<f32> = integer
                .chunks_exact(4)
                .map(|v| (i32::from_le_bytes(v.try_into().unwrap()) as f64 / scale) as f32)
                .collect();
            let peak = golden.iter().copied().map(f32::abs).fold(0., f32::max);
            let mut expected = Vec::new();
            for frame in golden.chunks_exact(channels) {
                // A zero mask names no roles. Preserve the committed audible
                // fallback independently of the production routing function.
                let mut stereo = if mask == 0 {
                    match channels {
                        1 => [frame[0]; 2],
                        2 => [frame[0], frame[1]],
                        _ => [frame.iter().sum::<f32>() / channels as f32; 2],
                    }
                } else {
                    [0f32; 2]
                };
                let mut bits = mask;
                for &sample in frame {
                    if bits == 0 {
                        break;
                    }
                    let speaker = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let [left, right] = if channels == 1 && speaker == 2 {
                        [1., 1.]
                    } else {
                        weights[speaker]
                    };
                    stereo[0] += sample * left;
                    stereo[1] += sample * right;
                }
                expected.extend(stereo);
            }
            let decoded = fvid_codecs::flac::decode(
                index.packet(0),
                fvid_codecs::flac::Config {
                    sample_rate: 8000,
                    channels: index.channels,
                    bits: index.bits_per_sample as u8,
                },
            )
            .unwrap();
            let pcm: Vec<u8> = decoded
                .samples
                .into_iter()
                .flat_map(|v| ((f64::from(v) / scale) as f32).to_le_bytes())
                .collect();
            let mut backend = CpalBackend::new();
            backend.channels = 2;
            backend.sample_rate = 8000;
            *backend.shared.ring.lock().unwrap() = Ring::new(2);
            backend.stereo_router = Some(
                fvid_media::owned_stereo_router::StereoRouter::new(index.channels, mask).unwrap(),
            );
            for base in [0, 8000] {
                backend.flush(Duration::from_secs(base / 8000)).unwrap();
                backend
                    .push(AudioPacket {
                        data: pcm.clone(),
                        pts: base,
                        timebase_num: 1,
                        timebase_den: 8000,
                    })
                    .unwrap();
                let mut actual = vec![0f32; expected.len()];
                backend.shared.fill_timed(&mut actual, 8000, Duration::ZERO);
                for (&actual, &wanted) in actual.iter().zip(&expected) {
                    assert!(
                        (actual - wanted).abs() <= 16. * f32::EPSILON * peak * channels as f32,
                        "{name}: {actual} vs {wanted}"
                    );
                }
                assert_eq!(backend.shared.ring.lock().unwrap().write_pos, 64);
                assert_eq!(backend.position(), Duration::from_millis(base / 8 + 8));
            }
        }
    }
}
#[cfg(test)]
mod physical_layout_smoke {
    use super::*;
    #[test]
    #[ignore = "requires a physical default output device; opens paused and emits no sound"]
    fn real_device_flac_layout_smoke() {
        let mut backend = CpalBackend::new();
        backend
            .start_with_layout(
                AudioSpec {
                    sample_rate: 48000,
                    channels: 6,
                    format: SampleFormat::F32,
                },
                Some(0x3f),
            )
            .unwrap();
        assert!(backend.stream.is_some());
        assert_eq!(backend.channels, 2);
        assert_eq!(backend.stereo_router.as_ref().unwrap().input_channels(), 6);
        assert_eq!(backend.shared.ring.lock().unwrap().channels, 2);
        backend.stop().unwrap();
        assert!(backend.stream.is_none());
    }
}
#[cfg(test)]
mod explicit_aac_speaker_route_regressions {
    use super::*;
    #[test]
    fn actual_cpal_ring_and_callbacks_follow_independent_aac_pce_stereo_oracles() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("aac-device-layout/manifest.json")).unwrap(),
        )
        .unwrap();
        for case in manifest["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let raw = std::fs::read(root.join(format!("aac-height-{name}.aac"))).unwrap();
            let source = crate::playback_aac::Aac::parse(&raw, &Default::default()).unwrap();
            let mut decoder =
                fvid_media::owned_aac::NativeAacDecoder::new(&source.configuration).unwrap();
            let mut pcm = Vec::new();
            for i in 0..source.packets() {
                pcm.extend(
                    decoder
                        .decode(source.packet(i))
                        .unwrap()
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
            let router = fvid_media::owned_aac::speaker_routing::stereo_router(
                &source.configuration,
                48000,
                source.channels,
                false,
            )
            .unwrap();
            let mut backend = CpalBackend::new();
            backend.channels = 2;
            backend.sample_rate = 48000;
            *backend.shared.ring.lock().unwrap() = Ring::new(2);
            backend.stereo_router = Some(router);
            let expected =
                std::fs::read(root.join(format!("aac-device-layout/{name}.stereo.f32le"))).unwrap();
            backend
                .push(AudioPacket {
                    data: pcm,
                    pts: 0,
                    timebase_num: 1,
                    timebase_den: 48000,
                })
                .unwrap();
            let mut actual = vec![0f32; expected.len() / 4];
            for block in actual.chunks_mut(2000) {
                backend.shared.fill_timed(block, 48000, Duration::ZERO);
            }
            let mut squared = 0f64;
            let mut peak = 0f32;
            for (&a, b) in actual.iter().zip(expected.chunks_exact(4)) {
                let b = f32::from_le_bytes(b.try_into().unwrap());
                let delta = (a - b).abs();
                peak = peak.max(delta);
                squared += f64::from(delta).powi(2);
            }
            assert!(
                peak < 2e-6 && (squared / actual.len() as f64).sqrt() < 4e-7,
                "{name}: callback peak{peak}"
            );
            assert_eq!(backend.shared.ring.lock().unwrap().write_pos, 6144);
            assert_eq!(backend.position(), Duration::from_millis(128));
        }
    }
    #[test]
    #[ignore = "requires physical default output; opens paused without emitting sound"]
    fn real_device_aac_pce_layout_smoke() {
        let asc = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/aac-device-layout/normal-ten.asc"),
        )
        .unwrap();
        let router =
            fvid_media::owned_aac::speaker_routing::stereo_router(&asc, 48000, 10, false).unwrap();
        assert_eq!(router.source_mask(), None);
        let mut backend = CpalBackend::new();
        backend
            .start_with_router(
                AudioSpec {
                    sample_rate: 48000,
                    channels: 10,
                    format: SampleFormat::F32,
                },
                Some(router),
            )
            .unwrap();
        assert!(backend.stream.is_some());
        assert_eq!(backend.channels, 2);
        assert_eq!(backend.stereo_router.as_ref().unwrap().input_channels(), 10);
        assert_eq!(backend.shared.ring.lock().unwrap().channels, 2);
        backend.stop().unwrap();
    }
}

#[cfg(test)]
mod signed_aac_timeline_regressions {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn signed_aac_windows_reach_the_cpal_callback_at_their_original_sample_positions() {
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let raw=std::fs::read(root.join("aac-device-layout/normal-ten.stereo.f32le")).unwrap();
        let stereo:Vec<f32>=raw.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        for(name,head_packet,head,tail,offset,gap)in [("head",0,17,0,0,0),("middle-head",2,17,13,0,0),("head-gap",2,17,13,0,4768),("offset-head",0,17,13,2400,0),("full-head",0,1024,13,0,0)] {
            let end=6144+offset+gap-tail;let mut expected=vec![0.;end*2];
            for i in 0..6 {
                let first=if i==head_packet{head}else{0};let last=if i==5{1024-tail}else{1024};
                let at=i*1024+offset+if i>=2{gap}else{0};
                expected[(at+first)*2..(at+last)*2].copy_from_slice(&stereo[(i*1024+first)*2..(i*1024+last)*2]);
            }
            for anchor in [0,2400] {
                let bytes=std::fs::read(root.join(format!("aac-clock-{name}.mkv"))).unwrap();
                let mut source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap();
                let router=source.stereo_router().unwrap().unwrap();
                source.seek_to(anchor*1_000_000_000/48000);let mut decoder=source.make_decoder().unwrap();
                let mut backend=CpalBackend::new();backend.channels=2;backend.sample_rate=48000;
                *backend.shared.ring.lock().unwrap()=Ring::new(2);backend.stereo_router=Some(router);
                backend.flush(Duration::from_nanos(anchor as u64*1_000_000_000/48000)).unwrap();
                while let Some(p)=source.next_packet().unwrap(){
                    if let Some(f)=decoder.decode_packet(&p.data,p.pts,p.duration.max(0) as u64).unwrap(){
                        if let Some(out)=source.present_decoded(f.packet,f.source_pts).unwrap(){backend.push(out).unwrap();}
                    }
                }
                backend.begin_drain().unwrap();let mut actual=vec![0f32;(end-anchor as usize)*2];
                for block in actual.chunks_mut(800){backend.shared.fill_timed(block,48000,Duration::ZERO);}
                for(a,b)in actual.iter().zip(&expected[anchor as usize*2..]){assert!((a-b).abs()<2e-6,"{name}/{anchor}: {a}/{b}");}
                assert_eq!(backend.shared.ring.lock().unwrap().readable(),0);
                // Device elapsed time rounds to nearest ns; packet PTS floors.
                assert_eq!(backend.position(),Duration::from_nanos((end as u64*1_000_000_000+24000)/48000));
            }
        }
    }
}

#[cfg(test)]
mod eac3_codec_stereo_device_path {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn codec_stereo_survives_cpal_routing_ring_and_callback() {
        let base=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-player-stereo");
        for name in ["l1-f1-c0-s7", "l7-f1-c0-s1", "l5-f0-c3-s4"] {
            let raw=std::fs::read(base.join(format!("{name}.ec3"))).unwrap();
            let mut source=crate::playback_ac3::Ac3AudioReader::open(std::io::Cursor::new(raw),Default::default()).unwrap();
            source.set_eac3_stereo_output(true).unwrap();
            let router=source.stereo_router().unwrap().unwrap();
            assert_eq!(router.coefficients(), &[[1.,0.],[0.,1.]]);
            let mut decoder=source.make_decoder().unwrap();
            decoder.set_eac3_stereo_downmix(crate::codec::eac3_decoder::StereoDownmixPolicy {mode:crate::codec::eac3_decoder::StereoDownmixMode::LtRt,headroom:true});
            let mut backend=CpalBackend::new();backend.channels=2;backend.sample_rate=48000;
            *backend.shared.ring.lock().unwrap()=Ring::new(2);backend.stereo_router=Some(router);
            let mut expected=Vec::new();
            while let Some(p)=source.next_packet().unwrap() {
                if let Some(frame)=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap() {
                    let packet=source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap();
                    expected.extend(packet.data.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())));
                    backend.push(packet).unwrap();
                }
            }
            while let Some(frame)=decoder.finish_packet().unwrap() {
                let packet=source.present_decoded(frame.packet,frame.source_pts).unwrap().unwrap();
                expected.extend(packet.data.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())));
                backend.push(packet).unwrap();
            }
            assert_eq!(expected.len(),2048*2);
            let mut actual=vec![0.;expected.len()];
            for chunk in actual.chunks_mut(254) {backend.shared.fill_timed(chunk,48000,Duration::ZERO);}
            assert!(actual==expected,"CPAL altered codec stereo for {name}");
        }
    }
}
