//! Local window playback. Software decode to a display-sized BGRA buffer, optional
//! device audio, and an egui window. Not a streaming server and not a hardware presenter.
use super::*;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

include!("owned_play_controls_impl.rs");

struct VideoFrame {
    pts_us: i64,
    duration_us: i64,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
    color_trc: u32,
}

struct Shared {
    video: Mutex<VecDeque<VideoFrame>>,
    video_cv: Condvar,
    audio: Mutex<VecDeque<f32>>,
    audio_cv: Condvar,
    error: Mutex<Option<String>>,
    quit: AtomicBool,
    ready: AtomicBool,
    finished: AtomicBool,
    paused: AtomicBool,
    use_audio_clock: AtomicBool,
    audio_eof: AtomicBool,
    origin_us: AtomicI64,
    sample_rate: AtomicU32,
    channels: AtomicU32,
    played_samples: AtomicU64,
    source_width: AtomicU32,
    source_height: AtomicU32,
    duration_us: AtomicI64,
    /// Media time to seek to, or -1 when idle. AV timestamps are derived from this.
    seek_us: AtomicI64,
    /// Linear output gain. 1000 is unity.
    volume_milli: AtomicU32,
    /// Playback rate in thousandths. 1000 is 1×.
    rate_milli: AtomicU32,
    rate_phase: AtomicU32,
    rate_held: AtomicBool,
    muted: AtomicBool,
    held_audio: Mutex<Vec<f32>>,
    cues: Mutex<VecDeque<SubtitleCue>>,
    bitmaps: Mutex<VecDeque<BitmapSubtitle>>,
    audio_count: AtomicU32,
    subtitle_count: AtomicU32,
    audio_ordinal: AtomicI32,
    subtitle_ordinal: AtomicI32,
    track_gen: AtomicU32,
    watch_us: AtomicI64,
    external_cues: Mutex<Vec<SubtitleCue>>,
    external_sub: AtomicBool,
    device_name: Mutex<String>,
    media_title: Mutex<String>,
    chapters: Mutex<Vec<i64>>,
    audio_delay_us: AtomicI64,
    audio_skew_frames: AtomicI64,
    /// Last seen `AVFrame.color_trc` (PQ/HLG detection for play HDR).
    color_trc: AtomicU32,
    eq_gains_milli: [AtomicI32; EQ_BAND_COUNT],
    tone: Mutex<Vec<GraphicEqState>>,
    /// Per-channel Bass/Mid/Treble filter state.
    tone_bands: Mutex<Vec<ToneState>>,
    bass_milli: AtomicI32,
    mid_milli: AtomicI32,
    treble_milli: AtomicI32,
    audio_reset: AtomicBool,
    audio_channel: AtomicU32,
    /// Stereo balance. 1000 is center, 0 full left, 2000 full right.
    balance_milli: AtomicI32,
    /// Stereo width. 1000 is normal, 0 mono, 2000 double-wide.
    width_milli: AtomicI32,
    /// VLC-style peak compressor before balance/width.
    compressor_on: AtomicBool,
    /// Headphone crossfeed strength 0..=1000.
    crossfeed_milli: AtomicI32,
    /// When true, graphic EQ is skipped in the audio path.
    eq_bypass: AtomicBool,
    /// EQ preamp milli-gain around unity (0 = 0 dB).
    eq_preamp_milli: AtomicI32,
    /// Headphone spatializer strength 0..=2000.
    spatializer_milli: AtomicI32,
    /// ReplayGain linear milli-gain (1000 = unity).
    replaygain_milli: AtomicI32,
    /// Fold multichannel PCM to stereo before balance/width.
    surround_downmix: AtomicBool,
    /// VLC-style volume normalizer (peak follower + makeup gain).
    normalizer_on: AtomicBool,
    /// Smoothed peak ×1000 for the normalizer.
    normalizer_peak_milli: AtomicU32,
    /// Instantaneous output peak ×1000 for VU meter OSD.
    vu_peak_milli: AtomicU32,
}

struct Finish(Arc<Shared>);

impl Drop for Finish {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.video_cv.notify_all();
        self.0.audio_cv.notify_all();
    }
}

struct Clock {
    origin_us: i64,
    anchor: Instant,
    paused_at: Option<Instant>,
    rate_milli: u32,
}

impl Clock {
    fn new(origin_us: i64, rate_milli: u32) -> Self {
        Self {
            origin_us,
            anchor: Instant::now(),
            paused_at: None,
            rate_milli,
        }
    }

    fn pause(&mut self) {
        if self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
    }

    fn resume(&mut self) {
        if self.paused_at.take().is_some() {
            let held = self.now();
            self.origin_us = held;
            self.anchor = Instant::now();
        }
    }

    fn now(&self) -> i64 {
        let at = self.paused_at.unwrap_or_else(Instant::now);
        let elapsed = at.saturating_duration_since(self.anchor).as_micros() as i64;
        self.origin_us
            .saturating_add(scale_elapsed_us(elapsed.max(0), self.rate_milli))
    }

    fn set_rate(&mut self, rate_milli: u32) {
        let held = self.now();
        self.rate_milli = rate_milli.clamp(RATE_MIN_MILLI, RATE_MAX_MILLI);
        self.jump(held);
    }

    fn snap(&mut self, media_us: i64) {
        if self.paused_at.is_some() {
            return;
        }
        self.jump(media_us);
    }

    fn jump(&mut self, media_us: i64) {
        self.origin_us = media_us;
        self.anchor = Instant::now();
        if self.paused_at.is_some() {
            self.paused_at = Some(self.anchor);
        }
    }
}

/// Fit inside 1920×1080, preserving aspect. Smaller frames stay at native size.
pub(crate) fn fit_display_size(width: i32, height: i32) -> Result<(u32, u32)> {
    if width <= 0 || height <= 0 || width > 8192 || height > 4320 {
        return Err("video frame size must be within 1..=8192 x 1..=4320".into());
    }
    if width <= MAX_W && height <= MAX_H {
        return Ok((width as u32, height as u32));
    }
    let width_bound = i64::from(MAX_W) * i64::from(height) <= i64::from(MAX_H) * i64::from(width);
    let (dw, dh) = if width_bound {
        let dh =
            (i64::from(height) * i64::from(MAX_W) / i64::from(width)).clamp(1, i64::from(MAX_H));
        (i64::from(MAX_W), dh)
    } else {
        let dw =
            (i64::from(width) * i64::from(MAX_H) / i64::from(height)).clamp(1, i64::from(MAX_W));
        (dw, i64::from(MAX_H))
    };
    Ok((dw as u32, dh as u32))
}

/// The playback runtime: software decode, cpal audio output, and the egui
/// window. It is the only part of this module that links a GUI toolkit, so the
/// whole block is a `player`-gated inner module; a library build that leaves the
/// feature off compiles none of it.
#[cfg(feature = "player")]
mod runtime {
    use super::*;
    use crate::lossless::{Codec, Frame};
    use cpal::SampleFormat;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use eframe::egui;
    use std::ffi::CStr;
    use std::ptr;
    use std::sync::MutexGuard;
    use std::thread;
    use std::time::Duration;

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn wait_timeout<'a, T>(cv: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match cv.wait_timeout(guard, Duration::from_millis(50)) {
        Ok((guard, _)) => guard,
        Err(poison) => poison.into_inner().0,
    }
}

/// Open an egui window and play one local file until the user closes it.
pub fn play(path: &Path, options: PlayOptions) -> Result<PlayStats> {
    play_paths(std::slice::from_ref(&path.to_path_buf()), options)
}

/// Open an egui window and play a playlist until the user closes it.
/// Space pauses. Escape or Q closes the window. Left and right seek 10 seconds.
/// Up and down change volume. M mutes. F toggles fullscreen. `[` and `]` change
/// speed. Period steps one frame while paused. S saves a bitmap snapshot.
/// Drop files, or use Open, to replace the playlist. Display is capped at 1920×1080.
pub fn play_paths(paths: &[PathBuf], options: PlayOptions) -> Result<PlayStats> {
    if paths.is_empty() {
        return Err("play requires an input file".into());
    }
    let paths = expand_play_inputs(paths)?;
    for path in &paths {
        if path.to_str().is_none() {
            return Err("path must be UTF-8".into());
        }
        if !is_playback_url(path.to_str().unwrap_or("")) && !path.is_file() {
            return Err(
                "media input must be an existing local file or a supported playback URL".into(),
            );
        }
    }
    let outcome = Arc::new(Mutex::new(None));
    let outcome_app = Arc::clone(&outcome);
    let paths = paths.to_vec();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("fvid")
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([480.0, 360.0])
            .with_drag_and_drop(true),
        run_and_return: true,
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "fvid",
        native,
        Box::new(move |_cc| Ok(Box::new(PlayerApp::open(paths, options, outcome_app)))),
    )
    .map_err(|err| err.to_string())?;
    match lock(&outcome).take() {
        Some(Ok(stats)) => Ok(stats),
        Some(Err(err)) => Err(err),
        None => Err("playback window closed before it started".into()),
    }
}

fn start_audio(
    shared: Arc<Shared>,
    wanted: Option<&str>,
) -> std::result::Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = if let Some(wanted) = wanted.filter(|name| !name.trim().is_empty()) {
        let names = audio_output_devices();
        let matched = find_audio_device(&names, wanted).ok_or_else(|| {
            format!(
                "audio output device not found: {wanted}; available: {}",
                if names.is_empty() {
                    "(none)".into()
                } else {
                    names.join(", ")
                }
            )
        })?;
        host.output_devices()
            .map_err(|err| err.to_string())?
            .find(|device| device.name().ok().as_deref() == Some(matched))
            .ok_or_else(|| format!("audio output device not found: {wanted}"))?
    } else {
        host.default_output_device()
            .ok_or("no default audio output device")?
    };
    let chosen = device.name().unwrap_or_else(|_| "default".into());
    *lock(&shared.device_name) = chosen.clone();
    eprintln!("fvid play: audio device: {chosen}");
    let supported = device
        .default_output_config()
        .map_err(|err| err.to_string())?;
    let channels = supported.channels();
    if !(1..=8).contains(&channels) {
        return Err(format!("unsupported output channel count {channels}"));
    }
    let rate = supported.sample_rate().0;
    if rate == 0 {
        return Err("audio output sample rate is 0".into());
    }
    let format = supported.sample_format();
    let config = supported.config();
    shared.sample_rate.store(rate, Ordering::Relaxed);
    shared
        .channels
        .store(u32::from(channels), Ordering::Relaxed);
    let err_fn = |err| eprintln!("fvid play audio: {err}");
    let stream = match format {
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| fill_audio(&shared, data, |sample, dst| *dst = sample),
                err_fn,
                None,
            )
            .map_err(|err| err.to_string())?,
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    fill_audio(&shared, data, |sample, dst| {
                        *dst = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                    });
                },
                err_fn,
                None,
            )
            .map_err(|err| err.to_string())?,
        SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    fill_audio(&shared, data, |sample, dst| {
                        let unit = (sample.clamp(-1.0, 1.0) + 1.0) * 0.5;
                        *dst = (unit * f32::from(u16::MAX)) as u16;
                    });
                },
                err_fn,
                None,
            )
            .map_err(|err| err.to_string())?,
        other => return Err(format!("unsupported output sample format {other:?}")),
    };
    Ok(stream)
}

fn fill_audio<T>(shared: &Shared, data: &mut [T], mut write: impl FnMut(f32, &mut T)) {
    let gain = if shared.muted.load(Ordering::Relaxed) {
        0.0
    } else {
        shared.volume_milli.load(Ordering::Relaxed) as f32 / 1000.0
    };
    if shared.paused.load(Ordering::Relaxed) {
        for sample in data.iter_mut() {
            write(0.0, sample);
        }
        return;
    }
    let channels = shared.channels.load(Ordering::Relaxed).max(1) as usize;
    let rate = shared
        .rate_milli
        .load(Ordering::Relaxed)
        .clamp(RATE_MIN_MILLI, RATE_MAX_MILLI);
    let frames_out = data.len() / channels;
    let mut phase = shared.rate_phase.load(Ordering::Relaxed);
    let mut queue = lock(&shared.audio);
    let mut held = lock(&shared.held_audio);
    if held.len() != channels {
        held.resize(channels, 0.0);
    }
    let mut have_held = shared.rate_held.load(Ordering::Relaxed);
    let mut consumed = 0u64;
    let mut written = 0usize;
    let mut skew = shared.audio_skew_frames.load(Ordering::Relaxed);
    let mut gains = eq_unity_gains();
    for (slot, gain) in shared.eq_gains_milli.iter().zip(gains.iter_mut()) {
        *gain = slot.load(Ordering::Relaxed);
    }
    let eq_bypass = shared.eq_bypass.load(Ordering::Relaxed);
    let bass_milli = shared.bass_milli.load(Ordering::Relaxed);
    let mid_milli = shared.mid_milli.load(Ordering::Relaxed);
    let treble_milli = shared.treble_milli.load(Ordering::Relaxed);
    let mut tone = lock(&shared.tone);
    if tone.len() != channels {
        tone.resize(channels, GraphicEqState::default());
    }
    let mut tone_bands = lock(&shared.tone_bands);
    if tone_bands.len() != channels {
        tone_bands.resize(channels, ToneState::default());
    }
    let apply_eq = |sample: f32, channel: usize, tone: &mut [GraphicEqState]| {
        if eq_bypass {
            sample * gain
        } else {
            graphic_eq_step(sample, &mut tone[channel], &gains) * gain
        }
    };
    let channel_mode = match shared.audio_channel.load(Ordering::Relaxed) {
        1 => AudioChannelMode::Left,
        2 => AudioChannelMode::Right,
        3 => AudioChannelMode::Mono,
        4 => AudioChannelMode::Reverse,
        5 => AudioChannelMode::Karaoke,
        _ => AudioChannelMode::Stereo,
    };
    let balance_milli = shared.balance_milli.load(Ordering::Relaxed);
    let eq_preamp_milli = shared.eq_preamp_milli.load(Ordering::Relaxed);
    let spatializer_milli = shared.spatializer_milli.load(Ordering::Relaxed);
    let replaygain_milli = shared.replaygain_milli.load(Ordering::Relaxed);
    let mut frame_buf = vec![0.0f32; channels];
    for frame_index in 0..frames_out {
        let (need, next_phase) = advance_rate_phase(phase, rate);
        let queued_frames = (queue.len() / channels) as u64;
        let (next_skew, silence, dropped) = step_audio_skew(skew, need, queued_frames);
        if dropped > 0 {
            queue.drain(..(dropped as usize) * channels);
        }
        if silence {
            consumed += (skew - next_skew).max(0) as u64;
            skew = next_skew;
            phase = next_phase;
            for channel in 0..channels {
                write(0.0, &mut data[frame_index * channels + channel]);
            }
            written = frame_index + 1;
            continue;
        }
        skew = next_skew;
        let available = queue.len() / channels;
        if need > 0 && available < need as usize {
            break;
        }
        phase = next_phase;
        if need == 0 {
            for channel in 0..channels {
                frame_buf[channel] = if have_held { held[channel] } else { 0.0 };
            }
        } else {
            let drop = (need as usize - 1) * channels;
            queue.drain(..drop);
            for channel in 0..channels {
                let sample = queue.pop_front().unwrap_or(0.0);
                held[channel] = sample;
                frame_buf[channel] = sample;
            }
            have_held = true;
            consumed += u64::from(need);
        }
        apply_tone_frame(
            &mut frame_buf,
            &mut tone_bands,
            bass_milli,
            mid_milli,
            treble_milli,
        );
        if shared.surround_downmix.load(Ordering::Relaxed) {
            downmix_surround_to_stereo(&mut frame_buf, channels);
        }
        for channel in 0..channels {
            frame_buf[channel] = apply_eq(frame_buf[channel], channel, &mut tone);
            frame_buf[channel] = apply_eq_preamp_sample(frame_buf[channel], eq_preamp_milli);
            frame_buf[channel] = apply_replaygain_sample(frame_buf[channel], replaygain_milli);
        }
        if shared.compressor_on.load(Ordering::Relaxed) {
            apply_compressor(&mut frame_buf, true, 0.35, 4.0);
        }
        apply_audio_balance(&mut frame_buf, balance_milli);
        let width_milli = shared.width_milli.load(Ordering::Relaxed);
        apply_stereo_width(&mut frame_buf, width_milli);
        apply_spatializer(&mut frame_buf, spatializer_milli);
        let crossfeed_milli = shared.crossfeed_milli.load(Ordering::Relaxed);
        apply_crossfeed(&mut frame_buf, crossfeed_milli);
        apply_audio_channel(&mut frame_buf, channel_mode);
        let mut frame_peak = 0.0f32;
        for channel in 0..channels {
            frame_peak = frame_peak.max(frame_buf[channel].abs());
        }
        shared.vu_peak_milli.store(
            ((frame_peak * 1_000.0).round() as u32).min(2_000),
            Ordering::Relaxed,
        );
        let mut norm_gain = 1_000u32;
        if shared.normalizer_on.load(Ordering::Relaxed) {
            let prev = shared.normalizer_peak_milli.load(Ordering::Relaxed) as f32 / 1_000.0;
            let next = normalizer_peak_step(prev, frame_peak, 0.35, 0.015);
            shared
                .normalizer_peak_milli
                .store((next * 1_000.0).round() as u32, Ordering::Relaxed);
            norm_gain = normalizer_gain_milli(next.max(frame_peak).max(1e-6), 0.95);
        }
        for channel in 0..channels {
            let boosted = frame_buf[channel] * (norm_gain as f32 / 1_000.0);
            let sample = if gain > 1.0 || norm_gain > 1_000 || boosted.abs() > 1.0 {
                soft_clip_sample(boosted)
            } else {
                boosted.clamp(-1.0, 1.0)
            };
            write(sample, &mut data[frame_index * channels + channel]);
        }
        written = frame_index + 1;
    }
    drop(queue);
    drop(held);
    drop(tone);
    drop(tone_bands);
    shared.rate_phase.store(phase, Ordering::Relaxed);
    shared.rate_held.store(have_held, Ordering::Relaxed);
    shared.audio_skew_frames.store(skew, Ordering::Relaxed);
    for sample in data.iter_mut().skip(written * channels) {
        write(0.0, sample);
    }
    if consumed > 0 {
        shared.played_samples.fetch_add(consumed, Ordering::Relaxed);
        shared.audio_cv.notify_all();
    }
}

fn reset_audio_rate(shared: &Shared) {
    shared.rate_phase.store(0, Ordering::Relaxed);
    shared.rate_held.store(false, Ordering::Relaxed);
    lock(&shared.held_audio).clear();
    lock(&shared.tone).clear();
}

fn rearm_audio_skew(shared: &Shared) {
    let frames = audio_delay_frames(
        shared.audio_delay_us.load(Ordering::Relaxed),
        shared.sample_rate.load(Ordering::Relaxed),
    );
    shared.audio_skew_frames.store(frames, Ordering::Relaxed);
}

fn worker(shared: Arc<Shared>, path: std::path::PathBuf) -> Result<()> {
    let _finish = Finish(Arc::clone(&shared));
    let result = decode_file(&shared, &path);
    if let Err(err) = &result {
        if !shared.quit.load(Ordering::Acquire) {
            let mut slot = lock(&shared.error);
            if slot.is_none() {
                *slot = Some(err.clone());
            }
        }
    }
    result
}

unsafe fn container_metadata_title(ctx: *mut AVFormatContext) -> Option<String> {
    if ctx.is_null() {
        return None;
    }
    unsafe {
        let metadata = (*ctx).metadata;
        if metadata.is_null() {
            return None;
        }
        for key in [c"title", c"TITLE", c"Title"] {
            let entry = av_dict_get(metadata, key.as_ptr(), ptr::null(), 0);
            if entry.is_null() {
                continue;
            }
            let value = CStr::from_ptr((*entry).value).to_string_lossy();
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn decode_file(shared: &Shared, path: &Path) -> Result<()> {
    let mut input = Input::open_playback(path.to_str().ok_or("path must be UTF-8")?)?;
    let meta_title = unsafe { container_metadata_title(input.0) };
    *lock(&shared.media_title) = media_display_title(path, meta_title.as_deref());
    let duration = unsafe { (*input.0).duration };
    if duration >= 0 {
        shared.duration_us.store(duration, Ordering::Relaxed);
    }
    let origin = unsafe {
        let start = (*input.0).start_time;
        if start == NOPTS { 0 } else { start }
    };
    let chapters = input.chapter_starts_us(origin);
    if !chapters.is_empty() {
        eprintln!("fvid play: chapters: {}", chapters.len());
    }
    *lock(&shared.chapters) = chapters;
    let streams = input.streams();
    let video_index = streams
        .iter()
        .position(|&stream| unsafe {
            (*(*stream).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .ok_or("input has no video stream")?;
    let video_tb = unsafe { (*input.streams()[video_index]).time_base };
    if shared.duration_us.load(Ordering::Relaxed) < 0 {
        let ticks = unsafe { (*input.streams()[video_index]).duration };
        if let Some(us) = timestamp_us(ticks, video_tb, 0) {
            if us > 0 {
                shared.duration_us.store(us, Ordering::Relaxed);
            }
        }
    }
    let video_rate = unsafe { (*input.streams()[video_index]).avg_frame_rate };
    let frame_fallback_us = if video_rate.num > 0 && video_rate.den > 0 {
        (i64::from(video_rate.den).saturating_mul(1_000_000) / i64::from(video_rate.num))
            .clamp(1, 1_000_000)
    } else {
        40_000
    };
    let video_decoder = open_decoder(&input, video_index, true)?;
    let audio_streams = if shared.sample_rate.load(Ordering::Relaxed) > 0 {
        stream_indexes(&input, AVMediaType_AVMEDIA_TYPE_AUDIO)
    } else {
        Vec::new()
    };
    let subtitle_streams = stream_indexes(&input, AVMediaType_AVMEDIA_TYPE_SUBTITLE);
    shared
        .audio_count
        .store(audio_streams.len() as u32, Ordering::Relaxed);
    shared.subtitle_count.store(
        subtitle_streams.len() as u32 + subtitle_extra(shared),
        Ordering::Relaxed,
    );
    let mut audio_ordinal = bind_ordinal(
        &audio_streams,
        shared.audio_ordinal.load(Ordering::Acquire),
        false,
        0,
    );
    let mut subtitle_ordinal = bind_ordinal(
        &subtitle_streams,
        shared.subtitle_ordinal.load(Ordering::Acquire),
        true,
        subtitle_extra(shared) as i32,
    );
    shared.audio_ordinal.store(audio_ordinal, Ordering::Relaxed);
    shared
        .subtitle_ordinal
        .store(subtitle_ordinal, Ordering::Relaxed);
    let (mut audio_index, mut audio_decoder, mut audio_tb) =
        bind_decoder(&input, &audio_streams, audio_ordinal)?;
    let (mut subtitle_index, mut subtitle_decoder, mut subtitle_tb) =
        bind_decoder(&input, &subtitle_streams, subtitle_ordinal)?;
    let mut seen_gen = shared.track_gen.load(Ordering::Acquire);
    if audio_decoder.is_some() {
        shared.use_audio_clock.store(true, Ordering::Release);
    } else {
        shared.audio_eof.store(true, Ordering::Release);
    }

    let mut packet = Packet::new()?;
    let mut video_frame = Frame::new()?;
    let mut audio_frame = Frame::new()?;
    let mut scaler = Scaler::default();
    let mut resampler: Option<PlayResampler> = None;
    let mut video_origin: Option<i64> = None;
    let mut audio_aligned = false;
    let mut pending_audio: Vec<(i64, Vec<f32>)> = Vec::new();
    let mut next_video_pts = 0i64;
    let mut signaled = false;

    loop {
        if shared.quit.load(Ordering::Acquire) {
            return Ok(());
        }
        if sync_tracks(
            shared,
            &mut input,
            &video_decoder,
            &audio_streams,
            &subtitle_streams,
            &mut audio_ordinal,
            &mut subtitle_ordinal,
            &mut seen_gen,
            &mut audio_index,
            &mut audio_decoder,
            &mut audio_tb,
            &mut subtitle_index,
            &mut subtitle_decoder,
            &mut subtitle_tb,
            &mut resampler,
            origin,
            &mut video_origin,
            &mut audio_aligned,
            &mut pending_audio,
            &mut next_video_pts,
        )? {
            continue;
        }
        if let Some(target) = take_seek(shared) {
            apply_playback_seek(
                shared,
                &mut input,
                &video_decoder,
                audio_decoder.as_ref(),
                &mut resampler,
                origin,
                target,
                &mut video_origin,
                &mut audio_aligned,
                &mut pending_audio,
                &mut next_video_pts,
            )?;
            if let Some(decoder) = subtitle_decoder.as_ref() {
                unsafe { avcodec_flush_buffers(decoder.0) };
            }
        }
        let available = packet.read(&mut input)?;
        if !available {
            drain_playback(
                shared,
                &video_decoder,
                audio_decoder.as_ref(),
                &mut video_frame,
                &mut audio_frame,
                &mut scaler,
                &mut resampler,
                video_tb,
                audio_tb,
                origin,
                frame_fallback_us,
                &mut video_origin,
                &mut next_video_pts,
                &mut signaled,
                &mut pending_audio,
                &mut audio_aligned,
            )?;
            if !signaled {
                return Err("input produced no video frames".into());
            }
            if shared.duration_us.load(Ordering::Relaxed) < 0 && next_video_pts > 0 {
                shared.duration_us.store(next_video_pts, Ordering::Relaxed);
            }
            loop {
                if shared.quit.load(Ordering::Acquire) {
                    return Ok(());
                }
                if sync_tracks(
                    shared,
                    &mut input,
                    &video_decoder,
                    &audio_streams,
                    &subtitle_streams,
                    &mut audio_ordinal,
                    &mut subtitle_ordinal,
                    &mut seen_gen,
                    &mut audio_index,
                    &mut audio_decoder,
                    &mut audio_tb,
                    &mut subtitle_index,
                    &mut subtitle_decoder,
                    &mut subtitle_tb,
                    &mut resampler,
                    origin,
                    &mut video_origin,
                    &mut audio_aligned,
                    &mut pending_audio,
                    &mut next_video_pts,
                )? {
                    break;
                }
                if let Some(target) = take_seek(shared) {
                    apply_playback_seek(
                        shared,
                        &mut input,
                        &video_decoder,
                        audio_decoder.as_ref(),
                        &mut resampler,
                        origin,
                        target,
                        &mut video_origin,
                        &mut audio_aligned,
                        &mut pending_audio,
                        &mut next_video_pts,
                    )?;
                    if let Some(decoder) = subtitle_decoder.as_ref() {
                        unsafe { avcodec_flush_buffers(decoder.0) };
                    }
                    break;
                }
                let guard = lock(&shared.video);
                let _guard = wait_timeout(&shared.video_cv, guard);
            }
            continue;
        }
        let index = unsafe { (*packet.0).stream_index };
        if index < 0 {
            continue;
        }
        let index = index as usize;
        if index == video_index {
            send_and_receive(&video_decoder, packet.0, || {
                receive_video(
                    shared,
                    &video_decoder,
                    &mut video_frame,
                    &mut scaler,
                    video_tb,
                    origin,
                    frame_fallback_us,
                    &mut video_origin,
                    &mut next_video_pts,
                    &mut signaled,
                    &mut pending_audio,
                    &mut audio_aligned,
                )
            })?;
        } else if audio_index == Some(index) {
            if let Some(decoder) = audio_decoder.as_ref() {
                send_and_receive(decoder, packet.0, || {
                    receive_audio(
                        shared,
                        decoder,
                        &mut audio_frame,
                        &mut resampler,
                        audio_tb.unwrap_or(video_tb),
                        origin,
                        video_origin,
                        &mut audio_aligned,
                        &mut pending_audio,
                    )
                })?;
            }
        } else if subtitle_index == Some(index)
            && let Some(decoder) = subtitle_decoder.as_ref()
        {
            receive_subtitle(
                shared,
                decoder,
                packet.0,
                subtitle_tb.unwrap_or(video_tb),
                origin,
            )?;
        }
    }
}

fn stream_indexes(input: &Input, kind: AVMediaType) -> Vec<usize> {
    input
        .streams()
        .iter()
        .enumerate()
        .filter(|pair| {
            let stream = pair.1;
            unsafe {
                let parameters = (*(*stream)).codecpar;
                (*parameters).codec_type == kind
            }
        })
        .map(|(index, _)| index)
        .collect()
}

fn subtitle_extra(shared: &Shared) -> u32 {
    u32::from(shared.external_sub.load(Ordering::Acquire))
}

fn bind_ordinal(streams: &[usize], ordinal: i32, allow_off: bool, extra: i32) -> i32 {
    let len = streams.len() as i32 + extra;
    if len <= 0 {
        return -1;
    }
    if allow_off {
        ordinal.clamp(-1, len - 1)
    } else {
        ordinal.clamp(0, len - 1)
    }
}

fn bind_decoder(
    input: &Input,
    streams: &[usize],
    ordinal: i32,
) -> Result<(Option<usize>, Option<Codec>, Option<AVRational>)> {
    if ordinal < 0 {
        return Ok((None, None, None));
    }
    let Some(index) = streams.get(ordinal as usize).copied() else {
        return Ok((None, None, None));
    };
    match open_decoder(input, index, false) {
        Ok(codec) => {
            let time_base = unsafe { (*input.streams()[index]).time_base };
            Ok((Some(index), Some(codec), Some(time_base)))
        }
        Err(err) => {
            eprintln!("fvid play: stream {index} disabled ({err})");
            Ok((None, None, None))
        }
    }
}

fn sync_tracks(
    shared: &Shared,
    input: &mut Input,
    video: &Codec,
    audio_streams: &[usize],
    subtitle_streams: &[usize],
    audio_ordinal: &mut i32,
    subtitle_ordinal: &mut i32,
    seen_gen: &mut u32,
    audio_index: &mut Option<usize>,
    audio_decoder: &mut Option<Codec>,
    audio_tb: &mut Option<AVRational>,
    subtitle_index: &mut Option<usize>,
    subtitle_decoder: &mut Option<Codec>,
    subtitle_tb: &mut Option<AVRational>,
    resampler: &mut Option<PlayResampler>,
    origin_us: i64,
    video_origin: &mut Option<i64>,
    audio_aligned: &mut bool,
    pending_audio: &mut Vec<(i64, Vec<f32>)>,
    next_video_pts: &mut i64,
) -> Result<bool> {
    let generation = shared.track_gen.load(Ordering::Acquire);
    let want_audio = shared.audio_ordinal.load(Ordering::Acquire);
    let want_sub = shared.subtitle_ordinal.load(Ordering::Acquire);
    if generation == *seen_gen && want_audio == *audio_ordinal && want_sub == *subtitle_ordinal {
        return Ok(false);
    }
    *seen_gen = generation;
    *audio_ordinal = bind_ordinal(audio_streams, want_audio, false, 0);
    *subtitle_ordinal = bind_ordinal(
        subtitle_streams,
        want_sub,
        true,
        subtitle_extra(shared) as i32,
    );
    shared
        .audio_ordinal
        .store(*audio_ordinal, Ordering::Relaxed);
    shared
        .subtitle_ordinal
        .store(*subtitle_ordinal, Ordering::Relaxed);
    shared.subtitle_count.store(
        subtitle_streams.len() as u32 + subtitle_extra(shared),
        Ordering::Relaxed,
    );
    (*audio_index, *audio_decoder, *audio_tb) = bind_decoder(input, audio_streams, *audio_ordinal)?;
    (*subtitle_index, *subtitle_decoder, *subtitle_tb) =
        bind_decoder(input, subtitle_streams, *subtitle_ordinal)?;
    *resampler = None;
    if audio_decoder.is_some() {
        shared.use_audio_clock.store(true, Ordering::Release);
        shared.audio_eof.store(false, Ordering::Release);
    } else {
        shared.use_audio_clock.store(false, Ordering::Release);
        shared.audio_eof.store(true, Ordering::Release);
    }
    let target = shared.watch_us.load(Ordering::Acquire).max(0);
    apply_playback_seek(
        shared,
        input,
        video,
        audio_decoder.as_ref(),
        resampler,
        origin_us,
        target,
        video_origin,
        audio_aligned,
        pending_audio,
        next_video_pts,
    )?;
    if let Some(decoder) = subtitle_decoder.as_ref() {
        unsafe { avcodec_flush_buffers(decoder.0) };
    }
    Ok(true)
}

struct PlaySubtitle(AVSubtitle);

impl Default for PlaySubtitle {
    fn default() -> Self {
        Self(unsafe { std::mem::zeroed() })
    }
}

impl Drop for PlaySubtitle {
    fn drop(&mut self) {
        unsafe { avsubtitle_free(&mut self.0) };
    }
}

fn receive_subtitle(
    shared: &Shared,
    decoder: &Codec,
    packet: *mut AVPacket,
    time_base: AVRational,
    origin_us: i64,
) -> Result<()> {
    let mut subtitle = PlaySubtitle::default();
    let mut got = 0i32;
    check(
        unsafe { avcodec_decode_subtitle2(decoder.0, &mut subtitle.0, &mut got, packet) },
        "decode subtitle",
    )?;
    if got == 0 {
        return Ok(());
    }
    let pts = unsafe { (*packet).pts };
    let packet_us = timestamp_us(pts, time_base, origin_us).unwrap_or(0);
    let (start_us, end_us) = subtitle_window(
        packet_us,
        subtitle.0.start_display_time,
        subtitle.0.end_display_time,
    );
    let text = subtitle_rect_text(&subtitle.0);
    if !text.is_empty() {
        let mut cues = lock(&shared.cues);
        cues.push_back(SubtitleCue {
            start_us,
            end_us,
            text,
        });
        while cues.len() > 64 {
            cues.pop_front();
        }
    }
    for plane in subtitle_rect_bitmaps(&subtitle.0, start_us, end_us)? {
        let mut bitmaps = lock(&shared.bitmaps);
        bitmaps.push_back(plane);
        while bitmaps.len() > 8 {
            bitmaps.pop_front();
        }
    }
    Ok(())
}

fn subtitle_rect_bitmaps(
    subtitle: &AVSubtitle,
    start_us: i64,
    end_us: i64,
) -> Result<Vec<BitmapSubtitle>> {
    if subtitle.rects.is_null() {
        return Ok(Vec::new());
    }
    let mut planes = Vec::new();
    unsafe {
        for index in 0..subtitle.num_rects {
            let rect = *subtitle.rects.add(index as usize);
            if rect.is_null() {
                continue;
            }
            let rect = &*rect;
            if rect.type_ != AVSubtitleType_SUBTITLE_BITMAP {
                continue;
            }
            if rect.w <= 0 || rect.h <= 0 || rect.data[0].is_null() || rect.linesize[0] <= 0 {
                continue;
            }
            let width = rect.w as u32;
            let height = rect.h as u32;
            let stride = rect.linesize[0] as usize;
            let bytes = stride
                .saturating_mul(height.saturating_sub(1) as usize)
                .saturating_add(width as usize);
            let bitmap = std::slice::from_raw_parts(rect.data[0], bytes);
            let mut palette = [0u8; 1024];
            if !rect.data[1].is_null() && rect.nb_colors > 0 {
                let colors = (rect.nb_colors as usize).min(256) * 4;
                palette[..colors].copy_from_slice(std::slice::from_raw_parts(rect.data[1], colors));
            }
            let pixels = pal8_to_rgba(width, height, bitmap, stride, &palette)?;
            planes.push(BitmapSubtitle {
                start_us,
                end_us,
                x: rect.x,
                y: rect.y,
                width,
                height,
                pixels,
            });
        }
    }
    Ok(planes)
}

fn subtitle_rect_text(subtitle: &AVSubtitle) -> String {
    if subtitle.rects.is_null() {
        return String::new();
    }
    let mut parts = Vec::new();
    unsafe {
        for index in 0..subtitle.num_rects {
            let rect = *subtitle.rects.add(index as usize);
            if rect.is_null() {
                continue;
            }
            let kind = (*rect).type_;
            let raw = if kind == AVSubtitleType_SUBTITLE_TEXT && !(*rect).text.is_null() {
                (*rect).text
            } else if !(*rect).ass.is_null() {
                (*rect).ass
            } else {
                continue;
            };
            let plain = plain_subtitle(&CStr::from_ptr(raw).to_string_lossy());
            if !plain.is_empty() {
                parts.push(plain);
            }
        }
    }
    parts.join("\n")
}

fn take_seek(shared: &Shared) -> Option<i64> {
    let value = shared.seek_us.swap(-1, Ordering::AcqRel);
    (value >= 0).then_some(value)
}

fn apply_playback_seek(
    shared: &Shared,
    input: &mut Input,
    video: &Codec,
    audio: Option<&Codec>,
    resampler: &mut Option<PlayResampler>,
    origin_us: i64,
    target_us: i64,
    video_origin: &mut Option<i64>,
    audio_aligned: &mut bool,
    pending_audio: &mut Vec<(i64, Vec<f32>)>,
    next_video_pts: &mut i64,
) -> Result<()> {
    if let Some(resampler) = resampler.as_mut() {
        let _ = resampler.flush()?;
    }
    let target_us = target_us.max(0);
    unsafe {
        avcodec_flush_buffers(video.0);
        if let Some(audio) = audio {
            avcodec_flush_buffers(audio.0);
        }
        let ts = playback_seek_ts(origin_us, target_us);
        check(
            avformat_seek_file(input.0, -1, i64::MIN, ts, ts, 0),
            "seek playback",
        )?;
    }
    lock(&shared.video).clear();
    lock(&shared.audio).clear();
    lock(&shared.cues).clear();
    lock(&shared.bitmaps).clear();
    reset_audio_rate(shared);
    shared.played_samples.store(0, Ordering::Relaxed);
    rearm_audio_skew(shared);
    shared.audio_eof.store(audio.is_none(), Ordering::Release);
    shared.finished.store(false, Ordering::Release);
    shared.origin_us.store(target_us, Ordering::Release);
    *video_origin = Some(target_us);
    *audio_aligned = false;
    pending_audio.clear();
    *next_video_pts = target_us;
    shared.video_cv.notify_all();
    shared.audio_cv.notify_all();
    Ok(())
}

fn drain_playback(
    shared: &Shared,
    video_decoder: &Codec,
    audio_decoder: Option<&Codec>,
    video_frame: &mut Frame,
    audio_frame: &mut Frame,
    scaler: &mut Scaler,
    resampler: &mut Option<PlayResampler>,
    video_tb: AVRational,
    audio_tb: Option<AVRational>,
    origin: i64,
    frame_fallback_us: i64,
    video_origin: &mut Option<i64>,
    next_video_pts: &mut i64,
    signaled: &mut bool,
    pending_audio: &mut Vec<(i64, Vec<f32>)>,
    audio_aligned: &mut bool,
) -> Result<()> {
    send_and_receive(video_decoder, ptr::null_mut(), || {
        receive_video(
            shared,
            video_decoder,
            video_frame,
            scaler,
            video_tb,
            origin,
            frame_fallback_us,
            video_origin,
            next_video_pts,
            signaled,
            pending_audio,
            audio_aligned,
        )
    })?;
    if let Some(decoder) = audio_decoder {
        send_and_receive(decoder, ptr::null_mut(), || {
            receive_audio(
                shared,
                decoder,
                audio_frame,
                resampler,
                audio_tb.unwrap_or(video_tb),
                origin,
                *video_origin,
                audio_aligned,
                pending_audio,
            )
        })?;
        if let Some(resampler) = resampler.as_mut() {
            let flushed = resampler.flush()?;
            if !flushed.is_empty() {
                push_audio(shared, &flushed)?;
            }
        }
    }
    shared.audio_eof.store(true, Ordering::Release);
    shared.finished.store(true, Ordering::Release);
    Ok(())
}

fn open_decoder(input: &Input, index: usize, video: bool) -> Result<Codec> {
    unsafe {
        let stream = &*input.streams()[index];
        let parameters = &*stream.codecpar;
        let decoder = avcodec_find_decoder(parameters.codec_id);
        if decoder.is_null() {
            return Err("decoder unavailable".into());
        }
        let codec = Codec(avcodec_alloc_context3(decoder));
        if codec.0.is_null() {
            return Err("decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(codec.0, stream.codecpar),
            "configure decoder",
        )?;
        (*codec.0).pkt_timebase = stream.time_base;
        if video {
            (*codec.0).thread_count = 0;
            (*codec.0).max_pixels = 8192 * 4320;
        }
        check(
            avcodec_open2(codec.0, decoder, ptr::null_mut()),
            "open decoder",
        )?;
        Ok(codec)
    }
}

fn send_and_receive(
    codec: &Codec,
    packet: *mut AVPacket,
    mut receive: impl FnMut() -> Result<()>,
) -> Result<()> {
    for _ in 0..8 {
        // SAFETY: Codec is live. Packet is either a live packet or null for drain.
        let code = unsafe { avcodec_send_packet(codec.0, packet) };
        if code == -libc::EAGAIN {
            receive()?;
            continue;
        }
        check(code, "send packet")?;
        receive()?;
        return Ok(());
    }
    Err("decoder send blocked".into())
}

fn receive_video(
    shared: &Shared,
    codec: &Codec,
    frame: &mut Frame,
    scaler: &mut Scaler,
    time_base: AVRational,
    origin_us: i64,
    fallback_us: i64,
    video_origin: &mut Option<i64>,
    next_pts: &mut i64,
    signaled: &mut bool,
    pending_audio: &mut Vec<(i64, Vec<f32>)>,
    audio_aligned: &mut bool,
) -> Result<()> {
    loop {
        if shared.quit.load(Ordering::Acquire) {
            return Ok(());
        }
        // SAFETY: Decoder and reusable frame are live for this receive call.
        let code = unsafe { avcodec_receive_frame(codec.0, frame.0) };
        if code == -libc::EAGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive video frame")?;
        // SAFETY: The frame was just filled by the decoder.
        let cropped = unsafe { av_frame_apply_cropping(frame.0, 0) };
        if cropped < 0 {
            check(
                unsafe { av_frame_apply_cropping(frame.0, AV_FRAME_CROP_UNALIGNED as i32) },
                "apply frame crop",
            )?;
        }
        let (width, height, pts, duration) = unsafe {
            let raw = &*frame.0;
            (
                raw.width,
                raw.height,
                raw.best_effort_timestamp,
                raw.duration,
            )
        };
        let pts_us = timestamp_us(pts, time_base, origin_us).unwrap_or(*next_pts);
        if video_origin.is_none() {
            *video_origin = Some(pts_us);
            shared.origin_us.store(pts_us, Ordering::Release);
            flush_pending_audio(shared, pending_audio, pts_us, audio_aligned)?;
        }
        let duration_us = timestamp_us(duration, time_base, 0)
            .filter(|value| *value > 0)
            .unwrap_or(fallback_us);
        *next_pts = pts_us.saturating_add(duration_us);
        let scaled = scaler.convert(frame.0)?;
        let color_trc = unsafe { (*frame.0).color_trc as u32 };
        shared.color_trc.store(color_trc, Ordering::Relaxed);
        if shared.source_width.load(Ordering::Relaxed) == 0 {
            shared
                .source_width
                .store(width.max(0) as u32, Ordering::Relaxed);
            shared
                .source_height
                .store(height.max(0) as u32, Ordering::Relaxed);
        }
        push_video(
            shared,
            VideoFrame {
                pts_us,
                duration_us,
                width: scaled.width,
                height: scaled.height,
                pixels: scaled.pixels,
                color_trc,
            },
            signaled,
        )?;
    }
}

fn receive_audio(
    shared: &Shared,
    codec: &Codec,
    frame: &mut Frame,
    resampler: &mut Option<PlayResampler>,
    time_base: AVRational,
    origin_us: i64,
    video_origin: Option<i64>,
    aligned: &mut bool,
    pending: &mut Vec<(i64, Vec<f32>)>,
) -> Result<()> {
    loop {
        if shared.quit.load(Ordering::Acquire) {
            return Ok(());
        }
        // SAFETY: Decoder and reusable frame are live for this receive call.
        let code = unsafe { avcodec_receive_frame(codec.0, frame.0) };
        if code == -libc::EAGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive audio frame")?;
        if shared.audio_reset.swap(false, Ordering::AcqRel) {
            *resampler = None;
            lock(&shared.audio).clear();
            lock(&shared.held_audio).clear();
            shared.rate_held.store(false, Ordering::Relaxed);
            shared.audio_cv.notify_all();
        }
        if resampler.is_none() {
            *resampler = Some(unsafe {
                PlayResampler::open(
                    frame.0,
                    shared.sample_rate.load(Ordering::Relaxed) as i32,
                    shared.channels.load(Ordering::Relaxed) as i32,
                )?
            });
        }
        let samples = resampler.as_mut().unwrap().convert(frame.0)?;
        let pts = unsafe { (*frame.0).best_effort_timestamp };
        let pts_us = timestamp_us(pts, time_base, origin_us).unwrap_or(0);
        if let Some(video_origin) = video_origin {
            if !pending.is_empty() {
                flush_pending_audio(shared, pending, video_origin, aligned)?;
            }
            let chunk = take_aligned(
                aligned,
                video_origin,
                shared.sample_rate.load(Ordering::Relaxed) as i32,
                shared.channels.load(Ordering::Relaxed).max(1) as usize,
                pts_us,
                &samples,
            );
            if !chunk.is_empty() {
                push_audio(shared, &chunk)?;
            }
        } else if !samples.is_empty() {
            let rate = shared.sample_rate.load(Ordering::Relaxed).max(1) as usize;
            let channels = shared.channels.load(Ordering::Relaxed).max(1) as usize;
            let queued: usize = pending.iter().map(|(_, chunk)| chunk.len()).sum();
            if queued.saturating_add(samples.len())
                > rate.saturating_mul(channels).saturating_mul(2)
            {
                return Err("buffered too much audio before the first video frame".into());
            }
            pending.push((pts_us, samples));
        }
    }
}

fn flush_pending_audio(
    shared: &Shared,
    pending: &mut Vec<(i64, Vec<f32>)>,
    video_origin: i64,
    aligned: &mut bool,
) -> Result<()> {
    let rate = shared.sample_rate.load(Ordering::Relaxed) as i32;
    let channels = shared.channels.load(Ordering::Relaxed).max(1) as usize;
    for (pts_us, samples) in pending.drain(..) {
        let chunk = take_aligned(aligned, video_origin, rate, channels, pts_us, &samples);
        if !chunk.is_empty() {
            push_audio(shared, &chunk)?;
        }
    }
    Ok(())
}

fn timestamp_us(ticks: i64, time_base: AVRational, origin_us: i64) -> Option<i64> {
    if ticks == NOPTS || time_base.num <= 0 || time_base.den <= 0 {
        return None;
    }
    let absolute = super::rescale_owned(
        ticks, time_base, AVRational { num: 1, den: 1_000_000 },
    ).ok()?;
    Some(absolute.saturating_sub(origin_us))
}

fn push_video(shared: &Shared, frame: VideoFrame, signaled: &mut bool) -> Result<()> {
    let mut queue = lock(&shared.video);
    loop {
        if shared.quit.load(Ordering::Acquire) {
            return Ok(());
        }
        if queue.len() < VIDEO_QUEUE {
            queue.push_back(frame);
            if !*signaled {
                shared.ready.store(true, Ordering::Release);
                *signaled = true;
            }
            shared.video_cv.notify_all();
            return Ok(());
        }
        queue = wait_timeout(&shared.video_cv, queue);
    }
}

fn push_audio(shared: &Shared, samples: &[f32]) -> Result<()> {
    let rate = shared.sample_rate.load(Ordering::Relaxed).max(1) as usize;
    let channels = shared.channels.load(Ordering::Relaxed).max(1) as usize;
    let cap = rate.saturating_mul(channels).saturating_mul(AUDIO_SECONDS);
    let mut queue = lock(&shared.audio);
    let mut rest = samples;
    while !rest.is_empty() {
        if shared.quit.load(Ordering::Acquire) {
            return Ok(());
        }
        let room = cap.saturating_sub(queue.len());
        if room == 0 {
            queue = wait_timeout(&shared.audio_cv, queue);
            continue;
        }
        let n = room.min(rest.len());
        queue.extend(rest[..n].iter().copied());
        rest = &rest[n..];
        shared.audio_cv.notify_all();
    }
    Ok(())
}

struct Session {
    path: PathBuf,
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<Result<()>>>,
    stream: Option<cpal::Stream>,
    clock: Clock,
    last_clock: i64,
    audio_switched: bool,
    media_now: i64,
    presented: u64,
    skipped: u64,
    frame: Option<VideoFrame>,
    dirty: bool,
    discard_until: Option<i64>,
}

struct PlayerApp {
    options: PlayOptions,
    playlist: Vec<PathBuf>,
    playlist_index: usize,
    advance_guard: bool,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    fitted: bool,
    error: Option<String>,
    notice: Option<String>,
    notice_at: Option<Instant>,
    notice_text: Option<String>,
    osd_timeout_ms: u64,
    mouse_moved_at: Instant,
    mouse_hide_ms: u64,
    pending: Option<PathBuf>,
    title: String,
    scrub: Option<f32>,
    volume_milli: u32,
    rate_milli: u32,
    muted: bool,
    fullscreen: bool,
    fullscreen_dirty: bool,
    on_top: bool,
    on_top_dirty: bool,
    step_pending: bool,
    snapshots: u32,
    snapshot_format: SnapshotFormat,
    snapshot_dir: Option<PathBuf>,
    snapshot_prefix: String,
    network_cache_ms: u32,
    audio_ordinal: i32,
    subtitle_ordinal: i32,
    logged_sub: String,
    url_text: String,
    jump_text: String,
    ab: Option<AbLoop>,
    stop_us: Option<i64>,
    repeat: RepeatMode,
    subtitle_delay_us: i64,
    secondary_subtitle_delay_us: i64,
    audio_delay_us: i64,
    aspect: AspectMode,
    crop: AspectMode,
    zoom_milli: u32,
    pan_x_px: i32,
    pan_y_px: i32,
    bookmarks: Vec<Bookmark>,
    shuffle: bool,
    order: Vec<usize>,
    order_cursor: usize,
    brightness_milli: i32,
    contrast_milli: i32,
    saturation_milli: i32,
    hue_milli: i32,
    gamma_milli: i32,
    adjust_dirty: bool,
    flip_h: bool,
    flip_v: bool,
    deinterlace: DeinterlaceMode,
    stereo3d: PlayStereo3D,
    show_stats: bool,
    audio_channel: AudioChannelMode,
    balance_milli: i32,
    width_milli: i32,
    compressor_on: bool,
    crossfeed_milli: i32,
    eq_bypass: bool,
    eq_preamp_milli: i32,
    spatializer_milli: i32,
    replaygain_milli: i32,
    gapless: bool,
    crossfade_ms: u32,
    subtitle_color: SubtitleColor,
    playlist_sort: PlaylistSort,
    recent: Vec<PathBuf>,
    post_fx: VideoPostFx,
    spherical_projection: SphericalProjection,
    hdr_maxcll: u32,
    hdr_maxfall: u32,
    color_primaries: u32,
    video_track: u32,
    video_track_count: u32,
    silence_skip: bool,
    intro_end_us: Option<i64>,
    credits_start_us: Option<i64>,
    playlist_filter: String,
    favorites: Vec<PathBuf>,
    remote_control: bool,
    remote_control_port: u16,
    bitperfect: bool,
    night_mode: bool,
    play_queue: Vec<usize>,
    forced_subs_only: bool,
    exclusive_latency_ms: u32,
    ipd_milli: i32,
    vr_display: VrDisplayMode,
    ambisonic: AmbisonicMode,
    cast_protocol: CastProtocol,
    cast_device: String,
    lyric_lines: Vec<LyricLine>,
    ab_slots: [(Option<i64>, Option<i64>); 4],
    pip_enabled: bool,
    horizon_lock: bool,
    horizon_pitch_milli: i32,
    deband_milli: i32,
    spherical_stereo: SphericalStereoLayout,
    tonemap_strength_milli: i32,
    hdr_highlight_desat_milli: i32,
    color_temp_kelvin: i32,
    barrel_k_milli: i32,
    audio_duck_enabled: bool,
    audio_duck_milli: i32,
    waveform_enabled: bool,
    playlist_fade_us: i64,
    hdr10_plus: bool,
    hlg_ootf_gamma_milli: i32,
    denoise_milli: i32,
    dialogue_enhance_milli: i32,
    vectorscope_enabled: bool,
    display_peak_nits: u32,
    hdr_mastering_min_milli: u32,
    hdr_mastering_max_nits: u32,
    gyro_look: bool,
    vr_vignette_milli: i32,
    hdr_black_lift_milli: i32,
    unsharp_milli: i32,
    clipboard_snapshot: bool,
    watch_party: bool,
    watch_party_offset_us: i64,
    auto_horizon: bool,
    ambisonic_order: AmbisonicChannelOrder,
    chromatic_aberration_milli: i32,
    soft_limiter_milli: i32,
    echo_feedback_milli: i32,
    anaglyph_dubois: bool,
    bt2446_tonemap: bool,
    gamut_map_bt709: bool,
    chorus_milli: i32,
    reverb_milli: i32,
    atempo_milli: i32,
    seek_jump: SeekJump,
    surround_downmix: bool,
    scaletempo: bool,
    minimal_interface: bool,
    pitch_milli: i32,
    visualization: VisualizationMode,
    image_duration_secs: u32,
    closed_captions: ClosedCaptionChannel,
    crop_pixels: CropPixels,
    wallpaper_mode: bool,
    subtitle_encoding: SubtitleEncoding,
    teletext_enabled: bool,
    teletext_page: u32,
    aspect_lock: bool,
    snapshot_sequential: bool,
    hw_decode: bool,
    logo_position: LogoPosition,
    logo_opacity_milli: i32,
    mosaic_cols: u32,
    mosaic_rows: u32,
    param_eq_milli: i32,
    amplifier_milli: i32,
    recording: bool,
    record_dir: Option<PathBuf>,
    proxy_mode: ProxyMode,
    proxy_host: String,
    http_auth_user: bool,
    http_auth_password: bool,
    volume_normalizer: bool,
    bass_milli: i32,
    mid_milli: i32,
    treble_milli: i32,
    subtitle_margin_px: i32,
    subtitle_scale_milli: i32,
    subtitle_opacity_milli: i32,
    subtitle_position: SubtitlePosition,
    show_osd: bool,
    marquee_text: String,
    marquee_position: MarqueePosition,
    drop_frame: DropFrameMode,
    display_effect: DisplayEffect,
    hdr_nits: u32,
    controls_autohide_ms: u64,
    http_reconnect: u32,
    http_failures: u32,
    rotate: RotateMode,
    eq_gains_milli: [i32; EQ_BAND_COUNT],
    eq_preset: EqPreset,
    position_display: PositionDisplay,
    sleep_min: u32,
    sleep_deadline_secs: Option<u64>,
    spherical: bool,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
    hdr_tonemap: HdrTonemap,
    hdr_auto_applied: bool,
    outcome: Arc<Mutex<Option<std::result::Result<PlayStats, String>>>>,
}

impl PlayerApp {
    fn open(
        playlist: Vec<PathBuf>,
        options: PlayOptions,
        outcome: Arc<Mutex<Option<std::result::Result<PlayStats, String>>>>,
    ) -> Self {
        let rate_milli = clamp_rate_milli(options.rate);
        let muted = options.muted;
        let fullscreen = options.fullscreen;
        let on_top = options.on_top;
        let audio_ordinal = options.audio_track as i32;
        let subtitle_ordinal = options.subtitle_track;
        let first = playlist.first().cloned().unwrap_or_default();
        let stop_us = options.stop_us;
        let spherical = options.spherical;
        let spherical_projection = options.spherical_projection;
        let yaw_deg_milli = clamp_yaw_milli(options.yaw_deg_milli);
        let pitch_deg_milli = clamp_pitch_milli(options.pitch_deg_milli);
        let roll_deg_milli = clamp_roll_milli(options.roll_deg_milli);
        let fov_deg_milli = clamp_fov_milli(options.fov_deg_milli);
        let hdr_tonemap = options.hdr_tonemap;
        let hdr_auto_applied = !matches!(options.hdr_tonemap, HdrTonemap::Off);
        let stereo3d = options.stereo3d;
        let snapshot_dir = options.snapshot_dir.clone();
        let network_cache_ms = clamp_network_cache_ms(options.network_cache_ms);
        let display_effect = options.display_effect;
        let hdr_nits = clamp_hdr_nits(options.hdr_nits);
        let hdr_maxcll = clamp_hdr_maxcll(options.hdr_maxcll);
        let hdr_maxfall = clamp_hdr_maxcll(options.hdr_maxfall);
        let spherical_stereo = options.spherical_stereo;
        let hdr_mastering_min_milli = options.hdr_mastering_min_milli;
        let hdr_mastering_max_nits = options.hdr_mastering_max_nits;
        let controls_autohide_ms = clamp_controls_autohide_ms(options.controls_autohide_ms);
        let http_reconnect = clamp_http_reconnect(options.http_reconnect);
        let mut app = Self {
            options,
            playlist,
            playlist_index: 0,
            advance_guard: false,
            session: None,
            texture: None,
            fitted: false,
            error: None,
            notice: None,
            notice_at: None,
            notice_text: None,
            osd_timeout_ms: OSD_TIMEOUT_DEFAULT_MS,
            mouse_moved_at: Instant::now(),
            mouse_hide_ms: MOUSE_HIDE_DEFAULT_MS,
            pending: None,
            title: String::new(),
            scrub: None,
            volume_milli: 1000,
            rate_milli,
            muted,
            fullscreen,
            fullscreen_dirty: fullscreen,
            on_top,
            on_top_dirty: on_top,
            step_pending: false,
            snapshots: 0,
            snapshot_format: SnapshotFormat::Bmp,
            snapshot_dir,
            snapshot_prefix: String::new(),
            network_cache_ms,
            audio_ordinal,
            subtitle_ordinal,
            logged_sub: String::new(),
            url_text: String::new(),
            jump_text: String::new(),
            ab: None,
            stop_us,
            repeat: RepeatMode::Off,
            subtitle_delay_us: 0,
            secondary_subtitle_delay_us: 0,
            audio_delay_us: 0,
            aspect: AspectMode::Source,
            crop: AspectMode::Source,
            zoom_milli: 1_000,
            pan_x_px: 0,
            pan_y_px: 0,
            bookmarks: Vec::new(),
            shuffle: false,
            order: Vec::new(),
            order_cursor: 0,
            brightness_milli: 1_000,
            contrast_milli: 1_000,
            saturation_milli: 1_000,
            hue_milli: 1_000,
            gamma_milli: 1_000,
            adjust_dirty: false,
            flip_h: false,
            flip_v: false,
            deinterlace: DeinterlaceMode::Off,
            show_stats: false,
            audio_channel: AudioChannelMode::Stereo,
            balance_milli: BALANCE_CENTER_MILLI,
            width_milli: WIDTH_UNITY_MILLI,
            compressor_on: false,
            crossfeed_milli: 0,
            eq_bypass: false,
            eq_preamp_milli: EQ_PREAMP_DEFAULT_MILLI,
            spatializer_milli: SPATIALIZER_DEFAULT_MILLI,
            replaygain_milli: REPLAYGAIN_UNITY_MILLI,
            gapless: false,
            crossfade_ms: CROSSFADE_DEFAULT_MS,
            subtitle_color: SubtitleColor::White,
            playlist_sort: PlaylistSort::Path,
            recent: Vec::new(),
            post_fx: VideoPostFx::Off,
            spherical_projection,
            hdr_maxcll,
            hdr_maxfall,
            color_primaries: 0,
            video_track: 0,
            video_track_count: 0,
            silence_skip: false,
            intro_end_us: None,
            credits_start_us: None,
            playlist_filter: String::new(),
            favorites: Vec::new(),
            remote_control: false,
            remote_control_port: REMOTE_CONTROL_DEFAULT_PORT,
            bitperfect: false,
            night_mode: false,
            play_queue: Vec::new(),
            forced_subs_only: false,
            exclusive_latency_ms: 50,
            ipd_milli: IPD_DEFAULT_MILLI,
            vr_display: VrDisplayMode::Off,
            ambisonic: AmbisonicMode::Off,
            cast_protocol: CastProtocol::Off,
            cast_device: String::new(),
            lyric_lines: Vec::new(),
            ab_slots: [(None, None); 4],
            pip_enabled: false,
            horizon_lock: false,
            horizon_pitch_milli: 0,
            deband_milli: DEBAND_DEFAULT_MILLI,
            spherical_stereo,
            tonemap_strength_milli: TONEMAP_STRENGTH_DEFAULT_MILLI,
            hdr_highlight_desat_milli: 0,
            color_temp_kelvin: COLOR_TEMP_DAYLIGHT_K,
            barrel_k_milli: 0,
            audio_duck_enabled: false,
            audio_duck_milli: 400,
            waveform_enabled: false,
            playlist_fade_us: 0,
            hdr10_plus: false,
            hlg_ootf_gamma_milli: 1_200,
            denoise_milli: 0,
            dialogue_enhance_milli: 0,
            vectorscope_enabled: false,
            display_peak_nits: HDR_NITS_DEFAULT,
            hdr_mastering_min_milli,
            hdr_mastering_max_nits,
            gyro_look: false,
            vr_vignette_milli: 0,
            hdr_black_lift_milli: 0,
            unsharp_milli: 0,
            clipboard_snapshot: false,
            watch_party: false,
            watch_party_offset_us: 0,
            auto_horizon: false,
            ambisonic_order: AmbisonicChannelOrder::AcnSn3d,
            chromatic_aberration_milli: 0,
            soft_limiter_milli: 1_000,
            echo_feedback_milli: 0,
            anaglyph_dubois: false,
            bt2446_tonemap: false,
            gamut_map_bt709: false,
            chorus_milli: 0,
            reverb_milli: 0,
            atempo_milli: 1_000,
            seek_jump: SeekJump::default(),
            surround_downmix: false,
            scaletempo: true,
            minimal_interface: false,
            pitch_milli: AUDIO_PITCH_UNITY_MILLI,
            visualization: VisualizationMode::Off,
            image_duration_secs: IMAGE_DURATION_DEFAULT_SECS,
            closed_captions: ClosedCaptionChannel::Off,
            crop_pixels: CropPixels::default(),
            wallpaper_mode: false,
            subtitle_encoding: SubtitleEncoding::Utf8,
            teletext_enabled: false,
            teletext_page: TELETEXT_PAGE_DEFAULT,
            aspect_lock: false,
            snapshot_sequential: false,
            hw_decode: false,
            logo_position: LogoPosition::TopLeft,
            logo_opacity_milli: 1_000,
            mosaic_cols: 1,
            mosaic_rows: 1,
            param_eq_milli: 0,
            amplifier_milli: 1_000,
            recording: false,
            record_dir: None,
            proxy_mode: ProxyMode::Off,
            proxy_host: String::new(),
            http_auth_user: false,
            http_auth_password: false,
            volume_normalizer: false,
            bass_milli: TONE_UNITY_MILLI,
            mid_milli: TONE_UNITY_MILLI,
            treble_milli: TONE_UNITY_MILLI,
            subtitle_margin_px: 0,
            subtitle_scale_milli: SUBTITLE_SCALE_UNITY_MILLI,
            subtitle_opacity_milli: SUBTITLE_OPACITY_UNITY_MILLI,
            subtitle_position: SubtitlePosition::Bottom,
            show_osd: true,
            marquee_text: String::new(),
            marquee_position: MarqueePosition::Top,
            drop_frame: DropFrameMode::Late,
            display_effect,
            hdr_nits,
            controls_autohide_ms,
            http_reconnect,
            http_failures: 0,
            rotate: RotateMode::Deg0,
            eq_gains_milli: eq_unity_gains(),
            eq_preset: EqPreset::Flat,
            position_display: PositionDisplay::Elapsed,
            sleep_min: 0,
            sleep_deadline_secs: None,
            spherical,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
            hdr_tonemap,
            hdr_auto_applied,
            stereo3d,
            outcome,
        };
        if let Err(err) = app.start_session(first) {
            app.error = Some(err);
        }
        eprintln!(
            "fvid play: Space pause, left/right seek, up/down volume, M mute, B audio, V subtitles, L A-B loop, R repeat, G/H subtitle delay, J/K audio delay, A aspect, C crop, Z zoom, Ctrl+B bookmark, Ctrl+Alt+B save bookmarks, Ctrl+R shuffle, T on-top, Shift+T time, Ctrl+N vol normalizer, W stereo width, O crossfeed, U compressor, X sleep timer, 3 toggle 360, Ctrl+arrows look, Ctrl+PgUp/Dn FOV, Ctrl+H HDR tonemap, F fullscreen, [ ] speed, . step, S snapshot, Esc quit"
        );
        app
    }

    fn start_session(&mut self, path: PathBuf) -> Result<()> {
        push_recent_path(&mut self.recent, path.clone());
        self.stop_session();
        self.logged_sub.clear();
        self.error = None;
        self.notice = None;
        self.ab = None;
        self.subtitle_delay_us = 0;
        self.secondary_subtitle_delay_us = 0;
        self.audio_delay_us = 0;
        self.bookmarks.clear();
        self.texture = None;
        self.hdr_tonemap = self.options.hdr_tonemap;
        self.hdr_auto_applied = !matches!(self.options.hdr_tonemap, HdrTonemap::Off);
        if path.to_str().is_none() {
            return Err("path must be UTF-8".into());
        }
        if !is_playback_url(path.to_str().unwrap_or("")) && !path.is_file() {
            return Err(
                "media input must be an existing local file or a supported playback URL".into(),
            );
        }
        let external = if let Some(subtitles) = self.options.subtitles.clone() {
            load_subtitle_file(&subtitles)?
        } else {
            Vec::new()
        };
        let has_external = !external.is_empty();
        let shared = Arc::new(Shared {
            video: Mutex::new(VecDeque::new()),
            video_cv: Condvar::new(),
            audio: Mutex::new(VecDeque::new()),
            audio_cv: Condvar::new(),
            error: Mutex::new(None),
            quit: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            use_audio_clock: AtomicBool::new(false),
            audio_eof: AtomicBool::new(false),
            origin_us: AtomicI64::new(0),
            sample_rate: AtomicU32::new(0),
            channels: AtomicU32::new(0),
            played_samples: AtomicU64::new(0),
            source_width: AtomicU32::new(0),
            source_height: AtomicU32::new(0),
            duration_us: AtomicI64::new(-1),
            seek_us: AtomicI64::new(-1),
            volume_milli: AtomicU32::new(self.volume_milli),
            rate_milli: AtomicU32::new(self.rate_milli),
            rate_phase: AtomicU32::new(0),
            rate_held: AtomicBool::new(false),
            muted: AtomicBool::new(self.muted),
            held_audio: Mutex::new(Vec::new()),
            cues: Mutex::new(VecDeque::new()),
            bitmaps: Mutex::new(VecDeque::new()),
            audio_count: AtomicU32::new(0),
            subtitle_count: AtomicU32::new(u32::from(has_external)),
            audio_ordinal: AtomicI32::new(self.audio_ordinal),
            subtitle_ordinal: AtomicI32::new(self.subtitle_ordinal),
            track_gen: AtomicU32::new(0),
            watch_us: AtomicI64::new(0),
            external_cues: Mutex::new(external),
            external_sub: AtomicBool::new(has_external),
            device_name: Mutex::new(String::new()),
            media_title: Mutex::new(String::new()),
            chapters: Mutex::new(Vec::new()),
            audio_delay_us: AtomicI64::new(0),
            audio_skew_frames: AtomicI64::new(0),
            color_trc: AtomicU32::new(0),
            eq_gains_milli: std::array::from_fn(|i| AtomicI32::new(self.eq_gains_milli[i])),
            tone: Mutex::new(Vec::new()),
            tone_bands: Mutex::new(Vec::new()),
            bass_milli: AtomicI32::new(self.bass_milli),
            mid_milli: AtomicI32::new(self.mid_milli),
            treble_milli: AtomicI32::new(self.treble_milli),
            audio_reset: AtomicBool::new(false),
            audio_channel: AtomicU32::new(match self.audio_channel {
                AudioChannelMode::Stereo => 0,
                AudioChannelMode::Left => 1,
                AudioChannelMode::Right => 2,
                AudioChannelMode::Mono => 3,
                AudioChannelMode::Reverse => 4,
                AudioChannelMode::Karaoke => 5,
            }),
            balance_milli: AtomicI32::new(self.balance_milli),
            width_milli: AtomicI32::new(self.width_milli),
            compressor_on: AtomicBool::new(self.compressor_on),
            crossfeed_milli: AtomicI32::new(self.crossfeed_milli),
            eq_bypass: AtomicBool::new(self.eq_bypass),
            eq_preamp_milli: AtomicI32::new(self.eq_preamp_milli),
            spatializer_milli: AtomicI32::new(self.spatializer_milli),
            replaygain_milli: AtomicI32::new(self.replaygain_milli),
            surround_downmix: AtomicBool::new(self.surround_downmix),
            normalizer_on: AtomicBool::new(self.volume_normalizer),
            normalizer_peak_milli: AtomicU32::new(0),
            vu_peak_milli: AtomicU32::new(0),
        });
        let stream = if self.options.audio {
            match start_audio(Arc::clone(&shared), self.options.audio_device.as_deref()) {
                Ok(started) => Some(started),
                Err(err) => {
                    eprintln!("fvid play: audio disabled ({err})");
                    None
                }
            }
        } else {
            None
        };
        let worker_shared = Arc::clone(&shared);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("fvid-play".into())
            .spawn(move || worker(worker_shared, worker_path))
            .map_err(|err| err.to_string())?;
        self.session = Some(Session {
            path,
            shared,
            worker: Some(worker),
            stream,
            clock: Clock::new(0, self.rate_milli),
            last_clock: 0,
            audio_switched: false,
            media_now: 0,
            presented: 0,
            skipped: 0,
            frame: None,
            dirty: false,
            discard_until: None,
        });
        if let Some(start) = self.options.start_us.take() {
            self.request_seek(start.max(0));
        }
        if self.options.start_paused {
            self.set_paused(true);
            self.notice = Some(format_pause_osd(true).into());
        }
        Ok(())
    }

    fn stop_session(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        session.shared.quit.store(true, Ordering::Release);
        session.shared.video_cv.notify_all();
        session.shared.audio_cv.notify_all();
        drop(session.stream.take());
        if let Some(worker) = session.worker.take() {
            if worker.join().is_err() {
                eprintln!("fvid play: playback thread panicked");
            }
        }
    }

    fn report(&self) -> std::result::Result<PlayStats, String> {
        if let Some(err) = &self.error {
            let presented = self
                .session
                .as_ref()
                .map(|session| session.presented)
                .unwrap_or(0);
            if presented == 0 {
                return Err(err.clone());
            }
        }
        if let Some(session) = &self.session {
            if session.presented == 0 {
                if let Some(err) = lock(&session.shared.error).clone() {
                    return Err(err);
                }
            }
        }
        Ok(self.stats())
    }

    fn stats(&self) -> PlayStats {
        let Some(session) = &self.session else {
            return PlayStats {
                presented_frames: 0,
                skipped_frames: 0,
                width: 0,
                height: 0,
                source_width: 0,
                source_height: 0,
                audio: false,
                sample_rate: 0,
                channels: 0,
            };
        };
        let frame = session.frame.as_ref();
        PlayStats {
            presented_frames: session.presented,
            skipped_frames: session.skipped,
            width: frame.map(|frame| frame.width).unwrap_or(0),
            height: frame.map(|frame| frame.height).unwrap_or(0),
            source_width: session.shared.source_width.load(Ordering::Relaxed),
            source_height: session.shared.source_height.load(Ordering::Relaxed),
            audio: session.shared.use_audio_clock.load(Ordering::Acquire),
            sample_rate: session.shared.sample_rate.load(Ordering::Relaxed),
            channels: session.shared.channels.load(Ordering::Relaxed),
        }
    }

    fn pump(&mut self) -> Option<String> {
        let session = self.session.as_mut()?;
        if let Some(err) = lock(&session.shared.error).clone() {
            return Some(err);
        }
        if session.frame.is_none() {
            if !session.shared.ready.load(Ordering::Acquire) {
                return None;
            }
            let frame = {
                let mut queue = lock(&session.shared.video);
                let frame = queue.pop_front()?;
                drop(queue);
                session.shared.video_cv.notify_all();
                frame
            };
            let width = frame.width as usize;
            let height = frame.height as usize;
            if width == 0 || height == 0 || frame.pixels.len() != width * height {
                return Some("decoded frame size does not match its buffer".into());
            }
            let source_w = session.shared.source_width.load(Ordering::Relaxed);
            let source_h = session.shared.source_height.load(Ordering::Relaxed);
            if source_w != frame.width || source_h != frame.height {
                eprintln!("fvid play: display {width}x{height} (source {source_w}x{source_h})");
            }
            session.clock = Clock::new(frame.pts_us, session.clock.rate_milli);
            session.last_clock = session.clock.now();
            session.media_now = frame.pts_us;
            session.presented = 1;
            session.frame = Some(frame);
            session.dirty = true;
            if session.shared.use_audio_clock.load(Ordering::Acquire) {
                if let Some(stream) = session.stream.as_mut() {
                    if let Err(err) = stream.play() {
                        eprintln!("fvid play: audio disabled ({err})");
                        session
                            .shared
                            .use_audio_clock
                            .store(false, Ordering::Release);
                    }
                }
            } else if let Some(stream) = session.stream.as_mut() {
                let _ = stream.pause();
            }
        }
        let media_now = master_clock(
            &session.shared,
            &mut session.clock,
            &mut session.last_clock,
            &mut session.audio_switched,
        );
        session.media_now = media_now;
        session.shared.watch_us.store(media_now, Ordering::Relaxed);
        let hit_stop = should_stop_playback(media_now, self.stop_us);
        if hit_stop && !session.shared.paused.load(Ordering::Relaxed) {
            drop(session);
            self.set_paused(true);
            self.notice = Some(format_stop_osd().into());
            return None;
        }
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        if sleep_timer_fired(self.sleep_deadline_secs, now_secs)
            && !session.shared.paused.load(Ordering::Relaxed)
        {
            drop(session);
            self.sleep_min = 0;
            self.sleep_deadline_secs = None;
            self.set_paused(true);
            self.notice = Some("Sleep timer".into());
            return None;
        }
        loop {
            let next_pts = lock(&session.shared.video)
                .front()
                .map(|frame| frame.pts_us);
            let Some(next_pts) = next_pts else {
                break;
            };
            if next_pts > media_now.saturating_add(SLACK_US) {
                break;
            }
            let more_due = {
                let queue = lock(&session.shared.video);
                queue.len() >= 2
                    && queue
                        .get(1)
                        .is_some_and(|frame| frame.pts_us <= media_now.saturating_add(SLACK_US))
            };
            let mut queue = lock(&session.shared.video);
            let Some(frame) = queue.pop_front() else {
                break;
            };
            drop(queue);
            session.shared.video_cv.notify_all();
            if let Some(until) = session.discard_until {
                if frame.pts_us.saturating_add(frame.duration_us) < until {
                    session.skipped += 1;
                    continue;
                }
                session.discard_until = None;
            } else if more_due
                && should_drop_late_frame(
                    self.drop_frame,
                    media_now.saturating_sub(frame.pts_us),
                    SLACK_US,
                )
            {
                session.skipped += 1;
                continue;
            }
            session.frame = Some(frame);
            session.dirty = true;
            session.presented += 1;
        }
        None
    }

    fn ended(&self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let Some(frame) = &session.frame else {
            return false;
        };
        let ended = session.shared.finished.load(Ordering::Acquire)
            && lock(&session.shared.video).is_empty()
            && (!session.shared.use_audio_clock.load(Ordering::Acquire)
                || (session.shared.audio_eof.load(Ordering::Acquire)
                    && lock(&session.shared.audio).is_empty()));
        let frame_end = frame.pts_us.saturating_add(frame.duration_us.max(1));
        ended && session.media_now >= frame_end
    }

    fn is_paused(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.shared.paused.load(Ordering::Relaxed))
    }

    fn toggle_pause(&mut self) {
        if self.ended() {
            self.request_seek(0);
            self.set_paused(false);
            self.notice = Some(format_pause_osd(false).into());
            return;
        }
        let Some(session) = &mut self.session else {
            return;
        };
        let paused = !session.shared.paused.load(Ordering::Relaxed);
        session.shared.paused.store(paused, Ordering::Relaxed);
        if paused {
            session.clock.pause();
        } else {
            session.clock.resume();
        }
        self.notice = Some(format_pause_osd(paused).into());
    }

    fn pick_file(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open media");
        if let Some(dir) = self
            .session
            .as_ref()
            .and_then(|session| session.path.parent())
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.replace_playlist(vec![path]);
        }
    }

    fn read_input(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|memory| memory.focused().is_some());
        let command = ctx.input(|input| input.modifiers.command);
        let keys = ctx.input(|input| {
            let pressed = |key| input.key_pressed(key);
            (
                pressed(egui::Key::Escape) || pressed(egui::Key::Q),
                !focused && pressed(egui::Key::Space),
                !focused && pressed(egui::Key::ArrowLeft),
                !focused && pressed(egui::Key::ArrowRight),
                !focused && pressed(egui::Key::ArrowUp) && !input.modifiers.alt,
                !focused && pressed(egui::Key::ArrowDown) && !input.modifiers.alt,
                !focused && (pressed(egui::Key::OpenBracket) || pressed(egui::Key::Minus)),
                !focused
                    && (pressed(egui::Key::CloseBracket)
                        || pressed(egui::Key::Equals)
                        || pressed(egui::Key::Plus)),
                !focused && pressed(egui::Key::M),
                !focused && (pressed(egui::Key::F) || pressed(egui::Key::F11)),
                !focused && pressed(egui::Key::Period),
                !focused && pressed(egui::Key::S),
                !focused && pressed(egui::Key::B),
                !focused && pressed(egui::Key::V),
                !focused && pressed(egui::Key::L),
                !focused && pressed(egui::Key::PageUp),
                !focused && pressed(egui::Key::PageDown),
                !focused && pressed(egui::Key::R),
                !focused && pressed(egui::Key::G),
                !focused && pressed(egui::Key::H),
                !focused && pressed(egui::Key::J),
                !focused && pressed(egui::Key::K),
                !focused && pressed(egui::Key::A),
                !focused && pressed(egui::Key::C),
                !focused && pressed(egui::Key::T) && !input.modifiers.shift,
            )
        });
        if keys.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if keys.1 {
            self.toggle_pause();
        }
        if keys.2 {
            if self.spherical {
                if ctx.input(|input| input.modifiers.shift) {
                    self.nudge_roll(-ROLL_STEP_MILLI);
                } else {
                    self.nudge_yaw(-YAW_STEP_MILLI);
                }
            } else if command {
                self.step_bookmark(-1);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(-PAN_STEP_PX, 0, 800, 450);
                } else {
                    self.request_seek(
                        self.shown_media_us()
                            .saturating_sub(seek_step_us_ex(fine, self.seek_jump)),
                    );
                }
            }
        }
        if keys.3 {
            if self.spherical {
                if ctx.input(|input| input.modifiers.shift) {
                    self.nudge_roll(ROLL_STEP_MILLI);
                } else {
                    self.nudge_yaw(YAW_STEP_MILLI);
                }
            } else if command {
                self.step_bookmark(1);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(PAN_STEP_PX, 0, 800, 450);
                } else {
                    self.request_seek(
                        self.shown_media_us()
                            .saturating_add(seek_step_us_ex(fine, self.seek_jump)),
                    );
                }
            }
        }
        if keys.4 {
            if self.spherical {
                self.nudge_pitch(PITCH_STEP_MILLI);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(0, -PAN_STEP_PX, 800, 450);
                } else {
                    self.nudge_volume(VOLUME_STEP_MILLI);
                }
            }
        }
        if keys.5 {
            if self.spherical {
                self.nudge_pitch(-PITCH_STEP_MILLI);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(0, PAN_STEP_PX, 800, 450);
                } else {
                    self.nudge_volume(-VOLUME_STEP_MILLI);
                }
            }
        }
        if keys.6 {
            if command {
                self.rate_milli = cycle_rate_preset_milli(self.rate_milli);
                self.set_rate(self.rate_milli);
            } else {
                self.nudge_rate(-RATE_STEP_MILLI);
            }
        }
        if keys.7 {
            if command {
                self.rate_milli = cycle_rate_preset_milli(self.rate_milli);
                self.set_rate(self.rate_milli);
            } else {
                self.nudge_rate(RATE_STEP_MILLI);
            }
        }
        if keys.8 {
            self.set_muted(!self.muted);
        }
        if keys.9 {
            self.fullscreen = !self.fullscreen;
            self.fullscreen_dirty = true;
        }
        if keys.10 {
            self.step_pending = true;
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Comma)) {
            self.step_frame_back();
        }
        if !focused {
            if command && ctx.input(|input| input.key_pressed(egui::Key::Num3)) {
                if ctx.input(|input| input.modifiers.shift) {
                    self.cycle_spherical_projection_mode();
                } else {
                    self.toggle_spherical();
                }
            } else {
                let digit = ctx.input(|input| {
                    [
                        egui::Key::Num0,
                        egui::Key::Num1,
                        egui::Key::Num2,
                        egui::Key::Num3,
                        egui::Key::Num4,
                        egui::Key::Num5,
                        egui::Key::Num6,
                        egui::Key::Num7,
                        egui::Key::Num8,
                        egui::Key::Num9,
                    ]
                    .into_iter()
                    .enumerate()
                    .find_map(|(digit, key)| input.key_pressed(key).then_some(digit as u8))
                });
                if let Some(digit) = digit {
                    if let Some(target) = position_us_from_digit(digit, self.duration_us()) {
                        self.request_seek(target);
                        self.notice = Some(format!(
                            "Position {}%",
                            if digit == 0 { 100 } else { digit * 10 }
                        ));
                    }
                }
            }
        }
        if keys.11 {
            if ctx.input(|input| input.modifiers.shift) {
                if command {
                    self.save_playlist();
                } else {
                    self.snapshot_format = cycle_snapshot_format(self.snapshot_format);
                    self.notice = Some(format!(
                        "Snapshot {}",
                        snapshot_format_ext(self.snapshot_format).to_ascii_uppercase()
                    ));
                }
            } else {
                self.save_snapshot();
            }
        }
        if keys.12 {
            if command {
                if ctx.input(|input| input.modifiers.alt) {
                    if ctx.input(|input| input.modifiers.shift) {
                        self.load_bookmarks();
                    } else {
                        self.save_bookmarks();
                    }
                } else if ctx.input(|input| input.modifiers.shift) {
                    self.clear_all_bookmarks();
                } else {
                    self.add_bookmark();
                }
            } else {
                self.cycle_audio(1);
            }
        }
        if keys.13 {
            self.cycle_subtitle(1);
        }
        if keys.14 {
            self.mark_ab();
        }
        if keys.15 {
            if self.spherical {
                self.nudge_fov(-FOV_STEP_MILLI);
            } else {
                self.step_chapter(-1);
            }
        }
        if keys.16 {
            if self.spherical {
                self.nudge_fov(FOV_STEP_MILLI);
            } else {
                self.step_chapter(1);
            }
        }
        if keys.17 {
            if command && !ctx.input(|input| input.modifiers.shift) {
                self.toggle_shuffle();
            } else if !command {
                self.cycle_repeat_mode();
            }
        }
        if keys.18 {
            if ctx.input(|input| input.modifiers.alt) {
                self.nudge_secondary_subtitle_delay(-1);
            } else {
                self.nudge_subtitle_delay(-1);
            }
        }
        if keys.19 {
            if command {
                if !ctx.input(|input| input.modifiers.shift) {
                    self.cycle_hdr_mode();
                }
            } else if ctx.input(|input| input.modifiers.alt) {
                self.nudge_secondary_subtitle_delay(1);
            } else {
                self.nudge_subtitle_delay(1);
            }
        }
        if keys.20 {
            self.nudge_audio_delay(-1);
        }
        if keys.21 {
            self.nudge_audio_delay(1);
        }
        if keys.22 {
            self.cycle_aspect_mode();
        }
        if keys.23 {
            self.cycle_crop_mode();
        }
        if keys.24 {
            self.toggle_on_top();
        }
        if !focused
            && !command
            && ctx.input(|input| input.key_pressed(egui::Key::T) && input.modifiers.shift)
        {
            self.cycle_position_osd();
        }
        if !focused && command && ctx.input(|input| input.key_pressed(egui::Key::N)) {
            self.toggle_volume_normalizer();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::N)) {
            if !command {
                self.step_playlist(1);
            }
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::P)) {
            self.step_playlist(-1);
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::Slash) && !input.modifiers.shift)
        {
            self.reset_av_sync();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Z)) {
            if command {
                self.zoom_milli = cycle_integer_zoom(self.zoom_milli);
                let (_, pan_x, pan_y) = reset_zoom_pan();
                self.pan_x_px = pan_x;
                self.pan_y_px = pan_y;
                self.notice = Some(format_integer_zoom_osd(self.zoom_milli));
            } else {
                let zoom_in = !ctx.input(|input| input.modifiers.shift);
                self.nudge_zoom(zoom_in);
            }
        }
        if !focused
            && ctx.input(|input| {
                input.modifiers.alt
                    && (input.key_pressed(egui::Key::Equals) || input.key_pressed(egui::Key::Plus))
            })
        {
            self.nudge_subtitle_scale(SUBTITLE_SCALE_STEP_MILLI);
        }
        if !focused && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Minus))
        {
            self.nudge_subtitle_scale(-SUBTITLE_SCALE_STEP_MILLI);
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.key_pressed(egui::Key::Equals) || input.key_pressed(egui::Key::Plus)
            })
        {
            self.nudge_subtitle_opacity(SUBTITLE_OPACITY_STEP_MILLI);
        }
        if !focused && command && ctx.input(|input| input.key_pressed(egui::Key::Minus)) {
            self.nudge_subtitle_opacity(-SUBTITLE_OPACITY_STEP_MILLI);
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::W)) {
            if command {
                self.fit_window_video();
            } else {
                let wider = !ctx.input(|input| input.modifiers.shift);
                self.nudge_stereo_width(if wider {
                    WIDTH_STEP_MILLI
                } else {
                    -WIDTH_STEP_MILLI
                });
            }
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::D)) {
            if command {
                self.cycle_stereo3d_mode();
            } else {
                self.deinterlace = cycle_deinterlace(self.deinterlace);
                self.adjust_dirty = true;
                self.notice = Some(format_deinterlace_osd(self.deinterlace));
            }
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::I)) {
            if ctx.input(|input| input.modifiers.shift) {
                self.show_media_info();
            } else {
                self.show_stats = !self.show_stats;
            }
        }
        if !focused
            && ctx.input(|input| {
                input.key_pressed(egui::Key::F1)
                    || (input.key_pressed(egui::Key::Slash) && input.modifiers.shift)
            })
        {
            self.notice = Some(format_hotkeys_help_osd().into());
        }
        if !focused && command && ctx.input(|input| input.key_pressed(egui::Key::O)) {
            if ctx.input(|input| input.modifiers.shift) {
                self.show_osd = cycle_show_osd(self.show_osd);
                self.notice = Some(format_show_osd(self.show_osd).into());
            }
        }
        if !focused && command && ctx.input(|input| input.key_pressed(egui::Key::L)) {
            self.drop_frame = cycle_drop_frame(self.drop_frame);
            self.notice = Some(format_drop_frame_osd(self.drop_frame));
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::T))
        {
            let title = self
                .session
                .as_ref()
                .map(|session| {
                    let stored = lock(&session.shared.media_title);
                    if stored.is_empty() {
                        media_display_title(&session.path, None)
                    } else {
                        stored.clone()
                    }
                })
                .unwrap_or_else(|| "Title".into());
            self.notice = Some(format_title_osd(&title));
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::M))
        {
            self.marquee_position = cycle_marquee_position(self.marquee_position);
            self.notice = Some(format_marquee_osd(
                &self.marquee_text,
                self.marquee_position,
            ));
        }
        if !focused && command && ctx.input(|input| input.key_pressed(egui::Key::E)) {
            if ctx.input(|input| input.modifiers.alt) {
                self.nudge_eq_preamp(EQ_PREAMP_STEP_MILLI);
            } else if ctx.input(|input| input.modifiers.shift) {
                self.nudge_eq_preamp(-EQ_PREAMP_STEP_MILLI);
            } else {
                self.display_effect = cycle_display_effect(self.display_effect);
                self.adjust_dirty = true;
                self.notice = Some(format_display_effect_osd(self.display_effect));
            }
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::X))
        {
            let stronger = !ctx.input(|input| input.modifiers.alt);
            self.nudge_spatializer(if stronger {
                SPATIALIZER_STEP_MILLI
            } else {
                -SPATIALIZER_STEP_MILLI
            });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::G))
        {
            self.toggle_gapless();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::C))
        {
            let longer = !ctx.input(|input| input.modifiers.alt);
            self.nudge_crossfade(if longer {
                CROSSFADE_STEP_MS as i32
            } else {
                -(CROSSFADE_STEP_MS as i32)
            });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::R))
        {
            self.cycle_replaygain_boost();
        }
        if !focused
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::C))
            && !command
        {
            self.cycle_subtitle_color_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::P))
        {
            self.cycle_playlist_sort_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::L))
        {
            self.show_loudness_osd();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::R))
            && !ctx.input(|input| input.modifiers.alt)
        {
            self.show_recent_osd();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::F))
        {
            self.cycle_post_fx_mode();
        }
        if !focused
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::J))
            && !command
        {
            self.cycle_seek_jump_size();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::D))
        {
            self.toggle_surround_downmix();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::T))
        {
            self.toggle_scaletempo();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::M))
            && !ctx.input(|input| input.modifiers.alt)
        {
            self.toggle_minimal_interface();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::P))
        {
            let up = !ctx.input(|input| input.modifiers.shift);
            self.nudge_audio_pitch(if up {
                AUDIO_PITCH_STEP_MILLI
            } else {
                -AUDIO_PITCH_STEP_MILLI
            });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::V))
        {
            self.cycle_visualization_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::I))
        {
            let longer = !ctx.input(|input| input.modifiers.shift);
            self.nudge_image_duration(if longer { 5 } else { -5 });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::K))
        {
            self.cycle_closed_captions();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::W))
        {
            self.toggle_wallpaper_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::U))
        {
            self.cycle_subtitle_encoding_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::OpenBracket))
        {
            self.nudge_crop_pixels(8, 0, 0, 0);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::CloseBracket))
        {
            self.nudge_crop_pixels(0, 0, 8, 0);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::Y))
        {
            self.toggle_teletext();
        }
        if !focused
            && self.teletext_enabled
            && !command
            && ctx.input(|input| input.key_pressed(egui::Key::PageUp) && input.modifiers.alt)
        {
            self.nudge_teletext_page(1);
        }
        if !focused
            && self.teletext_enabled
            && !command
            && ctx.input(|input| input.key_pressed(egui::Key::PageDown) && input.modifiers.alt)
        {
            self.nudge_teletext_page(-1);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::A))
        {
            self.toggle_aspect_lock();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::N))
        {
            self.toggle_snapshot_sequential();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::H))
        {
            self.toggle_hw_decode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::U))
        {
            self.cycle_logo_pos();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::O))
            && !ctx.input(|input| input.modifiers.shift)
        {
            self.cycle_mosaic_grid();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Q))
        {
            let up = !ctx.input(|input| input.modifiers.shift);
            self.nudge_param_eq(if up { 100 } else { -100 });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Equals))
        {
            self.nudge_amplifier(100);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Minus))
        {
            self.nudge_amplifier(-100);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::R))
            && ctx.input(|input| input.modifiers.alt)
        {
            self.toggle_recording();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::X))
        {
            self.cycle_proxy();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::A)
            })
        {
            self.toggle_http_auth_flags();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::I))
            && !ctx.input(|input| input.modifiers.alt)
        {
            // Shift+I is media-info elsewhere; Ctrl+Shift+I shows HDR metadata.
            self.show_hdr_metadata_osd();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::V))
        {
            self.cycle_video_track_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::Period))
            && !ctx.input(|input| input.modifiers.alt)
        {
            self.toggle_silence_skip();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::S))
        {
            self.try_skip_marker();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::F))
            && ctx.input(|input| input.modifiers.alt)
        {
            self.toggle_current_favorite();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Slash))
        {
            self.set_playlist_filter(String::new());
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::H))
            && ctx.input(|input| input.modifiers.alt)
        {
            self.toggle_remote_control();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Period))
        {
            self.toggle_bitperfect();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::N))
        {
            self.toggle_night_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Q))
            && ctx.input(|input| input.modifiers.shift)
        {
            self.queue_current(true);
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::F))
            && !ctx.input(|input| input.modifiers.shift)
        {
            self.toggle_forced_subs_only();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::Period))
            && ctx.input(|input| input.modifiers.alt)
        {
            self.show_frame_rate_osd();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num0))
        {
            let wider = !ctx.input(|input| input.modifiers.shift);
            self.nudge_ipd(if wider {
                IPD_STEP_MILLI
            } else {
                -IPD_STEP_MILLI
            });
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::Num4))
        {
            self.cycle_vr_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num8))
        {
            self.cycle_ambisonic_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::Num5))
        {
            self.cycle_cast_mode();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num9))
        {
            self.show_active_lyric();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num1))
        {
            if ctx.input(|input| input.modifiers.shift) {
                self.load_ab_slot(0);
            } else {
                self.store_ab_slot(0);
            }
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num2))
        {
            self.toggle_pip();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num3))
        {
            self.toggle_horizon_lock();
            self.apply_horizon_lock_pitch();
            self.adjust_dirty = true;
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num6))
        {
            self.show_hdr_peak_suggestion();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num4))
        {
            self.cycle_spherical_stereo_layout();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num5))
        {
            if ctx.input(|input| input.modifiers.shift) {
                self.show_compass();
            } else {
                self.recenter_view();
            }
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Num7))
        {
            if ctx.input(|input| input.modifiers.shift) {
                self.cycle_hdr_highlight_desat();
            } else {
                self.cycle_tonemap_strength();
            }
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::B)
            })
        {
            self.cycle_barrel_distortion();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::T)
            })
        {
            self.cycle_color_temp();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::D)
            })
        {
            self.toggle_audio_duck();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::V)
            })
        {
            self.toggle_waveform();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::L)
            })
        {
            self.detect_and_show_letterbox();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num0)
            })
        {
            self.cycle_fov_preset();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num8)
            })
        {
            self.toggle_hdr10_plus();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num9)
            })
        {
            self.cycle_hlg_ootf();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::H)
            })
        {
            self.show_hdr_headroom();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::C)
            })
        {
            self.show_timecode();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::N)
            })
        {
            self.cycle_denoise();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::E)
            })
        {
            self.cycle_dialogue_enhance();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::S)
            })
        {
            self.toggle_vectorscope();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::G)
            })
        {
            self.toggle_gyro_look();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::U)
            })
        {
            self.cycle_vr_vignette();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::K)
            })
        {
            self.cycle_hdr_black_lift();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::P)
            })
        {
            self.cycle_unsharp();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::M)
            })
        {
            self.toggle_clipboard_snapshot();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num6)
            })
        {
            self.show_eac_face();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::W)
            })
        {
            self.toggle_watch_party();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Y)
            })
        {
            self.toggle_auto_horizon();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::I)
            })
        {
            self.show_live_edge();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::R)
            })
        {
            self.do_instant_replay();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num7)
            })
        {
            self.show_hdr_sdr_ratio();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num1)
            })
        {
            self.cycle_ambisonic_channel_order();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num2)
            })
        {
            self.cycle_chromatic_aberration();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num3)
            })
        {
            self.cycle_soft_limiter();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num4)
            })
        {
            self.cycle_echo_feedback();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Num5)
            })
        {
            self.toggle_anaglyph_dubois();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::F)
            })
        {
            self.toggle_bt2446();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::O)
            })
        {
            self.cycle_chorus();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::Z)
            })
        {
            self.cycle_reverb();
        }
        if !focused
            && command
            && ctx.input(|input| {
                input.modifiers.alt && input.modifiers.shift && input.key_pressed(egui::Key::X)
            })
        {
            self.cycle_atempo();
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::H))
        {
            self.hdr_nits = match self.hdr_nits {
                0..=99 => 100,
                100..=199 => 400,
                200..=999 => 1_000,
                _ => 100,
            };
            self.adjust_dirty = true;
            self.notice = Some(format_hdr_nits_osd(self.hdr_nits));
        }
        if !focused
            && command
            && ctx.input(|input| input.modifiers.shift && input.key_pressed(egui::Key::J))
        {
            self.jump_random();
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowUp) && input.modifiers.alt)
        {
            if command {
                self.cycle_subtitle_pos();
            } else {
                self.subtitle_margin_px = clamp_subtitle_margin(self.subtitle_margin_px + 10);
            }
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowDown) && input.modifiers.alt)
        {
            if command {
                self.cycle_subtitle_pos();
            } else {
                self.subtitle_margin_px = clamp_subtitle_margin(self.subtitle_margin_px - 10);
            }
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft) && input.modifiers.alt)
        {
            self.nudge_balance(-BALANCE_STEP_MILLI);
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowRight) && input.modifiers.alt)
        {
            self.nudge_balance(BALANCE_STEP_MILLI);
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::O)) {
            let stronger = !ctx.input(|input| input.modifiers.shift);
            self.nudge_crossfeed(if stronger {
                CROSSFEED_STEP_MILLI
            } else {
                -CROSSFEED_STEP_MILLI
            });
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::U)) {
            self.toggle_compressor();
        }
        if !focused && !command && ctx.input(|input| input.key_pressed(egui::Key::X)) {
            self.cycle_sleep_timer();
        }
        if !focused && !command && ctx.input(|input| input.key_pressed(egui::Key::E)) {
            self.toggle_eq_bypass();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Y)) {
            self.cycle_audio_channel_mode();
        }
        if !focused {
            let (scroll, ctrl) =
                ctx.input(|input| (input.smooth_scroll_delta.y, input.modifiers.command));
            if scroll.abs() > 0.1 {
                let lines = if scroll > 0.0 { 1 } else { -1 };
                if ctrl {
                    let next = rate_from_wheel(self.rate_milli, lines);
                    if next != self.rate_milli {
                        self.set_rate(next);
                    }
                } else {
                    let next = volume_from_wheel(self.volume_milli, lines);
                    if next != self.volume_milli {
                        self.nudge_volume(next as i32 - self.volume_milli as i32);
                    }
                }
            }
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Backspace)) {
            self.stop_playback();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Home)) {
            let (zoom, pan_x, pan_y) = reset_zoom_pan();
            self.zoom_milli = zoom;
            self.pan_x_px = pan_x;
            self.pan_y_px = pan_y;
            self.notice = Some(format_zoom_osd(zoom));
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::End)) {
            let target = seek_end_us(self.duration_us(), self.stop_us);
            self.request_seek(target);
            self.notice = Some(format_jump_osd(target));
        }
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let files: Vec<PathBuf> = dropped
            .into_iter()
            .filter_map(|file| {
                let path = file.path().to_path_buf();
                (path.to_str().is_some() && path.is_file()).then_some(path)
            })
            .collect();
        if files.is_empty() {
            self.notice = Some("Dropped item is not a local file".into());
        } else {
            self.replace_playlist(files);
        }
    }

    fn upload(&mut self, ctx: &egui::Context) {
        let Some(image) = self.take_image() else {
            return;
        };
        if !self.fitted {
            self.fitted = true;
            let width = (image.size[0] as f32).clamp(640.0, 1280.0);
            let height = (image.size[1] as f32 + 168.0).clamp(460.0, 920.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
        }
        let options = egui::TextureOptions::LINEAR;
        if let Some(texture) = &mut self.texture {
            texture.set(image, options);
        } else {
            self.texture = Some(ctx.load_texture("video", image, options));
        }
    }

    fn take_image(&mut self) -> Option<egui::ColorImage> {
        self.maybe_auto_hdr();
        self.maybe_auto_equirect();
        let now = self.shown_media_us();
        let session = self.session.as_mut()?;
        if !session.dirty && !self.adjust_dirty {
            return None;
        }
        let frame = session.frame.as_ref()?;
        let bitmap = {
            let mut planes = lock(&session.shared.bitmaps);
            active_bitmap_subtitle(planes.make_contiguous(), now).cloned()
        };
        let image = color_image(
            frame,
            self.brightness_milli,
            self.contrast_milli,
            self.saturation_milli,
            self.hue_milli,
            self.gamma_milli,
            self.flip_h,
            self.flip_v,
            self.rotate,
            self.deinterlace,
            self.stereo3d,
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
            self.hdr_tonemap,
            frame.color_trc,
            self.display_effect,
            self.hdr_nits,
            self.post_fx,
            self.spherical_projection,
            self.hdr_maxcll,
            self.hdr_maxfall,
            self.color_primaries,
            self.tonemap_strength_milli,
            self.hdr_highlight_desat_milli,
            self.hdr_black_lift_milli,
            self.color_temp_kelvin,
            self.hlg_ootf_gamma_milli,
            self.bt2446_tonemap,
            self.gamut_map_bt709,
            bitmap.as_ref(),
        );
        session.dirty = false;
        self.adjust_dirty = false;
        Some(image)
    }

    fn sync_title(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else {
            return;
        };
        let name = {
            let stored = lock(&session.shared.media_title);
            if stored.is_empty() {
                media_display_title(&session.path, None)
            } else {
                stored.clone()
            }
        };
        let title = window_title(
            &name,
            &session.shared,
            session.media_now,
            self.position_display,
        );
        if title != self.title {
            self.title.clone_from(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn wants_frames(&self) -> bool {
        if self.error.is_some() {
            return false;
        }
        let Some(session) = &self.session else {
            return false;
        };
        if session.frame.is_none() || session.discard_until.is_some() {
            return true;
        }
        !session.shared.paused.load(Ordering::Relaxed) && !self.ended()
    }

    fn progress(&self) -> f32 {
        let Some(session) = &self.session else {
            return 0.0;
        };
        let duration = session.shared.duration_us.load(Ordering::Relaxed);
        if duration <= 0 {
            return 0.0;
        }
        (session.media_now as f32 / duration as f32).clamp(0.0, 1.0)
    }

    fn clock_label(&self) -> String {
        if self.session.is_none() {
            return "00:00".into();
        }
        let now = format_clock(self.shown_media_us());
        let duration = self.duration_us();
        if duration >= 0 {
            format!("{now} / {}", format_clock(duration))
        } else {
            now
        }
    }

    fn duration_us(&self) -> i64 {
        self.session
            .as_ref()
            .map(|session| session.shared.duration_us.load(Ordering::Relaxed))
            .unwrap_or(-1)
    }

    fn shown_media_us(&self) -> i64 {
        if let Some(frac) = self.scrub {
            let duration = self.duration_us();
            if duration > 0 {
                return (f64::from(frac) * duration as f64).round() as i64;
            }
        }
        self.session
            .as_ref()
            .map(|session| session.media_now)
            .unwrap_or(0)
    }

    fn seek_bar(&mut self, ui: &mut egui::Ui) {
        let duration = self.duration_us();
        if duration <= 0 {
            ui.add(egui::ProgressBar::new(0.0).desired_width(ui.available_width()));
            return;
        }
        
        // Обновляем buffered_us из размера видео-очереди
        if let Some(session) = &self.session {
            let queued_frames = lock(&session.shared.video).len() as i64;
            self.buffered_us = queued_frames * duration / (duration.max(1) / 30);
        }
        
        let played_frac = self.scrub.unwrap_or(self.progress());
        let buffered_frac = (self.buffered_us.max(0) as f32 / duration as f32).clamp(0.0, 1.0);
        
        // Кастомная отрисовка с сегментами
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect.shrink(2.0), 4.0, egui::Color32::from_rgb(40, 40, 40));
        
        // Buffer segment (синий)
        if buffered_frac > 0.0 && buffered_frac < 1.0 {
            let buf_w = (buffered_frac * (rect.width() - 4.0)).max(1.0);
            let buf_rect = egui::Rect::from_min_x_max(rect.min.x + 2.0, rect.min.x + 2.0 + buf_w)
                .expand(-2.0);
            ui.painter().rect_filled(buf_rect, 4.0, egui::Color32::from_rgb(64, 128, 255));
        }
        
        // Played segment (зеленый поверх синего если больше буфера)
        if played_frac > 0.0 && played_frac < 1.0 && played_frac > buffered_frac {
            let start_x = rect.min.x + 2.0 + buffered_frac * (rect.width() - 4.0);
            let played_w = ((played_frac - buffered_frac) * (rect.width() - 4.0)).max(0.0);
            let played_rect = egui::Rect::from_min_x_max(start_x, start_x + played_w)
                .expand(-2.0);
            ui.painter().rect_filled(played_rect, 4.0, egui::Color32::from_rgb(80, 200, 120));
        }
        
        // Slider для seek (прозрачный поверх всего)
        let mut frac = self.scrub.unwrap_or(played_frac);
        let response = ui.add(egui::Slider::new(&mut frac, 0.0..=1.0).show_value(false));
        if response.dragged() {
            self.scrub = Some(frac);
        }
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            self.scrub = None;
            let target = (f64::from(frac) * duration as f64).round() as i64;
            self.request_seek(target);
        }
    }

    fn volume_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mute = if self.muted { "Unmute" } else { "Mute" };
            if ui.add(egui::Button::new(mute)).clicked() {
                self.set_muted(!self.muted);
            }
            ui.label("Vol");
            let mut volume = self.volume_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut volume, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.volume_milli = clamp_volume_milli((volume * 1000.0).round() as i32);
                if let Some(session) = &self.session {
                    session
                        .shared
                        .volume_milli
                        .store(self.volume_milli, Ordering::Relaxed);
                }
            }
            ui.label("Speed");
            let mut speed = self.rate_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut speed, 0.25..=4.0).show_value(false))
                .changed()
            {
                self.set_rate(clamp_rate_milli(speed));
            }
            ui.label(format_rate(self.rate_milli));
            let peak = self
                .session
                .as_ref()
                .map(|session| session.shared.vu_peak_milli.load(Ordering::Relaxed))
                .unwrap_or(0);
            let fills = vu_bar_fills(peak, 8);
            ui.label("VU");
            for fill in fills {
                let tall = 4.0 + (fill as f32 / 100.0) * 16.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 20.0), egui::Sense::hover());
                let bar = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, rect.max.y - tall),
                    egui::vec2(4.0, tall),
                );
                let color = if fill > 85 {
                    egui::Color32::from_rgb(255, 80, 80)
                } else if fill > 60 {
                    egui::Color32::from_rgb(255, 200, 80)
                } else {
                    egui::Color32::from_rgb(80, 200, 120)
                };
                ui.painter().rect_filled(bar, 0.0, color);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Brt");
            let mut brightness = self.brightness_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut brightness, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.brightness_milli = clamp_adjust_milli((brightness * 1000.0).round() as i32);
                self.adjust_dirty = true;
            }
            ui.label("Con");
            let mut contrast = self.contrast_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut contrast, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.contrast_milli = clamp_adjust_milli((contrast * 1000.0).round() as i32);
                self.adjust_dirty = true;
            }
            ui.label("Sat");
            let mut saturation = self.saturation_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut saturation, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.saturation_milli = clamp_adjust_milli((saturation * 1000.0).round() as i32);
                self.adjust_dirty = true;
            }
            ui.label("Hue");
            let mut hue = self.hue_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut hue, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.hue_milli = clamp_adjust_milli((hue * 1000.0).round() as i32);
                self.adjust_dirty = true;
            }
            ui.label("Gam");
            let mut gamma = self.gamma_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut gamma, 0.25..=4.0).show_value(false))
                .changed()
            {
                self.gamma_milli = clamp_adjust_milli((gamma * 1000.0).round() as i32);
                self.adjust_dirty = true;
            }
            if ui.button("Reset").clicked() {
                let (b, c, s, h, g) = reset_video_adjust();
                self.brightness_milli = b;
                self.contrast_milli = c;
                self.saturation_milli = s;
                self.hue_milli = h;
                self.gamma_milli = g;
                self.adjust_dirty = true;
                self.notice = Some(format_adjust_osd(b, c, s, h, g));
            }
            let flip_h = if self.flip_h { "H*" } else { "H" };
            if ui.button(flip_h).clicked() {
                self.flip_h = !self.flip_h;
                self.adjust_dirty = true;
                self.notice = Some(format_flip_osd(self.flip_h, self.flip_v));
            }
            let flip_v = if self.flip_v { "V*" } else { "V" };
            if ui.button(flip_v).clicked() {
                self.flip_v = !self.flip_v;
                self.adjust_dirty = true;
                self.notice = Some(format_flip_osd(self.flip_h, self.flip_v));
            }
            if ui.button(rotate_label(self.rotate)).clicked() {
                self.rotate = cycle_rotate(self.rotate);
                self.adjust_dirty = true;
                self.notice = Some(format_rotate_osd(self.rotate));
            }
            let deint = deinterlace_label(self.deinterlace);
            if ui.button(deint).clicked() {
                self.deinterlace = cycle_deinterlace(self.deinterlace);
                self.adjust_dirty = true;
            }
            let sph = if self.spherical { "360*" } else { "360" };
            if ui.button(sph).clicked() {
                self.toggle_spherical();
            }
            if ui.button(format_play_stereo3d_osd(self.stereo3d)).clicked() {
                self.cycle_stereo3d_mode();
            }
            let hdr = format!("HDR {}", hdr_tonemap_label(self.hdr_tonemap));
            if ui.button(hdr).clicked() {
                self.cycle_hdr_mode();
            }
        });
        ui.horizontal(|ui| {
            ui.label("EQ");
            for i in 0..EQ_BAND_COUNT {
                ui.vertical(|ui| {
                    ui.label(format!("{}", EQ_BAND_HZ[i]));
                    let mut gain = self.eq_gains_milli[i] as f32 / 1000.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut gain, 0.05..=4.0)
                                .vertical()
                                .show_value(false),
                        )
                        .changed()
                    {
                        self.eq_gains_milli[i] =
                            eq_band_step_milli(0, (gain * 1000.0).round() as i32);
                        if let Some(session) = &self.session {
                            session.shared.eq_gains_milli[i]
                                .store(self.eq_gains_milli[i], Ordering::Relaxed);
                        }
                    }
                });
            }
            if ui.button(eq_preset_label(self.eq_preset)).clicked() {
                self.eq_preset = cycle_eq_preset(self.eq_preset);
                self.eq_gains_milli = set_eq_gains_from_preset(self.eq_preset);
                if let Some(session) = &self.session {
                    for (slot, gain) in session
                        .shared
                        .eq_gains_milli
                        .iter()
                        .zip(self.eq_gains_milli.iter())
                    {
                        slot.store(*gain, Ordering::Relaxed);
                    }
                }
                self.notice = Some(format_eq_preset_osd(self.eq_preset));
            }
            let eq_label = if self.eq_bypass { "EQ*" } else { "EQ" };
            if ui.button(eq_label).clicked() {
                self.toggle_eq_bypass();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Bass");
            let mut bass = self.bass_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut bass, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.set_tone_gains(
                    clamp_adjust_milli((bass * 1000.0).round() as i32),
                    self.mid_milli,
                    self.treble_milli,
                );
            }
            ui.label("Mid");
            let mut mid = self.mid_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut mid, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.set_tone_gains(
                    self.bass_milli,
                    clamp_adjust_milli((mid * 1000.0).round() as i32),
                    self.treble_milli,
                );
            }
            ui.label("Treble");
            let mut treble = self.treble_milli as f32 / 1000.0;
            if ui
                .add(egui::Slider::new(&mut treble, 0.0..=2.0).show_value(false))
                .changed()
            {
                self.set_tone_gains(
                    self.bass_milli,
                    self.mid_milli,
                    clamp_adjust_milli((treble * 1000.0).round() as i32),
                );
            }
            if ui.button("Tone").clicked() {
                let (b, m, t) = reset_tone_gains();
                self.set_tone_gains(b, m, t);
            }
        });
    }

    fn track_row(&mut self, ui: &mut egui::Ui) {
        let (audio_count, subtitle_count) = self
            .session
            .as_ref()
            .map(|session| {
                (
                    session.shared.audio_count.load(Ordering::Relaxed),
                    session.shared.subtitle_count.load(Ordering::Relaxed),
                )
            })
            .unwrap_or((0, 0));
        ui.horizontal(|ui| {
            let audio_label = if audio_count == 0 {
                "Audio —".to_string()
            } else {
                format!(
                    "Audio {}/{audio_count}",
                    self.audio_ordinal.saturating_add(1)
                )
            };
            if ui
                .add_enabled(audio_count > 1, egui::Button::new(audio_label))
                .clicked()
            {
                self.cycle_audio(1);
            }
            let subtitle_label = if subtitle_count == 0 || self.subtitle_ordinal < 0 {
                "Subtitles off".to_string()
            } else {
                format!(
                    "Subtitles {}/{subtitle_count}",
                    self.subtitle_ordinal.saturating_add(1)
                )
            };
            if ui
                .add_enabled(subtitle_count > 0, egui::Button::new(subtitle_label))
                .clicked()
            {
                self.cycle_subtitle(1);
            }
            if ui.add(egui::Button::new("Sub file")).clicked() {
                self.pick_subtitles();
            }
            let device = self
                .session
                .as_ref()
                .map(|session| lock(&session.shared.device_name).clone())
                .filter(|name| !name.is_empty())
                .or_else(|| self.options.audio_device.clone())
                .unwrap_or_else(|| "default".into());
            if ui.add(egui::Button::new(format!("Out {device}"))).clicked() {
                let delta = if ui.input(|input| input.modifiers.shift) {
                    -1
                } else {
                    1
                };
                self.cycle_output_device(delta);
            }
            if ui.button(audio_channel_label(self.audio_channel)).clicked() {
                self.cycle_audio_channel_mode();
            }
            if ui.button("Bal-").clicked() {
                self.nudge_balance(-BALANCE_STEP_MILLI);
            }
            if ui.button("Bal+").clicked() {
                self.nudge_balance(BALANCE_STEP_MILLI);
            }
        });
    }

    fn cycle_output_device(&mut self, delta: i32) {
        let names = audio_output_devices();
        let current = self
            .session
            .as_ref()
            .map(|session| lock(&session.shared.device_name).clone())
            .filter(|name| !name.is_empty())
            .or_else(|| self.options.audio_device.clone())
            .unwrap_or_default();
        let Some(next) = cycle_output_device(&names, &current, delta) else {
            self.notice = Some("No audio output devices".into());
            return;
        };
        self.options.audio_device = Some(next.to_string());
        let Some(session) = &mut self.session else {
            self.notice = Some(format!("Out {next}"));
            return;
        };
        drop(session.stream.take());
        session.shared.audio_reset.store(true, Ordering::Release);
        lock(&session.shared.audio).clear();
        lock(&session.shared.held_audio).clear();
        session.shared.audio_cv.notify_all();
        match start_audio(Arc::clone(&session.shared), Some(next)) {
            Ok(stream) => {
                if !session.shared.paused.load(Ordering::Relaxed) {
                    if let Err(err) = stream.play() {
                        self.notice = Some(format!("Out {next} (paused: {err})"));
                        session.stream = Some(stream);
                        return;
                    }
                }
                session.stream = Some(stream);
                eprintln!("fvid play: switched audio device: {next}");
                self.notice = Some(format!("Out {next}"));
            }
            Err(err) => {
                self.notice = Some(err);
            }
        }
    }

    fn pick_subtitles(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open subtitles")
            .add_filter("Subtitles", &["srt", "ass", "ssa"]);
        if let Some(dir) = self
            .session
            .as_ref()
            .and_then(|session| session.path.parent())
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.load_external_subtitles(path);
        }
    }

    fn mark_ab(&mut self) {
        let now = self.shown_media_us().max(0);
        self.ab = ab_mark(self.ab, now);
        let notice = format_ab_osd(self.ab);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn step_chapter(&mut self, delta: i32) {
        let starts = self
            .session
            .as_ref()
            .map(|session| lock(&session.shared.chapters).clone())
            .unwrap_or_default();
        let Some(target) = chapter_step(&starts, self.shown_media_us(), delta) else {
            return;
        };
        let index = chapter_index(&starts, target).unwrap_or(0);
        let notice = format_chapter_osd(index, starts.len(), target);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
        self.request_seek(target);
    }

    fn cycle_repeat_mode(&mut self) {
        self.repeat = cycle_repeat(self.repeat);
        let label = format_repeat_osd(self.repeat);
        eprintln!("fvid play: {label}");
        self.notice = Some(label.to_string());
    }

    fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.rebuild_order();
        let notice = format_shuffle_osd(self.shuffle);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice.into());
    }

    fn rebuild_order(&mut self) {
        if !self.shuffle || self.playlist.len() < 2 {
            self.order.clear();
            self.order_cursor = 0;
            return;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(1);
        self.order = shuffled_indices(self.playlist.len(), seed);
        self.order_cursor = self
            .order
            .iter()
            .position(|index| *index == self.playlist_index)
            .unwrap_or(0);
    }

    fn step_playlist(&mut self, delta: i32) {
        if self.shuffle && !self.order.is_empty() {
            let wrap = self.repeat == RepeatMode::All;
            if let Some((index, cursor)) = order_step(&self.order, self.order_cursor, delta, wrap) {
                self.order_cursor = cursor;
                self.goto_playlist(index);
            }
            return;
        }
        if let Some(index) = playlist_step(self.playlist.len(), self.playlist_index, delta) {
            self.goto_playlist(index);
        }
    }

    fn nudge_subtitle_delay(&mut self, steps: i32) {
        self.subtitle_delay_us = subtitle_delay_us(self.subtitle_delay_us, steps);
        let notice = format_delay_osd("subtitle", self.subtitle_delay_us);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn nudge_secondary_subtitle_delay(&mut self, steps: i32) {
        self.secondary_subtitle_delay_us =
            secondary_subtitle_delay_us(self.secondary_subtitle_delay_us, steps);
        let notice = format_secondary_subtitle_delay_osd(self.secondary_subtitle_delay_us);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn nudge_audio_delay(&mut self, steps: i32) {
        let next = subtitle_delay_us(self.audio_delay_us, steps);
        let Some(session) = &self.session else {
            self.audio_delay_us = next;
            return;
        };
        let rate = session.shared.sample_rate.load(Ordering::Relaxed);
        let delta = audio_delay_frames(next, rate) - audio_delay_frames(self.audio_delay_us, rate);
        self.audio_delay_us = next;
        session.shared.audio_delay_us.store(next, Ordering::Relaxed);
        session
            .shared
            .audio_skew_frames
            .fetch_add(delta, Ordering::Relaxed);
        let notice = format_delay_osd("audio", next);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn reset_av_sync(&mut self) {
        let (sub, audio) = reset_av_delays();
        let prev_audio = self.audio_delay_us;
        self.subtitle_delay_us = sub;
        self.secondary_subtitle_delay_us = 0;
        self.audio_delay_us = audio;
        if let Some(session) = &self.session {
            let rate = session.shared.sample_rate.load(Ordering::Relaxed);
            let delta = audio_delay_frames(audio, rate) - audio_delay_frames(prev_audio, rate);
            session
                .shared
                .audio_delay_us
                .store(audio, Ordering::Relaxed);
            session
                .shared
                .audio_skew_frames
                .fetch_add(delta, Ordering::Relaxed);
        }
        self.notice = Some("A/V sync reset".into());
    }

    fn cycle_aspect_mode(&mut self) {
        self.aspect = cycle_aspect(self.aspect);
        let notice = format_aspect_osd(self.aspect);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn cycle_crop_mode(&mut self) {
        self.crop = cycle_aspect(self.crop);
        let notice = format_crop_osd(self.crop);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn nudge_zoom(&mut self, zoom_in: bool) {
        self.zoom_milli = zoom_step(self.zoom_milli, zoom_in);
        let (_, pan_x, pan_y) = reset_zoom_pan();
        self.pan_x_px = pan_x;
        self.pan_y_px = pan_y;
        let notice = format_zoom_osd(self.zoom_milli);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn nudge_pan(&mut self, dx: i32, dy: i32, viewport_w: u32, viewport_h: u32) {
        let (content_w, content_h) = {
            let (zw, zh) = zoom_size(viewport_w.max(1), viewport_h.max(1), self.zoom_milli);
            (zw, zh)
        };
        self.pan_x_px = pan_step_px(self.pan_x_px, dx, viewport_w, content_w);
        self.pan_y_px = pan_step_px(self.pan_y_px, dy, viewport_h, content_h);
        if self.zoom_milli > 1_000 {
            self.notice = Some(format_pan_osd(self.pan_x_px, self.pan_y_px));
        }
    }

    fn toggle_on_top(&mut self) {
        self.on_top = !self.on_top;
        self.on_top_dirty = true;
        let notice = if self.on_top {
            "always on top"
        } else {
            "not always on top"
        };
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice.into());
    }

    fn toggle_spherical(&mut self) {
        self.spherical = !self.spherical;
        self.adjust_dirty = true;
        let notice = format_spherical_osd_ex(
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
        );
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn nudge_yaw(&mut self, delta: i32) {
        self.yaw_deg_milli = yaw_step_milli(self.yaw_deg_milli, delta);
        self.adjust_dirty = true;
        self.notice = Some(format_spherical_osd_ex(
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
        ));
    }

    fn nudge_pitch(&mut self, delta: i32) {
        self.pitch_deg_milli = pitch_step_milli(self.pitch_deg_milli, delta);
        self.adjust_dirty = true;
        self.notice = Some(format_spherical_osd_ex(
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
        ));
    }

    fn nudge_roll(&mut self, delta: i32) {
        self.roll_deg_milli = roll_step_milli(self.roll_deg_milli, delta);
        self.adjust_dirty = true;
        self.notice = Some(format_spherical_osd_ex(
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
        ));
    }

    fn nudge_fov(&mut self, delta: i32) {
        self.fov_deg_milli = fov_step_milli(self.fov_deg_milli, delta);
        self.adjust_dirty = true;
        self.notice = Some(format_spherical_osd_ex(
            self.spherical,
            self.yaw_deg_milli,
            self.pitch_deg_milli,
            self.roll_deg_milli,
            self.fov_deg_milli,
        ));
    }

    fn cycle_subtitle_pos(&mut self) {
        self.subtitle_position = cycle_subtitle_position(self.subtitle_position);
        self.notice = Some(format_subtitle_position_osd(self.subtitle_position));
    }

    fn tick_osd_and_mouse(&mut self, ctx: &egui::Context) {
        match &self.notice {
            Some(text) => {
                if self.notice_text.as_deref() != Some(text.as_str()) {
                    self.notice_text = Some(text.clone());
                    self.notice_at = Some(Instant::now());
                }
            }
            None => {
                self.notice_text = None;
                self.notice_at = None;
            }
        }
        if let Some(at) = self.notice_at
            && osd_should_clear(at.elapsed().as_millis() as u64, self.osd_timeout_ms)
        {
            self.notice = None;
            self.notice_text = None;
            self.notice_at = None;
        }
        if ctx.input(|input| input.pointer.delta().length_sq() > 0.0 || !input.events.is_empty()) {
            // Only count pointer motion for hide timer.
            if ctx.input(|input| input.pointer.delta().length_sq() > 0.0) {
                self.mouse_moved_at = Instant::now();
            }
        }
        let idle = self.mouse_moved_at.elapsed().as_millis() as u64;
        if mouse_should_hide(idle, self.mouse_hide_ms) {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
    }

    fn cycle_hdr_mode(&mut self) {
        self.hdr_tonemap = cycle_hdr_tonemap(self.hdr_tonemap);
        self.hdr_auto_applied = true;
        self.adjust_dirty = true;
        let notice = format_hdr_tonemap_osd(self.hdr_tonemap);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn cycle_stereo3d_mode(&mut self) {
        self.stereo3d = cycle_play_stereo3d(self.stereo3d);
        self.adjust_dirty = true;
        let notice = format_play_stereo3d_osd(self.stereo3d);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn show_media_info(&mut self) {
        let (width, height, color_trc) = self
            .session
            .as_ref()
            .and_then(|session| session.frame.as_ref())
            .map(|frame| (frame.width, frame.height, frame.color_trc))
            .unwrap_or((0, 0, 0));
        let title = self
            .session
            .as_ref()
            .map(|session| {
                let stored = lock(&session.shared.media_title);
                if stored.is_empty() {
                    media_display_title(&session.path, None)
                } else {
                    stored.clone()
                }
            })
            .unwrap_or_else(|| "media".into());
        let notice = format_media_info_osd(
            &title,
            width,
            height,
            self.duration_us(),
            color_trc,
            self.spherical,
        );
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn jump_random(&mut self) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(1);
        let Some(target) = random_seek_us(self.duration_us(), seed) else {
            self.notice = Some("Random seek unavailable".into());
            return;
        };
        self.request_seek(target);
        self.notice = Some(format!("Random {}", format_clock(target)));
    }

    fn maybe_auto_hdr(&mut self) {
        if self.hdr_auto_applied {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let trc = session
            .frame
            .as_ref()
            .map(|frame| frame.color_trc)
            .unwrap_or_else(|| session.shared.color_trc.load(Ordering::Relaxed));
        if is_hdr_transfer(trc) {
            self.hdr_tonemap = auto_hdr_tonemap(trc);
            self.hdr_auto_applied = true;
            self.adjust_dirty = true;
            self.notice = Some(format_hdr_tonemap_osd(self.hdr_tonemap));
        }
    }

    fn cycle_position_osd(&mut self) {
        self.position_display = cycle_position_display(self.position_display);
        let duration = self.duration_us();
        let now = self
            .session
            .as_ref()
            .map(|session| session.media_now)
            .unwrap_or(0);
        let notice = format_position_osd(now, duration, self.position_display);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
        self.title.clear();
    }

    fn toggle_volume_normalizer(&mut self) {
        self.volume_normalizer = !self.volume_normalizer;
        let gain = if let Some(session) = self.session.as_ref() {
            session
                .shared
                .normalizer_on
                .store(self.volume_normalizer, Ordering::Relaxed);
            if !self.volume_normalizer {
                session
                    .shared
                    .normalizer_peak_milli
                    .store(0, Ordering::Relaxed);
            }
            session.shared.normalizer_peak_milli.load(Ordering::Relaxed)
        } else {
            0
        };
        let gain = if gain == 0 {
            1_000
        } else {
            normalizer_gain_milli(gain as f32 / 1_000.0, 0.95)
        };
        let notice = format_normalizer_osd(self.volume_normalizer, gain);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn add_bookmark(&mut self) {
        let now = self.shown_media_us().max(0);
        let added = insert_bookmark(&mut self.bookmarks, now);
        let notice = format_bookmark_osd(now, self.bookmarks.len(), added);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn clear_all_bookmarks(&mut self) {
        clear_bookmarks(&mut self.bookmarks);
        self.notice = Some("Bookmarks cleared".into());
    }

    fn save_bookmarks(&mut self) {
        if self.bookmarks.is_empty() {
            self.notice = Some("No bookmarks".into());
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save bookmarks")
            .add_filter("M3U", &["m3u", "m3u8", "txt"])
            .set_file_name("bookmarks.m3u");
        if let Some(dir) = self.playlist.first().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let body = format_bookmarks_export(&self.bookmarks);
        match std::fs::write(&path, body) {
            Ok(()) => {
                self.notice = Some(format!(
                    "Saved {} bookmarks → {}",
                    self.bookmarks.len(),
                    path.display()
                ))
            }
            Err(err) => self.notice = Some(err.to_string()),
        }
    }

    fn load_bookmarks(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Load bookmarks")
            .add_filter("M3U", &["m3u", "m3u8", "txt"]);
        if let Some(dir) = self.playlist.first().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let marks = parse_bookmarks_export(&text);
                if marks.is_empty() {
                    self.notice = Some("No bookmarks in file".into());
                } else {
                    self.bookmarks = marks;
                    self.notice = Some(format!("Loaded {} bookmarks", self.bookmarks.len()));
                }
            }
            Err(err) => self.notice = Some(err.to_string()),
        }
    }

    fn save_playlist(&mut self) {
        if self.playlist.is_empty() {
            self.notice = Some("Playlist empty".into());
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save playlist")
            .add_filter("M3U", &["m3u", "m3u8"])
            .set_file_name("playlist.m3u");
        if let Some(dir) = self.playlist.first().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let body = format_playlist_m3u(&self.playlist);
        match std::fs::write(&path, body) {
            Ok(()) => self.notice = Some(format!("Saved {}", path.display())),
            Err(err) => self.notice = Some(err.to_string()),
        }
    }

    fn step_bookmark(&mut self, delta: i32) {
        let Some(target) = bookmark_step(&self.bookmarks, self.shown_media_us(), delta) else {
            return;
        };
        eprintln!("fvid play: bookmark {}", format_clock(target));
        self.request_seek(target);
    }

    fn request_seek(&mut self, media_us: i64) {
        let Some(session) = &mut self.session else {
            return;
        };
        let duration = session.shared.duration_us.load(Ordering::Relaxed);
        let target = clamp_seek_us(media_us, duration);
        session.discard_until = Some(target);
        session.shared.seek_us.store(target, Ordering::Release);
        lock(&session.shared.video).clear();
        lock(&session.shared.audio).clear();
        lock(&session.shared.cues).clear();
        session.shared.played_samples.store(0, Ordering::Relaxed);
        reset_audio_rate(&session.shared);
        rearm_audio_skew(&session.shared);
        session.shared.audio_eof.store(false, Ordering::Release);
        session.shared.finished.store(false, Ordering::Release);
        session.shared.origin_us.store(target, Ordering::Release);
        session.clock.jump(target);
        session.last_clock = target;
        session.media_now = target;
        session.audio_switched = false;
        session.shared.video_cv.notify_all();
        session.shared.audio_cv.notify_all();
    }

    fn set_paused(&mut self, paused: bool) {
        let Some(session) = &mut self.session else {
            return;
        };
        if session.shared.paused.load(Ordering::Relaxed) == paused {
            return;
        }
        session.shared.paused.store(paused, Ordering::Relaxed);
        if paused {
            session.clock.pause();
        } else {
            session.clock.resume();
        }
    }

    fn set_rate(&mut self, rate_milli: u32) {
        let rate_milli = rate_milli.clamp(RATE_MIN_MILLI, RATE_MAX_MILLI);
        self.rate_milli = rate_milli;
        if let Some(session) = &mut self.session {
            session.clock.set_rate(rate_milli);
            session
                .shared
                .rate_milli
                .store(rate_milli, Ordering::Relaxed);
            session.last_clock = session.clock.now();
            session.media_now = session.last_clock;
        }
        self.notice = Some(format_rate_osd(rate_milli));
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(session) = &self.session {
            session.shared.muted.store(muted, Ordering::Relaxed);
        }
        self.notice = Some(format_volume_osd(self.volume_milli, self.muted));
    }

    fn nudge_rate(&mut self, delta_milli: i32) {
        self.set_rate(rate_step_milli(self.rate_milli, delta_milli));
    }

    fn stop_playback(&mut self) {
        self.scrub = None;
        self.request_seek(stop_playback_us());
        self.set_paused(true);
        self.notice = Some(format_stop_osd().into());
    }

    fn nudge_volume(&mut self, delta_milli: i32) {
        self.volume_milli = volume_step_milli(self.volume_milli, delta_milli);
        if let Some(session) = &self.session {
            session
                .shared
                .volume_milli
                .store(self.volume_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_volume_osd(self.volume_milli, self.muted));
    }

    fn nudge_balance(&mut self, delta: i32) {
        self.balance_milli = balance_step_milli(self.balance_milli, delta);
        if let Some(session) = &self.session {
            session
                .shared
                .balance_milli
                .store(self.balance_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_balance_osd(self.balance_milli));
    }

    fn nudge_stereo_width(&mut self, delta: i32) {
        self.width_milli = width_step_milli(self.width_milli, delta);
        if let Some(session) = &self.session {
            session
                .shared
                .width_milli
                .store(self.width_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_width_osd(self.width_milli));
    }

    fn nudge_crossfeed(&mut self, delta: i32) {
        self.crossfeed_milli = crossfeed_step_milli(self.crossfeed_milli, delta);
        if let Some(session) = &self.session {
            session
                .shared
                .crossfeed_milli
                .store(self.crossfeed_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_crossfeed_osd(self.crossfeed_milli));
    }

    fn toggle_compressor(&mut self) {
        self.compressor_on = !self.compressor_on;
        if let Some(session) = &self.session {
            session
                .shared
                .compressor_on
                .store(self.compressor_on, Ordering::Relaxed);
        }
        let notice = format_compressor_osd(self.compressor_on);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice.into());
    }

    fn cycle_sleep_timer(&mut self) {
        self.sleep_min = cycle_sleep_timer_min(self.sleep_min);
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        self.sleep_deadline_secs = sleep_deadline_secs(self.sleep_min, now_secs);
        let notice = format_sleep_osd(self.sleep_min);
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
    }

    fn toggle_eq_bypass(&mut self) {
        self.eq_bypass = cycle_eq_bypass(self.eq_bypass);
        if let Some(session) = &self.session {
            session
                .shared
                .eq_bypass
                .store(self.eq_bypass, Ordering::Relaxed);
        }
        self.notice = Some(format_eq_bypass_osd(self.eq_bypass).into());
    }

    fn cycle_audio_channel_mode(&mut self) {
        self.audio_channel = cycle_audio_channel(self.audio_channel);
        if let Some(session) = &self.session {
            session.shared.audio_channel.store(
                match self.audio_channel {
                    AudioChannelMode::Stereo => 0,
                    AudioChannelMode::Left => 1,
                    AudioChannelMode::Right => 2,
                    AudioChannelMode::Mono => 3,
                    AudioChannelMode::Reverse => 4,
                    AudioChannelMode::Karaoke => 5,
                },
                Ordering::Relaxed,
            );
        }
        self.notice = Some(format_audio_channel_osd(self.audio_channel));
    }

    fn nudge_eq_preamp(&mut self, delta: i32) {
        self.eq_preamp_milli = eq_preamp_step_milli(self.eq_preamp_milli, delta);
        if let Some(session) = &self.session {
            session
                .shared
                .eq_preamp_milli
                .store(self.eq_preamp_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_eq_preamp_osd(self.eq_preamp_milli));
    }

    fn nudge_spatializer(&mut self, delta: i32) {
        self.spatializer_milli = spatializer_step_milli(self.spatializer_milli, delta);
        if let Some(session) = &self.session {
            session
                .shared
                .spatializer_milli
                .store(self.spatializer_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_spatializer_osd(self.spatializer_milli));
    }

    fn toggle_gapless(&mut self) {
        self.gapless = !self.gapless;
        self.notice = Some(format_gapless_osd(self.gapless).into());
    }

    fn nudge_crossfade(&mut self, delta: i32) {
        self.crossfade_ms = crossfade_step_ms(self.crossfade_ms, delta);
        self.notice = Some(format_crossfade_osd(self.crossfade_ms));
    }

    fn cycle_replaygain_boost(&mut self) {
        self.replaygain_milli = if self.replaygain_milli == REPLAYGAIN_UNITY_MILLI {
            replaygain_milli_from_db_milli(-6_000)
        } else {
            REPLAYGAIN_UNITY_MILLI
        };
        if let Some(session) = &self.session {
            session
                .shared
                .replaygain_milli
                .store(self.replaygain_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_replaygain_osd(self.replaygain_milli));
    }

    fn cycle_subtitle_color_mode(&mut self) {
        self.subtitle_color = cycle_subtitle_color(self.subtitle_color);
        self.notice = Some(format_subtitle_color_osd(self.subtitle_color));
    }

    fn cycle_playlist_sort_mode(&mut self) {
        self.playlist_sort = cycle_playlist_sort(self.playlist_sort);
        sort_playlist_paths(&mut self.playlist, self.playlist_sort);
        self.playlist_index = self
            .playlist_index
            .min(self.playlist.len().saturating_sub(1));
        self.notice = Some(format_playlist_sort_osd(self.playlist_sort));
    }

    fn show_loudness_osd(&mut self) {
        let peak = self
            .session
            .as_ref()
            .map(|session| session.shared.vu_peak_milli.load(Ordering::Relaxed))
            .unwrap_or(0);
        self.notice = Some(format_loudness_osd(peak));
    }

    fn show_recent_osd(&mut self) {
        self.notice = Some(format_recent_osd(&self.recent));
    }

    fn cycle_post_fx_mode(&mut self) {
        self.post_fx = cycle_video_post_fx(self.post_fx);
        self.adjust_dirty = true;
        self.notice = Some(format_video_post_fx_osd(self.post_fx));
    }

    fn cycle_seek_jump_size(&mut self) {
        self.seek_jump = cycle_seek_jump(self.seek_jump);
        self.notice = Some(format_seek_jump_osd(self.seek_jump));
    }

    fn toggle_surround_downmix(&mut self) {
        self.surround_downmix = !self.surround_downmix;
        if let Some(session) = &self.session {
            session
                .shared
                .surround_downmix
                .store(self.surround_downmix, Ordering::Relaxed);
        }
        self.notice = Some(format_downmix_osd(self.surround_downmix).into());
    }

    fn toggle_scaletempo(&mut self) {
        self.scaletempo = !self.scaletempo;
        self.notice = Some(format_scaletempo_osd(self.scaletempo).into());
    }

    fn toggle_minimal_interface(&mut self) {
        self.minimal_interface = !self.minimal_interface;
        self.notice = Some(format_minimal_interface_osd(self.minimal_interface).into());
    }

    fn nudge_audio_pitch(&mut self, delta: i32) {
        self.pitch_milli = audio_pitch_step_milli(self.pitch_milli, delta);
        self.notice = Some(format_audio_pitch_osd(self.pitch_milli));
    }

    fn cycle_visualization_mode(&mut self) {
        self.visualization = cycle_visualization(self.visualization);
        self.notice = Some(format_visualization_osd(self.visualization));
    }

    fn nudge_image_duration(&mut self, delta: i32) {
        let next = (self.image_duration_secs as i64).saturating_add(i64::from(delta));
        self.image_duration_secs =
            clamp_image_duration_secs(next.clamp(1, i64::from(IMAGE_DURATION_MAX_SECS)) as u32);
        self.notice = Some(format_image_duration_osd(self.image_duration_secs));
    }

    fn cycle_closed_captions(&mut self) {
        self.closed_captions = cycle_closed_caption(self.closed_captions);
        self.notice = Some(format_closed_caption_osd(self.closed_captions));
    }

    fn toggle_wallpaper_mode(&mut self) {
        self.wallpaper_mode = !self.wallpaper_mode;
        self.notice = Some(format_wallpaper_osd(self.wallpaper_mode).into());
    }

    fn cycle_subtitle_encoding_mode(&mut self) {
        self.subtitle_encoding = cycle_subtitle_encoding(self.subtitle_encoding);
        self.notice = Some(format_subtitle_encoding_osd(self.subtitle_encoding));
    }

    fn nudge_crop_pixels(&mut self, left: i32, top: i32, right: i32, bottom: i32) {
        let add = |base: u32, delta: i32| -> u32 {
            (base as i64 + i64::from(delta)).clamp(0, 4_000) as u32
        };
        self.crop_pixels = CropPixels {
            left: add(self.crop_pixels.left, left),
            top: add(self.crop_pixels.top, top),
            right: add(self.crop_pixels.right, right),
            bottom: add(self.crop_pixels.bottom, bottom),
        };
        self.adjust_dirty = true;
        self.notice = Some(format_crop_pixels_osd(self.crop_pixels));
    }

    fn toggle_teletext(&mut self) {
        self.teletext_enabled = !self.teletext_enabled;
        self.notice = Some(format_teletext_osd(
            self.teletext_page,
            self.teletext_enabled,
        ));
    }

    fn nudge_teletext_page(&mut self, delta: i32) {
        self.teletext_page = teletext_page_step(self.teletext_page, delta);
        self.teletext_enabled = true;
        self.notice = Some(format_teletext_osd(
            self.teletext_page,
            self.teletext_enabled,
        ));
    }

    fn toggle_aspect_lock(&mut self) {
        self.aspect_lock = !self.aspect_lock;
        self.notice = Some(format_aspect_lock_osd(self.aspect_lock).into());
    }

    fn toggle_snapshot_sequential(&mut self) {
        self.snapshot_sequential = !self.snapshot_sequential;
        self.notice = Some(format_snapshot_sequential_osd(
            self.snapshot_sequential,
            self.snapshots,
        ));
    }

    fn toggle_hw_decode(&mut self) {
        self.hw_decode = !self.hw_decode;
        self.notice = Some(format_hw_decode_osd(self.hw_decode).into());
    }

    fn cycle_logo_pos(&mut self) {
        self.logo_position = cycle_logo_position(self.logo_position);
        self.notice = Some(format_logo_osd(self.logo_position, self.logo_opacity_milli));
    }

    fn cycle_mosaic_grid(&mut self) {
        match (self.mosaic_cols, self.mosaic_rows) {
            (1, 1) => {
                self.mosaic_cols = 2;
                self.mosaic_rows = 2;
            }
            (2, 2) => {
                self.mosaic_cols = 3;
                self.mosaic_rows = 2;
            }
            _ => {
                self.mosaic_cols = 1;
                self.mosaic_rows = 1;
            }
        }
        self.notice = Some(format_mosaic_osd(self.mosaic_cols, self.mosaic_rows));
    }

    fn nudge_param_eq(&mut self, delta: i32) {
        self.param_eq_milli = clamp_param_eq_milli(self.param_eq_milli.saturating_add(delta));
        self.notice = Some(format_param_eq_osd(self.param_eq_milli));
    }

    fn nudge_amplifier(&mut self, delta: i32) {
        self.amplifier_milli = clamp_amplifier_milli(self.amplifier_milli.saturating_add(delta));
        self.notice = Some(format_amplifier_osd(self.amplifier_milli));
    }

    fn toggle_recording(&mut self) {
        self.recording = !self.recording;
        let path = if self.recording {
            Some(format_record_path(
                self.record_dir.as_deref(),
                "capture",
                self.snapshots,
                "mkv",
            ))
        } else {
            None
        };
        self.notice = Some(format_record_osd(self.recording, path.as_deref()));
    }

    fn cycle_proxy(&mut self) {
        self.proxy_mode = cycle_proxy_mode(self.proxy_mode);
        self.notice = Some(format_proxy_osd(self.proxy_mode, &self.proxy_host));
    }

    fn toggle_http_auth_flags(&mut self) {
        match (self.http_auth_user, self.http_auth_password) {
            (false, false) => self.http_auth_user = true,
            (true, false) => self.http_auth_password = true,
            _ => {
                self.http_auth_user = false;
                self.http_auth_password = false;
            }
        }
        self.notice =
            Some(format_http_auth_osd(self.http_auth_user, self.http_auth_password).into());
    }

    fn cycle_spherical_projection_mode(&mut self) {
        self.spherical_projection = cycle_spherical_projection(self.spherical_projection);
        self.spherical = true;
        self.adjust_dirty = true;
        self.notice = Some(format_spherical_projection_osd(self.spherical_projection));
    }

    fn show_hdr_metadata_osd(&mut self) {
        let trc = self
            .session
            .as_ref()
            .map(|s| s.shared.color_trc.load(Ordering::Relaxed))
            .unwrap_or(0);
        self.notice = Some(format_hdr_metadata_osd(
            self.hdr_maxcll,
            self.hdr_maxfall,
            trc,
        ));
    }

    fn cycle_video_track_mode(&mut self) {
        self.video_track = cycle_video_track(self.video_track_count, self.video_track);
        self.notice = Some(format_video_track_osd(
            self.video_track,
            self.video_track_count,
        ));
    }

    fn toggle_silence_skip(&mut self) {
        self.silence_skip = !self.silence_skip;
        self.notice = Some(if self.silence_skip {
            "Silence skip On".into()
        } else {
            "Silence skip Off".into()
        });
    }

    fn try_skip_marker(&mut self) {
        let now = self.shown_media_us();
        if let Some(target) = skip_marker_target_us(now, self.intro_end_us, self.credits_start_us) {
            let kind = if self.intro_end_us.is_some_and(|end| now < end) {
                "intro"
            } else {
                "credits"
            };
            self.request_seek(target);
            self.notice = Some(format_skip_marker_osd(kind, target));
        }
    }

    fn set_playlist_filter(&mut self, query: String) {
        self.playlist_filter = query;
        let matched = filter_playlist_paths(&self.playlist, &self.playlist_filter).len();
        self.notice = Some(format_playlist_filter_osd(
            &self.playlist_filter,
            matched,
            self.playlist.len(),
        ));
    }

    fn toggle_current_favorite(&mut self) {
        let Some(path) = self.playlist.get(self.playlist_index).cloned() else {
            return;
        };
        let starred = toggle_favorite(&mut self.favorites, path);
        self.notice = Some(format_favorite_osd(starred).into());
    }

    fn toggle_remote_control(&mut self) {
        self.remote_control = !self.remote_control;
        self.notice = Some(format_remote_control_osd(
            self.remote_control,
            self.remote_control_port,
        ));
    }

    fn toggle_bitperfect(&mut self) {
        self.bitperfect = !self.bitperfect;
        self.notice = Some(format_bitperfect_osd(self.bitperfect).into());
    }

    fn toggle_night_mode(&mut self) {
        self.night_mode = !self.night_mode;
        self.notice = Some(format_night_mode_osd(self.night_mode).into());
    }

    fn queue_current(&mut self, at_front: bool) {
        queue_insert(&mut self.play_queue, self.playlist_index, at_front);
        self.notice = Some(format_queue_osd(self.play_queue.len()));
    }

    fn toggle_forced_subs_only(&mut self) {
        self.forced_subs_only = !self.forced_subs_only;
        self.notice = Some(format_forced_only_osd(self.forced_subs_only).into());
    }

    fn show_frame_rate_osd(&mut self) {
        let dur = self
            .session
            .as_ref()
            .and_then(|s| s.frame.as_ref())
            .map(|f| f.duration_us)
            .unwrap_or(0);
        self.notice = Some(format_frame_rate_osd(dur));
    }

    fn nudge_ipd(&mut self, delta: i32) {
        self.ipd_milli = ipd_step_milli(self.ipd_milli, delta);
        self.notice = Some(format_ipd_osd(self.ipd_milli));
    }

    fn cycle_vr_mode(&mut self) {
        self.vr_display = cycle_vr_display(self.vr_display);
        if !matches!(self.vr_display, VrDisplayMode::Off) {
            self.spherical = true;
        }
        self.adjust_dirty = true;
        self.notice = Some(format_vr_display_osd(self.vr_display));
    }

    fn cycle_ambisonic_mode(&mut self) {
        self.ambisonic = cycle_ambisonic(self.ambisonic);
        self.notice = Some(format_ambisonic_osd(self.ambisonic));
    }

    fn cycle_cast_mode(&mut self) {
        self.cast_protocol = cycle_cast_protocol(self.cast_protocol);
        self.notice = Some(format_cast_osd(self.cast_protocol, &self.cast_device));
    }

    fn show_active_lyric(&mut self) {
        let now = self.shown_media_us();
        self.notice = Some(format_lyric_osd(active_lyric_line(&self.lyric_lines, now)));
    }

    fn store_ab_slot(&mut self, index: usize) {
        let pair = self.ab.and_then(|loop_| {
            if loop_.b_us > loop_.a_us {
                Some((loop_.a_us, loop_.b_us))
            } else {
                None
            }
        });
        if let Some((a, b)) = pair {
            ab_slot_store(&mut self.ab_slots, index, a, b);
        }
        self.notice = Some(format_ab_slot_osd(
            index,
            ab_slot_load(&self.ab_slots, index),
        ));
    }

    fn load_ab_slot(&mut self, index: usize) {
        if let Some((a, b)) = ab_slot_load(&self.ab_slots, index) {
            self.ab = Some(AbLoop { a_us: a, b_us: b });
            self.notice = Some(format_ab_slot_osd(index, Some((a, b))));
        } else {
            self.notice = Some(format_ab_slot_osd(index, None));
        }
    }

    fn toggle_pip(&mut self) {
        self.pip_enabled = !self.pip_enabled;
        self.notice = Some(format_pip_osd(self.pip_enabled).into());
    }

    fn toggle_horizon_lock(&mut self) {
        self.horizon_lock = !self.horizon_lock;
        if self.horizon_lock {
            self.horizon_pitch_milli = self.pitch_deg_milli;
        }
        self.notice = Some(format_horizon_lock_osd(self.horizon_lock).into());
    }

    fn apply_horizon_lock_pitch(&mut self) {
        if self.horizon_lock {
            self.pitch_deg_milli =
                locked_pitch_milli(self.pitch_deg_milli, true, self.horizon_pitch_milli);
        }
    }

    fn show_hdr_peak_suggestion(&mut self) {
        let peak = self
            .session
            .as_ref()
            .and_then(|s| s.frame.as_ref())
            .map(|f| estimate_frame_peak_milli(&f.pixels, 16))
            .unwrap_or(0);
        let suggested = suggest_hdr_nits_from_peak(peak, self.hdr_nits);
        self.notice = Some(format_hdr_peak_osd(peak, suggested));
    }

    fn cycle_spherical_stereo_layout(&mut self) {
        self.spherical_stereo = cycle_spherical_stereo(self.spherical_stereo);
        self.notice = Some(format_spherical_stereo_osd(self.spherical_stereo));
    }

    fn recenter_view(&mut self) {
        let (y, p, r) = recenter_spherical_view();
        self.yaw_deg_milli = y;
        self.pitch_deg_milli = p;
        self.roll_deg_milli = r;
        self.horizon_pitch_milli = 0;
        self.adjust_dirty = true;
        self.notice = Some(format_recenter_osd().into());
    }

    fn show_compass(&mut self) {
        self.notice = Some(format_compass_osd(self.yaw_deg_milli));
    }

    fn cycle_tonemap_strength(&mut self) {
        let next = match self.tonemap_strength_milli {
            0 => 500,
            1..=500 => 1_000,
            _ => 0,
        };
        self.tonemap_strength_milli = clamp_tonemap_strength_milli(next);
        self.notice = Some(format_tonemap_strength_osd(self.tonemap_strength_milli));
    }

    fn cycle_hdr_highlight_desat(&mut self) {
        let next = match self.hdr_highlight_desat_milli {
            0 => 350,
            1..=350 => 700,
            _ => 0,
        };
        self.hdr_highlight_desat_milli = next;
        self.notice = Some(format_hdr_highlight_desat_osd(
            self.hdr_highlight_desat_milli,
        ));
    }

    fn cycle_color_temp(&mut self) {
        let next = match self.color_temp_kelvin {
            ..=4_000 => COLOR_TEMP_DAYLIGHT_K,
            4_001..=6_500 => 9_300,
            _ => 3_200,
        };
        self.color_temp_kelvin = clamp_color_temp_kelvin(next);
        self.notice = Some(format_color_temp_osd(self.color_temp_kelvin));
    }

    fn cycle_barrel_distortion(&mut self) {
        let next = match self.barrel_k_milli {
            0 => 300,
            1..=300 => 600,
            _ => 0,
        };
        self.barrel_k_milli = next;
        self.notice = Some(format_barrel_osd(self.barrel_k_milli));
    }

    fn toggle_audio_duck(&mut self) {
        self.audio_duck_enabled = !self.audio_duck_enabled;
        self.notice = Some(format_audio_duck_osd(
            self.audio_duck_enabled,
            self.audio_duck_milli,
        ));
    }

    fn toggle_waveform(&mut self) {
        self.waveform_enabled = !self.waveform_enabled;
        self.notice = Some(format_waveform_osd(self.waveform_enabled).into());
    }

    fn detect_and_show_letterbox(&mut self) {
        let bars = self
            .session
            .as_ref()
            .and_then(|s| s.frame.as_ref())
            .map(|f| detect_letterbox_bars(f.width, f.height, &f.pixels, 16))
            .unwrap_or((0, 0, 0, 0));
        self.notice = Some(format_letterbox_osd(bars.0, bars.1, bars.2, bars.3));
    }

    fn cycle_fov_preset(&mut self) {
        self.fov_deg_milli = cycle_fov_preset_milli(self.fov_deg_milli);
        self.adjust_dirty = true;
        self.notice = Some(format_fov_preset_osd(self.fov_deg_milli));
    }

    fn toggle_hdr10_plus(&mut self) {
        self.hdr10_plus = !self.hdr10_plus;
        self.notice = Some(format_hdr10_plus_osd(self.hdr10_plus).into());
    }

    fn cycle_hlg_ootf(&mut self) {
        let next = match self.hlg_ootf_gamma_milli {
            ..=1_000 => 1_200,
            1_001..=1_200 => 1_500,
            _ => 1_000,
        };
        self.hlg_ootf_gamma_milli = next;
        self.notice = Some(format_hlg_ootf_osd(self.hlg_ootf_gamma_milli));
    }

    fn show_hdr_headroom(&mut self) {
        let content = if self.hdr_mastering_max_nits > 0 {
            self.hdr_mastering_max_nits
        } else if self.hdr_maxcll > 0 {
            self.hdr_maxcll
        } else {
            self.hdr_nits
        };
        self.notice = Some(format_hdr_headroom_osd(content, self.display_peak_nits));
    }

    fn show_timecode(&mut self) {
        let pos = self
            .session
            .as_ref()
            .map(|s| s.shared.watch_us.load(Ordering::Relaxed))
            .unwrap_or(0);
        self.notice = Some(format_timecode_osd(pos, 24_000));
    }

    fn cycle_denoise(&mut self) {
        let next = match self.denoise_milli {
            0 => 350,
            1..=350 => 700,
            _ => 0,
        };
        self.denoise_milli = next;
        self.notice = Some(format_box_denoise_osd(self.denoise_milli));
    }

    fn cycle_dialogue_enhance(&mut self) {
        let next = match self.dialogue_enhance_milli {
            0 => 300,
            1..=300 => 600,
            _ => 0,
        };
        self.dialogue_enhance_milli = next;
        self.notice = Some(format_dialogue_enhance_osd(self.dialogue_enhance_milli));
    }

    fn toggle_vectorscope(&mut self) {
        self.vectorscope_enabled = !self.vectorscope_enabled;
        self.notice = Some(format_vectorscope_osd(self.vectorscope_enabled).into());
    }

    fn toggle_gyro_look(&mut self) {
        self.gyro_look = !self.gyro_look;
        self.notice = Some(format_gyro_osd(self.gyro_look).into());
    }

    fn cycle_vr_vignette(&mut self) {
        let next = match self.vr_vignette_milli {
            0 => 350,
            1..=350 => 700,
            _ => 0,
        };
        self.vr_vignette_milli = next;
        self.notice = Some(format_vr_vignette_osd(self.vr_vignette_milli));
    }

    fn cycle_hdr_black_lift(&mut self) {
        let next = match self.hdr_black_lift_milli {
            0 => 80,
            1..=80 => 160,
            _ => 0,
        };
        self.hdr_black_lift_milli = next;
        self.notice = Some(format_hdr_black_lift_osd(self.hdr_black_lift_milli));
    }

    fn cycle_unsharp(&mut self) {
        let next = match self.unsharp_milli {
            0 => 400,
            1..=400 => 800,
            _ => 0,
        };
        self.unsharp_milli = next;
        self.notice = Some(format_unsharp_osd(self.unsharp_milli));
    }

    fn toggle_clipboard_snapshot(&mut self) {
        self.clipboard_snapshot = !self.clipboard_snapshot;
        self.notice = Some(format_clipboard_snapshot_osd(self.clipboard_snapshot).into());
    }

    fn show_eac_face(&mut self) {
        let yaw = (clamp_yaw_milli(self.yaw_deg_milli) as f32 / 1_000.0).to_radians();
        let pitch = (clamp_pitch_milli(self.pitch_deg_milli) as f32 / 1_000.0).to_radians();
        let x = yaw.cos() * pitch.cos();
        let y = pitch.sin();
        let z = yaw.sin() * pitch.cos();
        let (face, _, _) = eac_face_uv_from_dir(x, y, z);
        self.notice = Some(format_eac_face_osd(face));
    }

    fn toggle_watch_party(&mut self) {
        self.watch_party = !self.watch_party;
        self.notice = Some(format_watch_party_osd(
            self.watch_party,
            self.watch_party_offset_us,
        ));
    }

    fn toggle_auto_horizon(&mut self) {
        self.auto_horizon = !self.auto_horizon;
        self.notice = Some(format_auto_horizon_osd(self.auto_horizon).into());
    }

    fn show_live_edge(&mut self) {
        let playhead = self
            .session
            .as_ref()
            .map(|s| s.shared.watch_us.load(Ordering::Relaxed))
            .unwrap_or(0);
        let duration = self
            .session
            .as_ref()
            .map(|s| s.shared.duration_us.load(Ordering::Relaxed))
            .unwrap_or(0);
        let lag = timeshift_lag_us(duration, playhead);
        self.notice = Some(format_live_edge_osd(lag));
    }

    fn do_instant_replay(&mut self) {
        let window = 10_000_000i64;
        let playhead = self
            .session
            .as_ref()
            .map(|s| s.shared.watch_us.load(Ordering::Relaxed))
            .unwrap_or(0);
        let target = instant_replay_us(playhead, window);
        if let Some(session) = &self.session {
            session.shared.seek_us.store(target, Ordering::Release);
        }
        self.notice = Some(format_instant_replay_osd(window));
    }

    fn show_hdr_sdr_ratio(&mut self) {
        let content = if self.hdr_mastering_max_nits > 0 {
            self.hdr_mastering_max_nits
        } else if self.hdr_maxcll > 0 {
            self.hdr_maxcll
        } else {
            self.hdr_nits
        };
        let ratio = hdr_sdr_ratio_milli(content, self.display_peak_nits);
        self.notice = Some(format_hdr_sdr_ratio_osd(ratio));
    }

    fn cycle_ambisonic_channel_order(&mut self) {
        self.ambisonic_order = cycle_ambisonic_order(self.ambisonic_order);
        self.notice = Some(format_ambisonic_order_osd(self.ambisonic_order));
    }

    fn cycle_chromatic_aberration(&mut self) {
        let next = match self.chromatic_aberration_milli {
            0 => 250,
            1..=250 => 500,
            _ => 0,
        };
        self.chromatic_aberration_milli = next;
        self.notice = Some(format_chromatic_aberration_osd(
            self.chromatic_aberration_milli,
        ));
    }

    fn cycle_soft_limiter(&mut self) {
        let next = match self.soft_limiter_milli {
            1_000 => 800,
            700..=999 => 500,
            _ => 1_000,
        };
        self.soft_limiter_milli = next;
        self.notice = Some(format_soft_limiter_osd(self.soft_limiter_milli));
    }

    fn cycle_echo_feedback(&mut self) {
        let next = match self.echo_feedback_milli {
            0 => 250,
            1..=250 => 500,
            _ => 0,
        };
        self.echo_feedback_milli = next;
        self.notice = Some(format_echo_osd(self.echo_feedback_milli));
    }

    fn toggle_anaglyph_dubois(&mut self) {
        self.anaglyph_dubois = !self.anaglyph_dubois;
        self.notice = Some(format_anaglyph_dubois_osd(self.anaglyph_dubois).into());
    }

    fn toggle_bt2446(&mut self) {
        self.bt2446_tonemap = !self.bt2446_tonemap;
        if self.bt2446_tonemap && matches!(self.hdr_tonemap, HdrTonemap::Off) {
            self.hdr_tonemap = HdrTonemap::Hable;
        }
        self.adjust_dirty = true;
        self.notice = Some(format_bt2446_osd(self.bt2446_tonemap).into());
    }

    fn toggle_gamut_map(&mut self) {
        self.gamut_map_bt709 = !self.gamut_map_bt709;
        self.adjust_dirty = true;
        self.notice = Some(format_gamut_map_osd(self.gamut_map_bt709).into());
    }

    fn cycle_chorus(&mut self) {
        let next = match self.chorus_milli {
            0 => 300,
            1..=300 => 600,
            _ => 0,
        };
        self.chorus_milli = next;
        self.notice = Some(format_chorus_osd(self.chorus_milli));
    }

    fn cycle_reverb(&mut self) {
        let next = match self.reverb_milli {
            0 => 300,
            1..=300 => 600,
            _ => 0,
        };
        self.reverb_milli = next;
        self.notice = Some(format_reverb_osd(self.reverb_milli));
    }

    fn cycle_atempo(&mut self) {
        let next = match self.atempo_milli {
            1_000 => 800,
            700..=999 => 1_250,
            _ => 1_000,
        };
        self.atempo_milli = next;
        self.notice = Some(format_atempo_osd(self.atempo_milli));
    }

    fn set_tone_gains(&mut self, bass: i32, mid: i32, treble: i32) {
        self.bass_milli = clamp_adjust_milli(bass);
        self.mid_milli = clamp_adjust_milli(mid);
        self.treble_milli = clamp_adjust_milli(treble);
        if let Some(session) = &self.session {
            session
                .shared
                .bass_milli
                .store(self.bass_milli, Ordering::Relaxed);
            session
                .shared
                .mid_milli
                .store(self.mid_milli, Ordering::Relaxed);
            session
                .shared
                .treble_milli
                .store(self.treble_milli, Ordering::Relaxed);
        }
        self.notice = Some(format_tone_osd(
            self.bass_milli,
            self.mid_milli,
            self.treble_milli,
        ));
    }

    fn nudge_subtitle_scale(&mut self, delta: i32) {
        self.subtitle_scale_milli = subtitle_scale_step_milli(self.subtitle_scale_milli, delta);
        self.notice = Some(format_subtitle_scale_osd(self.subtitle_scale_milli));
    }

    fn nudge_subtitle_opacity(&mut self, delta: i32) {
        self.subtitle_opacity_milli =
            subtitle_opacity_step_milli(self.subtitle_opacity_milli, delta);
        self.notice = Some(format_subtitle_opacity_osd(self.subtitle_opacity_milli));
    }

    fn fit_window_video(&mut self) {
        let (vw, vh) = self
            .session
            .as_ref()
            .and_then(|session| session.frame.as_ref())
            .map(|frame| (frame.width, frame.height))
            .unwrap_or((0, 0));
        let (width, height) = fit_window_to_video(vw, vh, 1920, 1080);
        if width == 0 || height == 0 {
            self.notice = Some("Fit unavailable".into());
            return;
        }
        self.notice = Some(format_fit_window_osd(width, height));
    }

    fn maybe_auto_equirect(&mut self) {
        if self.spherical || self.options.spherical {
            return;
        }
        let Some(frame) = self
            .session
            .as_ref()
            .and_then(|session| session.frame.as_ref())
        else {
            return;
        };
        if detect_equirect_aspect(frame.width, frame.height) {
            self.spherical = true;
            self.adjust_dirty = true;
            self.notice = Some(format_spherical_osd_ex(
                true,
                self.yaw_deg_milli,
                self.pitch_deg_milli,
                self.roll_deg_milli,
                self.fov_deg_milli,
            ));
        }
    }

    fn cycle_audio(&mut self, delta: i32) {
        let Some(session) = &self.session else {
            return;
        };
        let count = session.shared.audio_count.load(Ordering::Relaxed) as usize;
        let next = cycle_track(count, self.audio_ordinal, delta, false);
        self.audio_ordinal = next;
        session.shared.audio_ordinal.store(next, Ordering::Release);
        session.shared.track_gen.fetch_add(1, Ordering::Release);
        lock(&session.shared.cues).clear();
        session.shared.video_cv.notify_all();
        session.shared.audio_cv.notify_all();
        self.notice = Some(format_track_osd("Audio", next, count));
    }

    fn cycle_subtitle(&mut self, delta: i32) {
        let Some(session) = &self.session else {
            return;
        };
        let count = session.shared.subtitle_count.load(Ordering::Relaxed) as usize;
        let next = cycle_track(count, self.subtitle_ordinal, delta, true);
        self.subtitle_ordinal = next;
        session
            .shared
            .subtitle_ordinal
            .store(next, Ordering::Release);
        session.shared.track_gen.fetch_add(1, Ordering::Release);
        lock(&session.shared.cues).clear();
        self.logged_sub.clear();
        session.shared.video_cv.notify_all();
        session.shared.audio_cv.notify_all();
        self.notice = Some(format_track_osd("Subtitles", next, count));
    }

    fn current_subtitle(&self) -> String {
        let Some(session) = &self.session else {
            return String::new();
        };
        let now = subtitle_clock_us(session.media_now, self.subtitle_delay_us);
        let external = session.shared.external_sub.load(Ordering::Acquire);
        let count = session.shared.subtitle_count.load(Ordering::Acquire) as i32;
        let ordinal = session.shared.subtitle_ordinal.load(Ordering::Acquire);
        if external && count > 0 && ordinal == count - 1 {
            let cues = lock(&session.shared.external_cues);
            return active_subtitle(&cues, now).unwrap_or("").to_string();
        }
        let cues = lock(&session.shared.cues);
        let owned: Vec<SubtitleCue> = cues.iter().cloned().collect();
        active_subtitle(&owned, now).unwrap_or("").to_string()
    }

    fn load_external_subtitles(&mut self, path: PathBuf) {
        match load_subtitle_file(&path) {
            Ok(cues) => {
                self.options.subtitles = Some(path);
                if let Some(session) = &self.session {
                    let was_external = session.shared.external_sub.swap(true, Ordering::AcqRel);
                    *lock(&session.shared.external_cues) = cues;
                    if !was_external {
                        session.shared.subtitle_count.fetch_add(1, Ordering::AcqRel);
                    }
                    let count = session.shared.subtitle_count.load(Ordering::Acquire) as i32;
                    let ordinal = (count - 1).max(0);
                    self.subtitle_ordinal = ordinal;
                    session
                        .shared
                        .subtitle_ordinal
                        .store(ordinal, Ordering::Release);
                    session.shared.track_gen.fetch_add(1, Ordering::Release);
                    session.shared.video_cv.notify_all();
                }
                self.logged_sub.clear();
                self.notice = None;
            }
            Err(err) => self.notice = Some(err),
        }
    }

    fn replace_playlist(&mut self, paths: Vec<PathBuf>) {
        let paths = match expand_play_inputs(&paths) {
            Ok(paths) => paths,
            Err(err) => {
                self.notice = Some(err);
                return;
            }
        };
        if paths.is_empty() {
            return;
        }
        self.playlist = paths;
        self.playlist_index = 0;
        self.advance_guard = true;
        self.rebuild_order();
        self.notice = None;
        self.pending = Some(self.playlist[0].clone());
    }

    fn goto_playlist(&mut self, index: usize) {
        if index >= self.playlist.len() {
            return;
        }
        self.playlist_index = index;
        if self.shuffle
            && let Some(cursor) = self.order.iter().position(|item| *item == index)
        {
            self.order_cursor = cursor;
        }
        self.advance_guard = true;
        self.pending = Some(self.playlist[index].clone());
    }

    fn step_frame(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        if !session.shared.paused.load(Ordering::Relaxed) {
            session.shared.paused.store(true, Ordering::Relaxed);
            session.clock.pause();
        }
        let next = {
            let mut queue = lock(&session.shared.video);
            let frame = queue.pop_front();
            drop(queue);
            session.shared.video_cv.notify_all();
            frame
        };
        let Some(frame) = next else {
            let now = session.media_now;
            let step = session
                .frame
                .as_ref()
                .map(|frame| frame.duration_us.max(1))
                .unwrap_or(40_000);
            drop(session);
            self.request_seek(frame_step_target_us(now, step, FrameStep::Forward));
            self.set_paused(true);
            return;
        };
        let pts = frame.pts_us;
        session.frame = Some(frame);
        session.dirty = true;
        session.presented += 1;
        session.discard_until = None;
        session.clock.jump(pts);
        session.last_clock = pts;
        session.media_now = pts;
        session.shared.origin_us.store(pts, Ordering::Release);
        session.shared.played_samples.store(0, Ordering::Relaxed);
        reset_audio_rate(&session.shared);
        rearm_audio_skew(&session.shared);
        lock(&session.shared.audio).clear();
    }

    fn step_frame_back(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let now = session.media_now;
        let step = session
            .frame
            .as_ref()
            .map(|frame| frame.duration_us.max(1))
            .unwrap_or(40_000);
        let target = frame_step_target_us(now, step, FrameStep::Backward);
        drop(session);
        self.request_seek(target);
        self.set_paused(true);
    }

    fn save_snapshot(&mut self) {
        let encoded = {
            let Some(session) = &self.session else {
                return;
            };
            let Some(frame) = &session.frame else {
                self.notice = Some("No frame to save".into());
                return;
            };
            let now = self.shown_media_us();
            let bitmap = {
                let mut planes = lock(&session.shared.bitmaps);
                active_bitmap_subtitle(planes.make_contiguous(), now).cloned()
            };
            let opts = PlayRenderOptions {
                brightness_milli: self.brightness_milli,
                contrast_milli: self.contrast_milli,
                saturation_milli: self.saturation_milli,
                hue_milli: self.hue_milli,
                gamma_milli: self.gamma_milli,
                flip_h: self.flip_h,
                flip_v: self.flip_v,
                rotate: self.rotate,
                deinterlace: self.deinterlace,
                stereo3d: self.stereo3d,
                spherical: self.spherical,
                yaw_deg_milli: self.yaw_deg_milli,
                pitch_deg_milli: self.pitch_deg_milli,
                roll_deg_milli: self.roll_deg_milli,
                fov_deg_milli: self.fov_deg_milli,
                hdr_tonemap: self.hdr_tonemap,
                color_trc: frame.color_trc,
                display_effect: self.display_effect,
                hdr_nits: self.hdr_nits,
                post_fx: self.post_fx,
                spherical_projection: self.spherical_projection,
                hdr_maxcll: self.hdr_maxcll,
                hdr_maxfall: self.hdr_maxfall,
                color_primaries: self.color_primaries,
                tonemap_strength_milli: self.tonemap_strength_milli,
                hdr_highlight_desat_milli: self.hdr_highlight_desat_milli,
                hdr_black_lift_milli: self.hdr_black_lift_milli,
                color_temp_kelvin: self.color_temp_kelvin,
                hlg_ootf_gamma_milli: self.hlg_ootf_gamma_milli,
                bt2446_tonemap: self.bt2446_tonemap,
                gamut_map_bt709: self.gamut_map_bt709,
            };
            let (width, height, pixels) = render_play_pixels(
                frame.width,
                frame.height,
                &frame.pixels,
                &opts,
                bitmap.as_ref(),
            );
            let bytes = match self.snapshot_format {
                SnapshotFormat::Bmp => encode_bmp(width, height, &pixels),
                SnapshotFormat::Png => encode_png(width, height, &pixels),
            };
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.notice = Some(err);
                    return;
                }
            };
            let path = snapshot_path_with_prefix(
                self.snapshot_dir.as_deref(),
                &session.path,
                &self.snapshot_prefix,
                self.snapshots + 1,
                snapshot_format_ext(self.snapshot_format),
            );
            (path, bytes)
        };
        match std::fs::write(&encoded.0, &encoded.1) {
            Ok(()) => {
                self.snapshots = next_snapshot_index(self.snapshots);
                self.notice = Some(format!("Saved {}", encoded.0.display()));
            }
            Err(err) => self.notice = Some(err.to_string()),
        }
    }

    fn file_name(&self) -> String {
        self.session
            .as_ref()
            .map(|session| {
                let stored = lock(&session.shared.media_title);
                if stored.is_empty() {
                    media_display_title(&session.path, None)
                } else {
                    stored.clone()
                }
            })
            .unwrap_or_default()
    }

    fn jump_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Go");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.jump_text)
                    .hint_text("mm:ss / seconds")
                    .desired_width(120.0),
            );
            let submit = ui.button("Jump").clicked()
                || (edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if !submit {
                return;
            }
            match parse_play_clock(&self.jump_text) {
                Some(target) => {
                    let target = clamp_seek_us(target, self.duration_us());
                    self.request_seek(target);
                    self.notice = Some(format_jump_osd(target));
                }
                None => {
                    self.notice = Some("Jump time must be mm:ss, hh:mm:ss, or seconds".into());
                }
            }
        });
    }

    fn url_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.url_text)
                    .hint_text("http://  https://  rtsp://")
                    .desired_width(ui.available_width() - 88.0),
            );
            let submit = ui.button("Open URL").clicked()
                || (edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if !submit {
                return;
            }
            let text = self.url_text.trim().to_string();
            if is_playback_url(&text) {
                self.notice = None;
                self.advance_guard = true;
                self.playlist = vec![PathBuf::from(&text)];
                self.playlist_index = 0;
                self.rebuild_order();
                self.pending = Some(PathBuf::from(text));
            } else {
                self.notice = Some("URL scheme is not a playback protocol".into());
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if self.ended() {
                "Replay"
            } else if self.is_paused() {
                "Play"
            } else {
                "Pause"
            };
            if ui
                .add_sized([72.0, 28.0], egui::Button::new(label))
                .clicked()
            {
                self.toggle_pause();
            }
            if ui
                .add_sized([72.0, 28.0], egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_playback();
            }
            let can_prev = if self.shuffle {
                order_step(
                    &self.order,
                    self.order_cursor,
                    -1,
                    self.repeat == RepeatMode::All,
                )
                .is_some()
            } else {
                playlist_step(self.playlist.len(), self.playlist_index, -1).is_some()
            };
            if ui
                .add_enabled(can_prev, egui::Button::new("Prev"))
                .clicked()
            {
                self.step_playlist(-1);
            }
            let can_next = if self.shuffle {
                order_step(
                    &self.order,
                    self.order_cursor,
                    1,
                    self.repeat == RepeatMode::All,
                )
                .is_some()
            } else {
                playlist_step(self.playlist.len(), self.playlist_index, 1).is_some()
            };
            if ui
                .add_enabled(can_next, egui::Button::new("Next"))
                .clicked()
            {
                self.step_playlist(1);
            }
            if ui
                .add_sized([72.0, 28.0], egui::Button::new("Open"))
                .clicked()
            {
                self.pick_file();
            }
            let ab_label = match self.ab {
                Some(loop_) if loop_.b_us >= 0 => "Loop",
                Some(_) => "A…",
                None => "A-B",
            };
            if ui
                .add_sized([56.0, 28.0], egui::Button::new(ab_label))
                .clicked()
            {
                self.mark_ab();
            }
            if ui.button("Ch-").clicked() {
                self.step_chapter(-1);
            }
            if ui.button("Ch+").clicked() {
                self.step_chapter(1);
            }
            let repeat_label = match self.repeat {
                RepeatMode::Off => "Repeat",
                RepeatMode::All => "All",
                RepeatMode::One => "One",
            };
            if ui.button(repeat_label).clicked() {
                self.cycle_repeat_mode();
            }
            let shuffle_label = if self.shuffle { "Rand" } else { "Order" };
            if ui.button(shuffle_label).clicked() {
                self.toggle_shuffle();
            }
            if ui.button(aspect_label(self.aspect)).clicked() {
                self.cycle_aspect_mode();
            }
            let crop_label = if self.crop == AspectMode::Source {
                "Crop"
            } else {
                aspect_label(self.crop)
            };
            if ui.button(crop_label).clicked() {
                self.cycle_crop_mode();
            }
            if ui.button("Z+").clicked() {
                self.nudge_zoom(true);
            }
            if ui.button("Z-").clicked() {
                self.nudge_zoom(false);
            }
            if ui.button("Mark").clicked() {
                self.add_bookmark();
            }
            let top_label = if self.on_top { "Top*" } else { "Top" };
            if ui.button(top_label).clicked() {
                self.toggle_on_top();
            }
            let full = if self.fullscreen { "Window" } else { "Full" };
            if ui.add(egui::Button::new(full)).clicked() {
                self.fullscreen = !self.fullscreen;
                self.fullscreen_dirty = true;
            }
            ui.label(
                egui::RichText::new(self.clock_label())
                    .monospace()
                    .size(16.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = format!("{}/{}", self.playlist_index + 1, self.playlist.len().max(1));
                ui.label(count);
                ui.label(egui::RichText::new(self.file_name()).strong());
            });
        });
        ui.add_space(4.0);
        self.seek_bar(ui);
        self.volume_row(ui);
        self.track_row(ui);
        self.url_row(ui);
        self.jump_row(ui);
        if self.show_osd {
            if let Some(notice) = &self.notice {
                ui.colored_label(egui::Color32::from_rgb(255, 186, 92), notice);
            }
        }
        ui.add_space(4.0);
    }

    fn show_video(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_rgb(8, 8, 8));
        if let Some(err) = &self.error {
            paint_center(ui, rect, err, egui::Color32::from_rgb(255, 140, 140));
            return;
        }
        let Some(texture) = &self.texture else {
            paint_center(
                ui,
                rect,
                "Opening...",
                egui::Color32::from_rgb(180, 180, 180),
            );
            return;
        };
        let tex_id = texture.id();
        let tex_size = texture.size_vec2();
        let tex_w = texture.size()[0] as u32;
        let tex_h = texture.size()[1] as u32;
        let source_w = tex_size.x.round().max(1.0) as u32;
        let source_h = tex_size.y.round().max(1.0) as u32;
        let (ratio_w, ratio_h) = display_ratio(source_w, source_h, self.aspect, self.crop);
        let (width, height) = fit_aspect(
            rect.width().max(0.0) as u32,
            rect.height().max(0.0) as u32,
            ratio_w,
            ratio_h,
        );
        let (zoomed_w, zoomed_h) = zoom_size(width, height, self.zoom_milli);
        let size = egui::vec2(zoomed_w as f32, zoomed_h as f32);
        let (crop_x, crop_y, crop_w, crop_h) = if self.crop == AspectMode::Source {
            (0, 0, source_w, source_h)
        } else {
            let (rw, rh) = frame_aspect(source_w, source_h, self.crop);
            center_crop(source_w, source_h, rw, rh)
        };
        let (u0, v0, u1, v1) = flip_uv(
            (
                crop_x as f32 / source_w as f32,
                crop_y as f32 / source_h as f32,
                (crop_x + crop_w) as f32 / source_w as f32,
                (crop_y + crop_h) as f32 / source_h as f32,
            ),
            self.flip_h,
            self.flip_v,
        );
        let uv = egui::Rect::from_min_max(egui::pos2(u0, v0), egui::pos2(u1, v1));
        let image = egui::Image::from_texture(egui::load::SizedTexture::new(tex_id, tex_size))
            .uv(uv)
            .fit_to_exact_size(size);
        ui.set_clip_rect(rect);
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if self.spherical && response.dragged() {
            let delta = response.drag_delta();
            self.yaw_deg_milli =
                yaw_step_milli(self.yaw_deg_milli, (delta.x * 200.0).round() as i32);
            self.pitch_deg_milli =
                pitch_step_milli(self.pitch_deg_milli, (-delta.y * 200.0).round() as i32);
            self.adjust_dirty = true;
        } else if self.zoom_milli > 1_000 && response.dragged() {
            let delta = response.drag_delta();
            self.pan_x_px = clamp_pan_px(
                self.pan_x_px + delta.x.round() as i32,
                rect.width().max(0.0) as u32,
                zoomed_w,
            );
            self.pan_y_px = clamp_pan_px(
                self.pan_y_px + delta.y.round() as i32,
                rect.height().max(0.0) as u32,
                zoomed_h,
            );
        }
        if self.spherical {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                self.nudge_fov(if scroll > 0.0 {
                    -FOV_STEP_MILLI
                } else {
                    FOV_STEP_MILLI
                });
            }
        }
        let (px, py, pw, ph) = zoom_pan_rect(
            (rect.min.x, rect.min.y, rect.width(), rect.height()),
            size.x,
            size.y,
            self.pan_x_px,
            self.pan_y_px,
        );
        let image_rect = egui::Rect::from_min_size(egui::pos2(px, py), egui::vec2(pw, ph));
        image.paint_at(ui, image_rect);
        paint_subtitle(
            ui,
            image_rect,
            &self.current_subtitle(),
            self.subtitle_margin_px,
            self.subtitle_scale_milli,
            self.subtitle_opacity_milli,
            self.subtitle_position,
            self.subtitle_color,
        );
        if !self.marquee_text.trim().is_empty() {
            paint_marquee(ui, image_rect, &self.marquee_text, self.marquee_position);
        }
        if self.show_stats {
            let stats = PlayStats {
                presented_frames: self
                    .session
                    .as_ref()
                    .map(|session| session.presented)
                    .unwrap_or(0),
                skipped_frames: self
                    .session
                    .as_ref()
                    .map(|session| session.skipped)
                    .unwrap_or(0),
                width: tex_w,
                height: tex_h,
                source_width: self
                    .session
                    .as_ref()
                    .map(|session| session.shared.source_width.load(Ordering::Relaxed))
                    .unwrap_or(0),
                source_height: self
                    .session
                    .as_ref()
                    .map(|session| session.shared.source_height.load(Ordering::Relaxed))
                    .unwrap_or(0),
                audio: self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.stream.is_some()),
                sample_rate: self
                    .session
                    .as_ref()
                    .map(|session| session.shared.sample_rate.load(Ordering::Relaxed))
                    .unwrap_or(0),
                channels: self
                    .session
                    .as_ref()
                    .map(|session| session.shared.channels.load(Ordering::Relaxed))
                    .unwrap_or(0),
            };
            let line = format_play_stats(&stats, self.shown_media_us(), self.duration_us());
            ui.painter().text(
                image_rect.left_top() + egui::vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::monospace(14.0),
                egui::Color32::from_rgb(220, 220, 220),
            );
        }
        ui.allocate_rect(rect, egui::Sense::hover());
    }
}

impl Drop for PlayerApp {
    fn drop(&mut self) {
        let report = self.report();
        self.stop_session();
        *lock(&self.outcome) = Some(report);
    }
}

impl eframe::App for PlayerApp {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick_osd_and_mouse(ctx);
        self.read_input(ctx);
        if self.fullscreen_dirty {
            self.fullscreen_dirty = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if self.on_top_dirty {
            self.on_top_dirty = false;
            let level = if self.on_top {
                egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop)
            } else {
                egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal)
            };
            ctx.send_viewport_cmd(level);
        }
        if self.ended() {
            if !self.advance_guard {
                if self.shuffle && !self.order.is_empty() && self.repeat != RepeatMode::One {
                    let wrap = self.repeat == RepeatMode::All;
                    if let Some((index, cursor)) =
                        order_step(&self.order, self.order_cursor, 1, wrap)
                    {
                        self.order_cursor = cursor;
                        self.goto_playlist(index);
                    } else if self.options.quit_at_end {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                } else {
                    match playback_continue(self.playlist.len(), self.playlist_index, self.repeat) {
                        PlaybackContinue::Next(index) => self.goto_playlist(index),
                        PlaybackContinue::Restart => {
                            self.advance_guard = true;
                            self.request_seek(0);
                            self.set_paused(false);
                        }
                        PlaybackContinue::Stop => {
                            if self.options.quit_at_end {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                    }
                }
            }
        } else {
            self.advance_guard = false;
        }
        if let Some(path) = self.pending.take() {
            if let Err(err) = self.start_session(path) {
                self.error = Some(err);
            }
        }
        if self.error.is_none()
            && let Some(err) = self.pump()
        {
            self.error = Some(err);
        }
        if !self.is_paused()
            && let Some(loop_) = self.ab
            && let Some(target) = ab_restart_us(loop_, self.shown_media_us())
        {
            self.request_seek(target);
        }
        if self.step_pending {
            self.step_pending = false;
            self.step_frame();
        }
        let subtitle = self.current_subtitle();
        if !subtitle.is_empty() && subtitle != self.logged_sub {
            eprintln!("fvid play: subtitle: {subtitle}");
            self.logged_sub = subtitle;
        }
        self.upload(ctx);
        self.sync_title(ctx);
        if let Some(session) = &self.session {
            self.audio_ordinal = session.shared.audio_ordinal.load(Ordering::Relaxed);
            self.subtitle_ordinal = session.shared.subtitle_ordinal.load(Ordering::Relaxed);
        }
        if self.wants_frames() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let idle = self.mouse_moved_at.elapsed().as_millis() as u64;
        let hide_controls = controls_should_hide(idle, self.controls_autohide_ms, self.fullscreen);
        if !hide_controls {
            egui::Panel::bottom("controls").show(ui, |ui| self.controls(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.show_video(ui));
        paint_drop_hover(&ctx);
    }
}

fn color_image(
    frame: &VideoFrame,
    brightness_milli: i32,
    contrast_milli: i32,
    saturation_milli: i32,
    hue_milli: i32,
    gamma_milli: i32,
    flip_h: bool,
    flip_v: bool,
    rotate: RotateMode,
    deinterlace: DeinterlaceMode,
    stereo3d: PlayStereo3D,
    spherical: bool,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
    hdr_tonemap: HdrTonemap,
    color_trc: u32,
    display_effect: DisplayEffect,
    hdr_nits: u32,
    post_fx: VideoPostFx,
    spherical_projection: SphericalProjection,
    hdr_maxcll: u32,
    hdr_maxfall: u32,
    color_primaries: u32,
    tonemap_strength_milli: i32,
    hdr_highlight_desat_milli: i32,
    hdr_black_lift_milli: i32,
    color_temp_kelvin: i32,
    hlg_ootf_gamma_milli: i32,
    bt2446_tonemap: bool,
    gamut_map_bt709: bool,
    bitmap: Option<&BitmapSubtitle>,
) -> egui::ColorImage {
    let opts = PlayRenderOptions {
        brightness_milli,
        contrast_milli,
        saturation_milli,
        hue_milli,
        gamma_milli,
        flip_h,
        flip_v,
        rotate,
        deinterlace,
        stereo3d,
        spherical,
        yaw_deg_milli,
        pitch_deg_milli,
        roll_deg_milli,
        fov_deg_milli,
        hdr_tonemap,
        color_trc,
        display_effect,
        hdr_nits,
        post_fx,
        spherical_projection,
        hdr_maxcll,
        hdr_maxfall,
        color_primaries,
        tonemap_strength_milli,
        hdr_highlight_desat_milli,
        hdr_black_lift_milli,
        color_temp_kelvin,
        hlg_ootf_gamma_milli,
        bt2446_tonemap,
        gamut_map_bt709,
    };
    let (out_w, out_h, rgb) =
        render_play_pixels(frame.width, frame.height, &frame.pixels, &opts, bitmap);
    let pixels: Vec<egui::Color32> = rgb
        .into_iter()
        .map(|pixel| {
            egui::Color32::from_rgb(
                ((pixel >> 16) & 0xff) as u8,
                ((pixel >> 8) & 0xff) as u8,
                (pixel & 0xff) as u8,
            )
        })
        .collect();
    egui::ColorImage::new([out_w as usize, out_h as usize], pixels)
}

fn paint_subtitle(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    margin_px: i32,
    scale_milli: i32,
    opacity_milli: i32,
    position: SubtitlePosition,
    color: SubtitleColor,
) {
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        return;
    }
    let size = subtitle_font_px(22.0, scale_milli);
    let line_h = size + 4.0;
    let font = egui::FontId::proportional(size);
    let margin = subtitle_margin_px(12, margin_px) as f32;
    let alpha = subtitle_opacity_u8(opacity_milli);
    let rgba = subtitle_color_rgba(color);
    let fill = egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], alpha);
    let shadow = egui::Color32::from_rgba_unmultiplied(0, 0, 0, alpha);
    let mut y =
        rect.top() + subtitle_block_top_y(rect.height(), lines.len(), line_h, margin, position);
    for line in lines {
        let pos = egui::pos2(rect.center().x, y);
        ui.painter().text(
            pos + egui::vec2(1.5, 1.5),
            egui::Align2::CENTER_TOP,
            line,
            font.clone(),
            shadow,
        );
        ui.painter()
            .text(pos, egui::Align2::CENTER_TOP, line, font.clone(), fill);
        y += line_h;
    }
}

fn paint_marquee(ui: &egui::Ui, rect: egui::Rect, text: &str, position: MarqueePosition) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let size = 20.0;
    let font = egui::FontId::proportional(size);
    let y = rect.top() + marquee_block_top_y(rect.height(), size + 4.0, 12.0, position);
    let pos = egui::pos2(rect.center().x, y);
    ui.painter().text(
        pos + egui::vec2(1.5, 1.5),
        egui::Align2::CENTER_TOP,
        trimmed,
        font.clone(),
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200),
    );
    ui.painter().text(
        pos,
        egui::Align2::CENTER_TOP,
        trimmed,
        font,
        egui::Color32::from_rgb(255, 220, 120),
    );
}

fn paint_center(ui: &mut egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(18.0),
        color,
    );
    ui.allocate_rect(rect, egui::Sense::hover());
}

fn paint_drop_hover(ctx: &egui::Context) {
    if ctx.input(|input| input.raw.hovered_files.is_empty()) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("file_drop_hover"),
    ));
    let screen = ctx.viewport_rect();
    painter.rect_filled(
        screen,
        0.0,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 170),
    );
    painter.text(
        screen.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to play",
        egui::FontId::proportional(28.0),
        egui::Color32::WHITE,
    );
}

fn window_title(name: &str, shared: &Shared, media_now: i64, position: PositionDisplay) -> String {
    format_window_title_with_position(
        name,
        media_now,
        shared.duration_us.load(Ordering::Relaxed),
        shared.paused.load(Ordering::Relaxed),
        shared.rate_milli.load(Ordering::Relaxed),
        position,
    )
}

fn master_clock(
    shared: &Shared,
    clock: &mut Clock,
    last: &mut i64,
    audio_switched: &mut bool,
) -> i64 {
    let wall = clock.now();
    let chosen = if shared.use_audio_clock.load(Ordering::Acquire) {
        let origin = shared.origin_us.load(Ordering::Acquire);
        let rate = i64::from(shared.sample_rate.load(Ordering::Relaxed).max(1));
        let played = shared.played_samples.load(Ordering::Relaxed) as i64;
        let audio_us = origin.saturating_add(played.saturating_mul(1_000_000) / rate);
        let queued = lock(&shared.audio).len();
        let audio_done = shared.audio_eof.load(Ordering::Acquire) && queued == 0;
        if audio_done {
            if !*audio_switched {
                clock.snap(audio_us.max(*last));
                *audio_switched = true;
            }
            clock.now()
        } else if queued == 0 {
            wall
        } else {
            *audio_switched = false;
            if clock.paused_at.is_none() && (wall - audio_us).abs() > 40_000 {
                clock.snap(audio_us);
            }
            audio_us
        }
    } else {
        wall
    };
    let chosen = chosen.max(*last);
    *last = chosen;
    chosen
}

struct Scaler {
    ctx: *mut SwsContext,
    src_w: i32,
    src_h: i32,
    src_fmt: i32,
    dst_w: i32,
    dst_h: i32,
}

impl Default for Scaler {
    fn default() -> Self {
        Self {
            ctx: ptr::null_mut(),
            src_w: 0,
            src_h: 0,
            src_fmt: -1,
            dst_w: 0,
            dst_h: 0,
        }
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { sws_freeContext(self.ctx) }
        }
    }
}

struct Scaled {
    pixels: Vec<u32>,
    width: u32,
    height: u32,
}

impl Scaler {
    fn convert(&mut self, frame: *mut AVFrame) -> Result<Scaled> {
        unsafe {
            let source = &*frame;
            if source.width <= 0 || source.height <= 0 {
                return Err("decoded video frame has no size".into());
            }
            if self.dst_w == 0 {
                let (width, height) = fit_display_size(source.width, source.height)?;
                self.dst_w = width as i32;
                self.dst_h = height as i32;
            }
            if self.ctx.is_null()
                || self.src_w != source.width
                || self.src_h != source.height
                || self.src_fmt != source.format
            {
                if !self.ctx.is_null() {
                    sws_freeContext(self.ctx);
                    self.ctx = ptr::null_mut();
                }
                let ctx = sws_getContext(
                    source.width,
                    source.height,
                    source.format,
                    self.dst_w,
                    self.dst_h,
                    AVPixelFormat_AV_PIX_FMT_BGRA,
                    SWS_BILINEAR,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                if ctx.is_null() {
                    return Err("display scaler allocation failed".into());
                }
                self.ctx = ctx;
                self.src_w = source.width;
                self.src_h = source.height;
                self.src_fmt = source.format;
            }
            let pixels = (self.dst_w as usize)
                .checked_mul(self.dst_h as usize)
                .ok_or("frame too large")?;
            let mut bgra = vec![0u8; pixels.checked_mul(4).ok_or("frame too large")?];
            let stride = self.dst_w * 4;
            let dst_planes = [
                bgra.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ];
            let dst_stride = [stride, 0, 0, 0];
            let code = sws_scale(
                self.ctx,
                source.data.as_ptr().cast::<*const u8>(),
                source.linesize.as_ptr(),
                0,
                source.height,
                dst_planes.as_ptr(),
                dst_stride.as_ptr(),
            );
            if code <= 0 {
                return Err("display scale failed".into());
            }
            Ok(Scaled {
                width: self.dst_w as u32,
                height: self.dst_h as u32,
                pixels: pack_bgra(
                    &bgra,
                    stride as usize,
                    self.dst_w as usize,
                    self.dst_h as usize,
                )?,
            })
        }
    }
}

struct PlayResampler {
    owned_s64: Option<crate::owned_resample_f64::Resampler<Vec<u8>>>,
    input_rate: i32,
    input_channels: i32,
    input_format: i32,
    owned: Option<crate::audio::Resampler>,
    swr: *mut SwrContext,
    layout: AVChannelLayout,
    rate: i32,
    channels: i32,
}

impl Drop for PlayResampler {
    fn drop(&mut self) {
        unsafe {
            channel_layout_uninit_owned(&mut self.layout);
            if !self.swr.is_null() {
                swr_free(&mut self.swr);
            }
        }
    }
}

impl PlayResampler {
    unsafe fn open(frame: *const AVFrame, rate: i32, channels: i32) -> Result<Self> {
        unsafe {
            if rate <= 0 || !(1..=8).contains(&channels) {
                return Err("invalid playback audio format".into());
            }
            let source = &*frame;
            let mut layout = std::mem::zeroed();
            channel_layout_default_owned(&mut layout, channels);
            if layout.nb_channels != channels {
                return Err("failed to build playback channel layout".into());
            }
            let mut swr = ptr::null_mut();
            let mut built = Self {
                owned_s64: None,
                input_rate: source.sample_rate,
                input_channels: source.ch_layout.nb_channels,
                input_format: source.format,
                owned: None,
                swr: ptr::null_mut(),
                layout,
                rate,
                channels,
            };
            let matching_layout = source.ch_layout.order == AVChannelOrder_AV_CHANNEL_ORDER_NATIVE
                && built.layout.order == AVChannelOrder_AV_CHANNEL_ORDER_NATIVE
                && (source.ch_layout.u.mask == built.layout.u.mask
                    || (matches!(channels, 1 | 2) && source.ch_layout.nb_channels <= 8
                        || matches!(source.ch_layout.nb_channels, 1 | 2) && channels <= 8));
            let s64_layout = matching_layout && (channels == source.ch_layout.nb_channels
                || crate::owned_pcm_channels::standard_mask(source.ch_layout.nb_channels as u16).is_some_and(|mask| mask == source.ch_layout.u.mask));
            if s64_layout && matches!(source.format, AVSampleFormat_AV_SAMPLE_FMT_S64 | AVSampleFormat_AV_SAMPLE_FMT_S64P) {
                built.owned_s64 = Some(crate::owned_resample_f64::Resampler::new(
                    Vec::new(), u32::try_from(source.sample_rate).map_err(|_| "invalid S64 playback rate")?,
                    rate as u32, channels as u16).map_err(|e| e.to_string())?);
                return Ok(built);
            }
            if matching_layout && matches!(source.format, AVSampleFormat_AV_SAMPLE_FMT_FLT | AVSampleFormat_AV_SAMPLE_FMT_FLTP | AVSampleFormat_AV_SAMPLE_FMT_DBL | AVSampleFormat_AV_SAMPLE_FMT_DBLP | AVSampleFormat_AV_SAMPLE_FMT_U8 | AVSampleFormat_AV_SAMPLE_FMT_U8P | AVSampleFormat_AV_SAMPLE_FMT_S16 | AVSampleFormat_AV_SAMPLE_FMT_S16P | AVSampleFormat_AV_SAMPLE_FMT_S32 | AVSampleFormat_AV_SAMPLE_FMT_S32P) {
                let mut owned = crate::audio::Resampler::open(frame, rate, channels)?;
                if owned.owns_float_pipeline() {
                    owned.float_output();
                    built.owned = Some(owned);
                    return Ok(built);
                }
            }
            check(
                swr_alloc_set_opts2(
                    &mut swr,
                    &built.layout,
                    AVSampleFormat_AV_SAMPLE_FMT_FLT,
                    rate,
                    &source.ch_layout,
                    source.format,
                    source.sample_rate,
                    0,
                    ptr::null_mut(),
                ),
                "allocate playback resampler",
            )?;
            built.swr = swr;
            check(swr_init(built.swr), "initialize playback resampler")?;
            Ok(built)
        }
    }

    fn convert(&mut self, frame: *const AVFrame) -> Result<Vec<f32>> {
        self.convert_inner(frame)
    }

    fn flush(&mut self) -> Result<Vec<f32>> {
        self.convert_inner(ptr::null())
    }

    fn convert_inner(&mut self, frame: *const AVFrame) -> Result<Vec<f32>> {
        unsafe {
            if let Some(owned) = &mut self.owned_s64 {
                use std::io::Write;
                if frame.is_null() {
                    owned.finish().map_err(|e| e.to_string())?;
                } else {
                    let source = &*frame;
                    if source.sample_rate != self.input_rate || source.ch_layout.nb_channels != self.input_channels
                        || source.format != self.input_format || source.nb_samples < 0 || source.extended_data.is_null()
                    { return Err("S64 playback input format changed".into()); }
                    let planar = source.format == AVSampleFormat_AV_SAMPLE_FMT_S64P;
                    let mut pcm = Vec::new();
                    for sample in 0..source.nb_samples as usize {
                        for channel in 0..self.input_channels as usize {
                            let plane = *source.extended_data.add(if planar {channel} else {0});
                            if plane.is_null() { return Err("missing S64 playback plane".into()); }
                            let index = if planar {sample} else {sample * self.input_channels as usize + channel};
                            let normalized = ptr::read_unaligned(plane.cast::<i64>().add(index)) as f64 / 9223372036854775808.;
                            pcm.extend_from_slice(&normalized.to_le_bytes());
                        }
                    }
                    crate::owned_pcm_gain_f64::PcmGain::new(&mut *owned,1.0,self.input_channels as u16,self.channels as u16)?
                        .write_all(&pcm).map_err(|e| e.to_string())?;
                }
                return Ok(owned.take_output().chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32).collect());
            }
            let dst = Frame::new()?;
            if let Some(owned) = &mut self.owned {
                let frames = owned.convert(dst.0, frame)?;
                let count = usize::try_from(frames).map_err(|_| "invalid owned playback frame count")?
                    .checked_mul(self.channels as usize).ok_or("audio frame too large")?;
                if count == 0 { return Ok(Vec::new()); }
                let data = (*dst.0).data[0].cast::<f32>();
                if data.is_null() { return Err("owned playback audio has no samples".into()); }
                if (*dst.0).format == AVSampleFormat_AV_SAMPLE_FMT_DBL {
                    return Ok(std::slice::from_raw_parts((*dst.0).data[0].cast::<f64>(), count).iter().map(|v| *v as f32).collect());
                }
                return Ok(std::slice::from_raw_parts(data, count).to_vec());
            }

            let out_samples = if frame.is_null() {
                let delay = swr_get_delay(self.swr, i64::from(self.rate));
                if delay <= 0 {
                    return Ok(Vec::new());
                }
                delay
            } else {
                let source = &*frame;
                let delay = swr_get_delay(self.swr, i64::from(self.rate));
                super::rescale_capacity_owned(
                    delay.checked_add(i64::from(source.nb_samples)).ok_or("audio capacity overflow")?,
                    self.rate, source.sample_rate.max(1),
                )?
            };
            if out_samples <= 0 || out_samples > i64::from(i32::MAX) {
                return Err("resampled audio size overflow".into());
            }
            (*dst.0).format = AVSampleFormat_AV_SAMPLE_FMT_FLT;
            (*dst.0).sample_rate = self.rate;
            (*dst.0).nb_samples = out_samples as i32;
            check(
                channel_layout_copy_owned(&mut (*dst.0).ch_layout, &self.layout),
                "copy playback layout",
            )?;
            check(av_frame_get_buffer(dst.0, 0), "allocate playback audio")?;
            let code = swr_convert_frame(self.swr, dst.0, frame);
            if code < 0 {
                return Err(check(code, "resample playback audio").unwrap_err());
            }
            let produced = (*dst.0).nb_samples.max(0) as usize;
            let channels = self.channels as usize;
            let count = produced
                .checked_mul(channels)
                .ok_or("audio frame too large")?;
            let data = (*dst.0).data[0] as *const f32;
            if count == 0 {
                return Ok(Vec::new());
            }
            if data.is_null() {
                return Err("resampled audio has no samples".into());
            }
            Ok(std::slice::from_raw_parts(data, count).to_vec())
        }
    }
}
#[cfg(test)]
mod owned_playback_audio_tests {
    use super::*;
    #[test]
    fn packed_and_planar_playback_use_owned_filter_and_rematrix() {
        use std::io::Write;
        for (planar, input_channels, output_channels) in [(false, 2, 1), (true, 2, 1), (false, 6, 2), (true, 6, 2), (false, 7, 2), (true, 7, 1), (false, 8, 1), (true, 8, 2), (false, 1, 6), (true, 1, 8), (false, 2, 7), (true, 2, 8)] {
            let input = Frame::new().unwrap();
            let pcm: Vec<f32> = (0..997).flat_map(|i| [(i as f32 * 0.07).sin(), -0.25, 0.5, 1.0, 0.1, 0.2, -0.3, 0.4].into_iter().take(input_channels as usize)).collect();
            let mut reference = crate::owned_resample::Resampler::new(Vec::new(), 48000, 16000, output_channels).unwrap();
            crate::owned_pcm_gain::PcmGain::new(&mut reference, 1.0, input_channels, output_channels).unwrap()
                .write_all(&pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            reference.finish().unwrap();
            // SAFETY: RAII frame owns checked 997xN float planes.
            unsafe {
                (*input.0).format = if planar { AVSampleFormat_AV_SAMPLE_FMT_FLTP } else { AVSampleFormat_AV_SAMPLE_FMT_FLT };
                (*input.0).sample_rate = 48000;
                (*input.0).nb_samples = 997;
                channel_layout_default_owned(&mut (*input.0).ch_layout, i32::from(input_channels));
                check(av_frame_get_buffer(input.0, 0), "playback test input").unwrap();
                for sample in 0..997 {
                    for channel in 0..input_channels as usize {
                        let plane = *(*input.0).extended_data.add(if planar {channel} else {0});
                        ptr::write_unaligned(plane.cast::<f32>().add(if planar {sample} else {sample * input_channels as usize + channel}), pcm[sample * input_channels as usize + channel]);
                    }
                }
                let mut adapter = PlayResampler::open(input.0,16000,i32::from(output_channels)).unwrap();
                assert!(adapter.owned.is_some());
                assert!(adapter.swr.is_null());
                let mut output = adapter.convert(input.0).unwrap();
                output.extend(adapter.flush().unwrap());
                assert!(adapter.flush().unwrap().is_empty());
                assert_eq!(output.len(),333 * output_channels as usize);
                assert_eq!(output.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),reference.take_output());
            }
        }
    }
}

#[cfg(test)]
mod owned_double_playback_tests {
    use super::*;
    #[test]
    fn packed_and_planar_playback_use_owned_filter_and_rematrix() {
        use std::io::Write;
        for (planar, input_channels, output_channels) in [(false, 2, 1), (true, 2, 1), (false, 6, 2), (true, 6, 2), (false, 7, 2), (true, 7, 1), (false, 8, 1), (true, 8, 2), (false, 1, 6), (true, 1, 8), (false, 2, 7), (true, 2, 8)] {
            let input = Frame::new().unwrap();
            let pcm: Vec<f64> = (0..997).flat_map(|i| [(i as f64 * 0.07).sin(), -0.25, 0.5, 1.0, 0.1, 0.2, -0.3, 0.4].into_iter().take(input_channels as usize)).collect();
            let mut reference = crate::owned_resample_f64::Resampler::new(Vec::new(), 48000, 16000, output_channels).unwrap();
            crate::owned_pcm_gain_f64::PcmGain::new(&mut reference, 1.0, input_channels, output_channels).unwrap()
                .write_all(&pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()).unwrap();
            reference.finish().unwrap();
            // SAFETY: RAII frame owns checked 997xN float planes.
            unsafe {
                (*input.0).format = if planar { AVSampleFormat_AV_SAMPLE_FMT_DBLP } else { AVSampleFormat_AV_SAMPLE_FMT_DBL };
                (*input.0).sample_rate = 48000;
                (*input.0).nb_samples = 997;
                channel_layout_default_owned(&mut (*input.0).ch_layout, i32::from(input_channels));
                check(av_frame_get_buffer(input.0, 0), "playback test input").unwrap();
                for sample in 0..997 {
                    for channel in 0..input_channels as usize {
                        let plane = *(*input.0).extended_data.add(if planar {channel} else {0});
                        ptr::write_unaligned(plane.cast::<f64>().add(if planar {sample} else {sample * input_channels as usize + channel}), pcm[sample * input_channels as usize + channel]);
                    }
                }
                let mut adapter = PlayResampler::open(input.0,16000,i32::from(output_channels)).unwrap();
                assert!(adapter.owned.is_some());
                assert!(adapter.swr.is_null());
                let mut output = adapter.convert(input.0).unwrap();
                output.extend(adapter.flush().unwrap());
                assert!(adapter.flush().unwrap().is_empty());
                assert_eq!(output.len(),333 * output_channels as usize);
                let expected: Vec<f32> = reference.take_output().chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32).collect();
                assert_eq!(output,expected);
            }
        }
    }
}

#[cfg(test)]
mod owned_integer_playback_tests {
    use super::*;
    #[test]
    fn integer_playback_filters_in_double_without_intermediate_quantization() {
        use std::io::Write;
        use crate::owned_pcm_integer::Format;
        for (packed, planar_format, format) in [
            (AVSampleFormat_AV_SAMPLE_FMT_U8, AVSampleFormat_AV_SAMPLE_FMT_U8P, Format::U8),
            (AVSampleFormat_AV_SAMPLE_FMT_S16, AVSampleFormat_AV_SAMPLE_FMT_S16P, Format::I16),
            (AVSampleFormat_AV_SAMPLE_FMT_S32, AVSampleFormat_AV_SAMPLE_FMT_S32P, Format::I32),
        ] {
            for (planar,input_channels,output_channels) in [(false,2,1),(true,2,1),(false,6,2),(true,6,2),(false,7,2),(true,7,1),(false,8,1),(true,8,2),(false,1,6),(true,1,8),(false,2,7),(true,2,8)] {
                let input = Frame::new().unwrap();
                let mut normalized = Vec::new();
                // SAFETY: RAII frame owns checked 997xN integer planes.
                unsafe {
                    (*input.0).format = if planar {planar_format} else {packed};
                    (*input.0).sample_rate = 48000;
                    (*input.0).nb_samples = 997;
                    channel_layout_default_owned(&mut (*input.0).ch_layout,input_channels);
                    check(av_frame_get_buffer(input.0,0),"integer playback test input").unwrap();
                    for sample in 0..997 {
                        for channel in 0..input_channels as usize {
                            let value = [(sample as f64 * 0.07).sin() * 0.7,-0.25,0.5,1.0,0.1,0.2,-0.3,0.4][channel];
                            let mut encoded = [0u8;4];
                            format.encode(value,&mut encoded[..format.bytes()]).unwrap();
                            normalized.extend_from_slice(&format.decode(&encoded[..format.bytes()]).unwrap().to_le_bytes());
                            let plane = *(*input.0).extended_data.add(if planar {channel} else {0});
                            let index = if planar {sample} else {sample * input_channels as usize + channel};
                            match format {
                                Format::U8 => *plane.add(index) = encoded[0],
                                Format::I16 => ptr::write_unaligned(plane.cast::<i16>().add(index),i16::from_le_bytes(encoded[..2].try_into().unwrap())),
                                Format::I32 => ptr::write_unaligned(plane.cast::<i32>().add(index),i32::from_le_bytes(encoded[..4].try_into().unwrap())),
                                Format::I64 => unreachable!("S64 playback has a dedicated acceptance test"),
                            }
                        }
                    }
                    let mut reference = crate::owned_resample_f64::Resampler::new(Vec::new(),48000,16000,output_channels as u16).unwrap();
                    crate::owned_pcm_gain_f64::PcmGain::new(&mut reference,1.0,input_channels as u16,output_channels as u16).unwrap().write_all(&normalized).unwrap();
                    reference.finish().unwrap();
                    let expected: Vec<f32> = reference.take_output().chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32).collect();
                    let mut adapter = PlayResampler::open(input.0,16000,output_channels).unwrap();
                    assert!(adapter.owned.is_some());
                    assert!(adapter.swr.is_null());
                    let mut actual = adapter.convert(input.0).unwrap();
                    actual.extend(adapter.flush().unwrap());
                    assert_eq!(actual,expected);
                    assert_eq!(actual.len(),333 * output_channels as usize);
                }
            }
        }
    }
}

#[cfg(test)]
mod owned_s64_playback_tests {
    use super::*;
    #[test]
    fn s64_playback_resamples_and_rematrices_without_swresample() {
        use std::io::Write;
        let values = [i64::MIN,i64::MIN+1,-1,0,1,i64::MAX-1,i64::MAX,0x123456789abcdef];
        for (planar,input_channels,output_channels) in [(false,2,2),(true,2,2),(false,6,2),(true,6,2),(false,7,2),(true,7,1),(false,8,1),(true,8,2),(false,1,6),(true,1,8),(false,2,7),(true,2,8)] {
            let input = Frame::new().unwrap();
            let mut normalized = Vec::new();
            // SAFETY: RAII frame owns checked 997xN S64 planes.
            unsafe {
                (*input.0).format = if planar {AVSampleFormat_AV_SAMPLE_FMT_S64P} else {AVSampleFormat_AV_SAMPLE_FMT_S64};
                (*input.0).sample_rate = 48000;
                (*input.0).nb_samples = 997;
                channel_layout_default_owned(&mut (*input.0).ch_layout,input_channels);
                check(av_frame_get_buffer(input.0,0),"S64 playback test input").unwrap();
                for sample in 0..997 {
                    for channel in 0..input_channels as usize {
                        let value = values[(sample + channel) % values.len()];
                        normalized.extend_from_slice(&(value as f64 / 9223372036854775808.).to_le_bytes());
                        let plane = *(*input.0).extended_data.add(if planar {channel} else {0});
                        ptr::write_unaligned(plane.cast::<i64>().add(if planar {sample} else {sample * input_channels as usize + channel}),value);
                    }
                }
                let mut reference = crate::owned_resample_f64::Resampler::new(Vec::new(),48000,16000,output_channels as u16).unwrap();
                crate::owned_pcm_gain_f64::PcmGain::new(&mut reference,1.0,input_channels as u16,output_channels as u16).unwrap().write_all(&normalized).unwrap();
                reference.finish().unwrap();
                let expected: Vec<f32> = reference.take_output().chunks_exact(8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32).collect();
                let mut adapter = PlayResampler::open(input.0,16000,output_channels).unwrap();
                assert!(adapter.owned_s64.is_some());
                assert!(adapter.swr.is_null());
                let mut actual = adapter.convert(input.0).unwrap();
                actual.extend(adapter.flush().unwrap());
                assert_eq!(actual,expected);
                assert_eq!(actual.len(),333 * output_channels as usize);
                assert!(adapter.flush().unwrap().is_empty());
            }
        }
    }
}

}

#[cfg(feature = "player")]
pub use runtime::{play, play_paths};

#[cfg(test)]
mod tests {
    use super::{
        advance_rate_phase, clamp_rate_milli, encode_bmp, fit_display_size, pack_bgra,
        playback_seek_ts, playlist_step, scale_elapsed_us, snapshot_path, take_aligned,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn display_size_keeps_small_frames_and_caps_4k() {
        assert_eq!(fit_display_size(320, 240).unwrap(), (320, 240));
        assert_eq!(fit_display_size(1919, 1080).unwrap(), (1919, 1080));
        assert_eq!(fit_display_size(3840, 2160).unwrap(), (1920, 1080));
        assert_eq!(fit_display_size(1000, 2000).unwrap(), (540, 1080));
        assert!(fit_display_size(0, 10).is_err());
    }

    #[test]
    fn packs_bgra_rows_with_stride() {
        let src = [1u8, 2, 3, 255, 0, 0, 0, 0, 4, 5, 6, 128, 0, 0, 0, 0];
        let out = pack_bgra(&src, 8, 1, 2).unwrap();
        assert_eq!(out, vec![0x0003_0201, 0x0006_0504]);
    }

    #[test]
    fn aligns_audio_to_video_origin() {
        let mut aligned = false;
        let early = take_aligned(&mut aligned, 500_000, 2, 2, 0, &[1.0, 1.0, 2.0, 2.0]);
        assert_eq!(early, vec![2.0, 2.0]);
        assert!(aligned);
        let later = take_aligned(&mut aligned, 500_000, 2, 2, 1_000_000, &[3.0, 3.0]);
        assert_eq!(later, vec![3.0, 3.0]);

        let mut aligned = false;
        let lead = take_aligned(&mut aligned, 0, 4, 1, 500_000, &[0.5, 0.25]);
        assert_eq!(lead, vec![0.0, 0.0, 0.5, 0.25]);
        assert!(aligned);
    }

    #[test]
    fn seek_timestamp_is_origin_plus_media_time() {
        assert_eq!(playback_seek_ts(0, 1_500_000), 1_500_000);
        assert_eq!(playback_seek_ts(250_000, 1_000), 251_000);
        assert_eq!(playback_seek_ts(10, -5), 10);
    }

    #[test]
    fn rate_scales_time_and_audio_phase() {
        assert_eq!(clamp_rate_milli(1.0), 1_000);
        assert_eq!(clamp_rate_milli(0.1), 250);
        assert_eq!(clamp_rate_milli(8.0), 4_000);
        assert_eq!(clamp_rate_milli(f32::NAN), 1_000);
        assert_eq!(scale_elapsed_us(1_000_000, 1_000), 1_000_000);
        assert_eq!(scale_elapsed_us(1_000_000, 2_000), 2_000_000);
        assert_eq!(scale_elapsed_us(1_000_000, 250), 250_000);
        assert_eq!(advance_rate_phase(0, 1_000), (1, 0));
        assert_eq!(advance_rate_phase(0, 2_000), (2, 0));
        let mut phase = 0;
        let mut consumed = 0u32;
        for _ in 0..4 {
            let (need, next) = advance_rate_phase(phase, 250);
            consumed += need;
            phase = next;
        }
        assert_eq!(consumed, 1);
        assert_eq!(phase, 0);
    }

    #[test]
    fn playlist_step_stays_inside_the_list() {
        assert_eq!(playlist_step(3, 0, -1), None);
        assert_eq!(playlist_step(3, 0, 1), Some(1));
        assert_eq!(playlist_step(3, 2, 1), None);
        assert_eq!(playlist_step(0, 0, 1), None);
    }

    #[test]
    fn snapshot_bmp_is_bottom_up_bgr() {
        let bytes = encode_bmp(1, 1, &[0x00FF_0000]).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(bytes[54], 0);
        assert_eq!(bytes[55], 0);
        assert_eq!(bytes[56], 255);
        let path = snapshot_path(Path::new("clips/demo.mp4"), 2);
        assert_eq!(path, PathBuf::from("clips/demo-fvid-2.bmp"));
    }
}
