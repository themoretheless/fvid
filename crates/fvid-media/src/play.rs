//! Local window playback. Software decode to a display-sized BGRA buffer, optional
//! device audio, and an egui window. Not a streaming server and not a hardware presenter.
use super::lossless::{Codec, Frame};
use super::*;
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use eframe::egui;
use serde::Serialize;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const MAX_W: i32 = 1920;
const MAX_H: i32 = 1080;
const VIDEO_QUEUE: usize = 4;
const AUDIO_SECONDS: usize = 1;
const SLACK_US: i64 = 20_000;
const SWS_BILINEAR: i32 = 2;
const RATE_MIN_MILLI: u32 = 250;
const RATE_MAX_MILLI: u32 = 4_000;
/// VLC's Qt slider goes to 200%. 1000 is unity.
pub const VOLUME_MAX_MILLI: u32 = 2_000;

pub fn clamp_volume_milli(value: i32) -> u32 {
    value.clamp(0, VOLUME_MAX_MILLI as i32) as u32
}

/// Video adjustment. `1000` is neutral. Range is `0..=2000`.
pub fn clamp_adjust_milli(value: i32) -> i32 {
    value.clamp(0, 2_000)
}

/// Apply VLC-style brightness, contrast, saturation and hue. `1000` is identity for each.
/// Hue maps `0..=2000` onto `-180°..=180°`.
pub fn adjust_pixel(
    red: u8,
    green: u8,
    blue: u8,
    brightness_milli: i32,
    contrast_milli: i32,
    saturation_milli: i32,
    hue_milli: i32,
) -> (u8, u8, u8) {
    let brightness = (clamp_adjust_milli(brightness_milli) - 1_000) as f32 / 1_000.0;
    let contrast = clamp_adjust_milli(contrast_milli) as f32 / 1_000.0;
    let saturation = clamp_adjust_milli(saturation_milli) as f32 / 1_000.0;
    let hue = (clamp_adjust_milli(hue_milli) - 1_000) as f32 / 1_000.0 * std::f32::consts::PI;
    let mut red = red as f32 / 255.0;
    let mut green = green as f32 / 255.0;
    let mut blue = blue as f32 / 255.0;
    red = (red - 0.5) * contrast + 0.5;
    green = (green - 0.5) * contrast + 0.5;
    blue = (blue - 0.5) * contrast + 0.5;
    red += brightness * 0.5;
    green += brightness * 0.5;
    blue += brightness * 0.5;
    let gray = 0.299 * red + 0.587 * green + 0.114 * blue;
    red = gray + (red - gray) * saturation;
    green = gray + (green - gray) * saturation;
    blue = gray + (blue - gray) * saturation;
    if hue.abs() > f32::EPSILON {
        let cos = hue.cos();
        let sin = hue.sin();
        let matrix = [
            [
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
            ],
            [
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
            ],
            [
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ],
        ];
        let (r, g, b) = (red, green, blue);
        red = matrix[0][0] * r + matrix[0][1] * g + matrix[0][2] * b;
        green = matrix[1][0] * r + matrix[1][1] * g + matrix[1][2] * b;
        blue = matrix[2][0] * r + matrix[2][1] * g + matrix[2][2] * b;
    }
    (
        (red * 255.0).round().clamp(0.0, 255.0) as u8,
        (green * 255.0).round().clamp(0.0, 255.0) as u8,
        (blue * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Apply display gamma. `1000` is identity; range matches [`clamp_adjust_milli`].
pub fn gamma_channel(value: u8, gamma_milli: i32) -> u8 {
    let gamma = clamp_adjust_milli(gamma_milli) as f32 / 1_000.0;
    if (gamma - 1.0).abs() < f32::EPSILON {
        return value;
    }
    let sample = value as f32 / 255.0;
    let out = sample.powf(1.0 / gamma.max(0.01));
    (out * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn apply_gamma_pixel(red: u8, green: u8, blue: u8, gamma_milli: i32) -> (u8, u8, u8) {
    (
        gamma_channel(red, gamma_milli),
        gamma_channel(green, gamma_milli),
        gamma_channel(blue, gamma_milli),
    )
}

/// Flip UV corners. `uv` is `(u0, v0, u1, v1)`.
pub fn flip_uv(uv: (f32, f32, f32, f32), flip_h: bool, flip_v: bool) -> (f32, f32, f32, f32) {
    let (mut u0, mut v0, mut u1, mut v1) = uv;
    if flip_h {
        std::mem::swap(&mut u0, &mut u1);
    }
    if flip_v {
        std::mem::swap(&mut v0, &mut v1);
    }
    (u0, v0, u1, v1)
}

/// Quarter turns clockwise. Display size swaps width/height for odd turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RotateMode {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

pub fn cycle_rotate(mode: RotateMode) -> RotateMode {
    match mode {
        RotateMode::Deg0 => RotateMode::Deg90,
        RotateMode::Deg90 => RotateMode::Deg180,
        RotateMode::Deg180 => RotateMode::Deg270,
        RotateMode::Deg270 => RotateMode::Deg0,
    }
}

pub fn rotate_label(mode: RotateMode) -> &'static str {
    match mode {
        RotateMode::Deg0 => "0°",
        RotateMode::Deg90 => "90°",
        RotateMode::Deg180 => "180°",
        RotateMode::Deg270 => "270°",
    }
}

pub fn rotate_turns(mode: RotateMode) -> u32 {
    match mode {
        RotateMode::Deg0 => 0,
        RotateMode::Deg90 => 1,
        RotateMode::Deg180 => 2,
        RotateMode::Deg270 => 3,
    }
}

/// Map source `(x, y)` into the rotated frame. Odd turns swap width and height.
pub fn rotate_pixel(x: u32, y: u32, width: u32, height: u32, mode: RotateMode) -> (u32, u32) {
    let last_x = width.saturating_sub(1);
    let last_y = height.saturating_sub(1);
    match mode {
        RotateMode::Deg0 => (x, y),
        RotateMode::Deg90 => (y, last_x.saturating_sub(x)),
        RotateMode::Deg180 => (last_x.saturating_sub(x), last_y.saturating_sub(y)),
        RotateMode::Deg270 => (last_y.saturating_sub(y), x),
    }
}

pub fn rotate_size(width: u32, height: u32, mode: RotateMode) -> (u32, u32) {
    match mode {
        RotateMode::Deg0 | RotateMode::Deg180 => (width, height),
        RotateMode::Deg90 | RotateMode::Deg270 => (height, width),
    }
}

/// One-pole state for a 3-band tone control.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToneState {
    pub low: f32,
    pub high: f32,
}

/// Split `sample` into bass / mid / treble, apply gains (`1000` = unity), and sum.
pub fn tone_step(
    sample: f32,
    state: &mut ToneState,
    bass_milli: i32,
    mid_milli: i32,
    treble_milli: i32,
) -> f32 {
    state.low += 0.08 * (sample - state.low);
    state.high += 0.02 * (sample - state.high);
    let low = state.low;
    let high = sample - state.high;
    let mid = sample - low - high;
    let bass = clamp_adjust_milli(bass_milli) as f32 / 1_000.0;
    let mid_gain = clamp_adjust_milli(mid_milli) as f32 / 1_000.0;
    let treble = clamp_adjust_milli(treble_milli) as f32 / 1_000.0;
    (low * bass + mid * mid_gain + high * treble).clamp(-1.0, 1.0)
}

/// Neutral Bass/Mid/Treble gain (`1000` = unity).
pub const TONE_UNITY_MILLI: i32 = 1_000;
pub const TONE_STEP_MILLI: i32 = 100;

pub fn tone_gain_step_milli(current: i32, delta: i32) -> i32 {
    clamp_adjust_milli(current + delta)
}

pub fn reset_tone_gains() -> (i32, i32, i32) {
    (TONE_UNITY_MILLI, TONE_UNITY_MILLI, TONE_UNITY_MILLI)
}

/// Apply Bass/Mid/Treble to one interleaved frame. `states.len()` should match channel count.
pub fn apply_tone_frame(
    frame: &mut [f32],
    states: &mut [ToneState],
    bass_milli: i32,
    mid_milli: i32,
    treble_milli: i32,
) {
    let n = frame.len().min(states.len());
    for i in 0..n {
        frame[i] = tone_step(frame[i], &mut states[i], bass_milli, mid_milli, treble_milli);
    }
}

pub fn format_tone_osd(bass_milli: i32, mid_milli: i32, treble_milli: i32) -> String {
    format!(
        "Tone B{} M{} T{}",
        clamp_adjust_milli(bass_milli) / 10,
        clamp_adjust_milli(mid_milli) / 10,
        clamp_adjust_milli(treble_milli) / 10
    )
}

/// VLC-style 10-band graphic equalizer.
pub const EQ_BAND_COUNT: usize = 10;

/// Center frequencies (Hz) matching the classic VLC 10-band preset labels.
pub const EQ_BAND_HZ: [u32; EQ_BAND_COUNT] =
    [60, 170, 310, 600, 1_000, 3_000, 6_000, 12_000, 14_000, 16_000];

/// Cascaded one-pole low-pass state for [`graphic_eq_step`].
#[derive(Clone, Copy, Debug)]
pub struct GraphicEqState {
    pub lp: [f32; EQ_BAND_COUNT - 1],
}

impl Default for GraphicEqState {
    fn default() -> Self {
        Self {
            lp: [0.0; EQ_BAND_COUNT - 1],
        }
    }
}

/// One-pole alphas at ~48 kHz, increasing cutoff across the 9 split stages.
const EQ_LP_ALPHA: [f32; EQ_BAND_COUNT - 1] =
    [0.004, 0.011, 0.020, 0.038, 0.062, 0.17, 0.30, 0.48, 0.55];

/// Unity gains for all graphic-EQ bands (`1000` = 0 dB).
pub fn eq_unity_gains() -> [i32; EQ_BAND_COUNT] {
    [1_000; EQ_BAND_COUNT]
}

/// EQ band gain range. `1000` is 0 dB; spans roughly −26 dB..=+12 dB.
pub fn clamp_eq_milli(value: i32) -> i32 {
    value.clamp(50, 4_000)
}

/// Convert a VLC-style band gain in dB to linear milli (`1000` = unity).
pub fn eq_db_to_milli(db: f32) -> i32 {
    let linear = 10f32.powf(db / 20.0);
    clamp_eq_milli((linear * 1_000.0).round() as i32)
}

/// Classic VLC 10-band equalizer presets (`equalizer_presets.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EqPreset {
    #[default]
    Flat,
    Classical,
    Club,
    Dance,
    FullBass,
    FullBassTreble,
    FullTreble,
    Headphones,
    LargeHall,
    Live,
    Party,
    Pop,
    Reggae,
    Rock,
    Ska,
    Soft,
    SoftRock,
    Techno,
}

pub const EQ_PRESET_COUNT: usize = 18;

pub fn eq_preset_label(preset: EqPreset) -> &'static str {
    match preset {
        EqPreset::Flat => "Flat",
        EqPreset::Classical => "Classical",
        EqPreset::Club => "Club",
        EqPreset::Dance => "Dance",
        EqPreset::FullBass => "Full bass",
        EqPreset::FullBassTreble => "Full bass+treble",
        EqPreset::FullTreble => "Full treble",
        EqPreset::Headphones => "Headphones",
        EqPreset::LargeHall => "Large hall",
        EqPreset::Live => "Live",
        EqPreset::Party => "Party",
        EqPreset::Pop => "Pop",
        EqPreset::Reggae => "Reggae",
        EqPreset::Rock => "Rock",
        EqPreset::Ska => "Ska",
        EqPreset::Soft => "Soft",
        EqPreset::SoftRock => "Soft rock",
        EqPreset::Techno => "Techno",
    }
}

pub fn cycle_eq_preset(preset: EqPreset) -> EqPreset {
    match preset {
        EqPreset::Flat => EqPreset::Classical,
        EqPreset::Classical => EqPreset::Club,
        EqPreset::Club => EqPreset::Dance,
        EqPreset::Dance => EqPreset::FullBass,
        EqPreset::FullBass => EqPreset::FullBassTreble,
        EqPreset::FullBassTreble => EqPreset::FullTreble,
        EqPreset::FullTreble => EqPreset::Headphones,
        EqPreset::Headphones => EqPreset::LargeHall,
        EqPreset::LargeHall => EqPreset::Live,
        EqPreset::Live => EqPreset::Party,
        EqPreset::Party => EqPreset::Pop,
        EqPreset::Pop => EqPreset::Reggae,
        EqPreset::Reggae => EqPreset::Rock,
        EqPreset::Rock => EqPreset::Ska,
        EqPreset::Ska => EqPreset::Soft,
        EqPreset::Soft => EqPreset::SoftRock,
        EqPreset::SoftRock => EqPreset::Techno,
        EqPreset::Techno => EqPreset::Flat,
    }
}

/// Band gains in dB from VLC `eqz_preset_10b` (near-zero epsilon treated as 0).
pub fn eq_preset_db(preset: EqPreset) -> [f32; EQ_BAND_COUNT] {
    match preset {
        EqPreset::Flat => [0.0; EQ_BAND_COUNT],
        EqPreset::Classical => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -7.2, -7.2, -7.2, -9.6],
        EqPreset::Club => [0.0, 0.0, 8.0, 5.6, 5.6, 5.6, 3.2, 0.0, 0.0, 0.0],
        EqPreset::Dance => [9.6, 7.2, 2.4, 0.0, 0.0, -5.6, -7.2, -7.2, 0.0, 0.0],
        EqPreset::FullBass => [-8.0, 9.6, 9.6, 5.6, 1.6, -4.0, -8.0, -10.4, -11.2, -11.2],
        EqPreset::FullBassTreble => [7.2, 5.6, 0.0, -7.2, -4.8, 1.6, 8.0, 11.2, 12.0, 12.0],
        EqPreset::FullTreble => [-9.6, -9.6, -9.6, -4.0, 2.4, 11.2, 16.0, 16.0, 16.0, 16.8],
        EqPreset::Headphones => [4.8, 11.2, 5.6, -3.2, -2.4, 1.6, 4.8, 9.6, 12.8, 14.4],
        EqPreset::LargeHall => [10.4, 10.4, 5.6, 5.6, 0.0, -4.8, -4.8, -4.8, 0.0, 0.0],
        EqPreset::Live => [-4.8, 0.0, 4.0, 5.6, 5.6, 5.6, 4.0, 2.4, 2.4, 2.4],
        EqPreset::Party => [7.2, 7.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7.2, 7.2],
        EqPreset::Pop => [-1.6, 4.8, 7.2, 8.0, 5.6, 0.0, -2.4, -2.4, -1.6, -1.6],
        EqPreset::Reggae => [0.0, 0.0, 0.0, -5.6, 0.0, 6.4, 6.4, 0.0, 0.0, 0.0],
        EqPreset::Rock => [8.0, 4.8, -5.6, -8.0, -3.2, 4.0, 8.8, 11.2, 11.2, 11.2],
        EqPreset::Ska => [-2.4, -4.8, -4.0, 0.0, 4.0, 5.6, 8.8, 9.6, 11.2, 9.6],
        EqPreset::Soft => [4.8, 1.6, 0.0, -2.4, 0.0, 4.0, 8.0, 9.6, 11.2, 12.0],
        EqPreset::SoftRock => [4.0, 4.0, 2.4, 0.0, -4.0, -5.6, -3.2, 0.0, 2.4, 8.8],
        EqPreset::Techno => [8.0, 5.6, 0.0, -5.6, -4.8, 0.0, 8.0, 9.6, 9.6, 8.8],
    }
}

/// Linear milli gains for a VLC preset.
pub fn eq_preset_gains(preset: EqPreset) -> [i32; EQ_BAND_COUNT] {
    let db = eq_preset_db(preset);
    std::array::from_fn(|i| eq_db_to_milli(db[i]))
}

/// Split `sample` into 10 complementary bands, apply per-band gains (`1000` = unity), and sum.
pub fn graphic_eq_step(
    sample: f32,
    state: &mut GraphicEqState,
    gains_milli: &[i32; EQ_BAND_COUNT],
) -> f32 {
    for i in 0..EQ_BAND_COUNT - 1 {
        state.lp[i] += EQ_LP_ALPHA[i] * (sample - state.lp[i]);
    }
    let mut out = 0.0f32;
    let mut lower = 0.0f32;
    for i in 0..EQ_BAND_COUNT - 1 {
        let band = state.lp[i] - lower;
        lower = state.lp[i];
        let gain = clamp_eq_milli(gains_milli[i]) as f32 / 1_000.0;
        out += band * gain;
    }
    let top = sample - lower;
    let top_gain = clamp_eq_milli(gains_milli[EQ_BAND_COUNT - 1]) as f32 / 1_000.0;
    (out + top * top_gain).clamp(-1.0, 1.0)
}

/// VLC-style stereo output channel modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AudioChannelMode {
    #[default]
    Stereo,
    Left,
    Right,
    Mono,
    Reverse,
}

pub fn cycle_audio_channel(mode: AudioChannelMode) -> AudioChannelMode {
    match mode {
        AudioChannelMode::Stereo => AudioChannelMode::Left,
        AudioChannelMode::Left => AudioChannelMode::Right,
        AudioChannelMode::Right => AudioChannelMode::Mono,
        AudioChannelMode::Mono => AudioChannelMode::Reverse,
        AudioChannelMode::Reverse => AudioChannelMode::Stereo,
    }
}

pub fn audio_channel_label(mode: AudioChannelMode) -> &'static str {
    match mode {
        AudioChannelMode::Stereo => "Stereo",
        AudioChannelMode::Left => "Left",
        AudioChannelMode::Right => "Right",
        AudioChannelMode::Mono => "Mono",
        AudioChannelMode::Reverse => "Reverse",
    }
}

pub fn format_audio_channel_osd(mode: AudioChannelMode) -> String {
    format!("Audio {}", audio_channel_label(mode))
}

/// Remap one interleaved frame in-place. No-op when `channels < 2`.
pub fn apply_audio_channel(frame: &mut [f32], mode: AudioChannelMode) {
    if frame.len() < 2 {
        return;
    }
    let left = frame[0];
    let right = frame[1];
    match mode {
        AudioChannelMode::Stereo => {}
        AudioChannelMode::Left => {
            frame[1] = left;
        }
        AudioChannelMode::Right => {
            frame[0] = right;
        }
        AudioChannelMode::Mono => {
            let mid = (left + right) * 0.5;
            frame[0] = mid;
            frame[1] = mid;
        }
        AudioChannelMode::Reverse => {
            frame[0] = right;
            frame[1] = left;
        }
    }
}

/// Stereo balance: `1000` is center, `0` full left, `2000` full right.
pub const BALANCE_CENTER_MILLI: i32 = 1_000;
pub const BALANCE_MIN_MILLI: i32 = 0;
pub const BALANCE_MAX_MILLI: i32 = 2_000;
pub const BALANCE_STEP_MILLI: i32 = 100;

pub fn clamp_balance_milli(value: i32) -> i32 {
    value.clamp(BALANCE_MIN_MILLI, BALANCE_MAX_MILLI)
}

pub fn balance_step_milli(current: i32, delta: i32) -> i32 {
    clamp_balance_milli(current + delta)
}

/// Scale L/R gains for stereo balance. No-op when `frame.len() < 2`.
pub fn apply_audio_balance(frame: &mut [f32], balance_milli: i32) {
    if frame.len() < 2 {
        return;
    }
    let balance = clamp_balance_milli(balance_milli);
    let left_gain = if balance <= BALANCE_CENTER_MILLI {
        1.0
    } else {
        (BALANCE_MAX_MILLI - balance) as f32 / 1_000.0
    };
    let right_gain = if balance >= BALANCE_CENTER_MILLI {
        1.0
    } else {
        balance as f32 / 1_000.0
    };
    frame[0] *= left_gain;
    frame[1] *= right_gain;
}

pub fn format_balance_osd(balance_milli: i32) -> String {
    let balance = clamp_balance_milli(balance_milli);
    if balance == BALANCE_CENTER_MILLI {
        "Balance center".into()
    } else if balance < BALANCE_CENTER_MILLI {
        format!("Balance L{}", BALANCE_CENTER_MILLI - balance)
    } else {
        format!("Balance R{}", balance - BALANCE_CENTER_MILLI)
    }
}

/// Clamp subtitle bottom margin in pixels (`0..=400`). Higher lifts text toward the top.
pub fn clamp_subtitle_margin(px: i32) -> i32 {
    px.clamp(0, 400)
}

/// Bottom margin for painted subtitles given a base inset and user offset.
pub fn subtitle_margin_px(base: i32, offset: i32) -> i32 {
    clamp_subtitle_margin(base.saturating_add(offset))
}

/// Subtitle text scale. `1000` is default size.
pub const SUBTITLE_SCALE_MIN_MILLI: i32 = 500;
pub const SUBTITLE_SCALE_MAX_MILLI: i32 = 2_000;
pub const SUBTITLE_SCALE_UNITY_MILLI: i32 = 1_000;
pub const SUBTITLE_SCALE_STEP_MILLI: i32 = 100;

pub fn clamp_subtitle_scale_milli(value: i32) -> i32 {
    value.clamp(SUBTITLE_SCALE_MIN_MILLI, SUBTITLE_SCALE_MAX_MILLI)
}

pub fn subtitle_scale_step_milli(current: i32, delta: i32) -> i32 {
    clamp_subtitle_scale_milli(current + delta)
}

/// Scaled font size in pixels for on-screen subtitles.
pub fn subtitle_font_px(base_px: f32, scale_milli: i32) -> f32 {
    base_px * (clamp_subtitle_scale_milli(scale_milli) as f32 / 1_000.0)
}

pub fn cycle_eq_bypass(bypassed: bool) -> bool {
    !bypassed
}

pub fn format_eq_bypass_osd(bypassed: bool) -> &'static str {
    if bypassed {
        "EQ off"
    } else {
        "EQ on"
    }
}

/// Average two packed `0x00RRGGBB` pixels channel-wise.
pub fn average_rgb_pixel(a: u32, b: u32) -> u32 {
    let ar = (a >> 16) & 0xff;
    let ag = (a >> 8) & 0xff;
    let ab = a & 0xff;
    let br = (b >> 16) & 0xff;
    let bg = (b >> 8) & 0xff;
    let bb = b & 0xff;
    (((ar + br) / 2) << 16) | (((ag + bg) / 2) << 8) | ((ab + bb) / 2)
}

/// VLC-style blend deinterlace on packed `0x00RRGGBB` rows: each field pair becomes their average.
pub fn deinterlace_blend_rgb(pixels: &mut [u32], width: u32, height: u32) {
    if width == 0 || height < 2 {
        return;
    }
    let w = width as usize;
    let h = height as usize;
    if pixels.len() < w * h {
        return;
    }
    for y in (0..h.saturating_sub(1)).step_by(2) {
        let top = y * w;
        let bot = (y + 1) * w;
        for x in 0..w {
            let avg = average_rgb_pixel(pixels[top + x], pixels[bot + x]);
            pixels[top + x] = avg;
            pixels[bot + x] = avg;
        }
    }
}

/// Bob deinterlace: copy even rows onto the following odd rows (VLC "Bob" style).
pub fn deinterlace_bob_rgb(pixels: &mut [u32], width: u32, height: u32) {
    if width == 0 || height < 2 {
        return;
    }
    let w = width as usize;
    let h = height as usize;
    if pixels.len() < w * h {
        return;
    }
    for y in (0..h.saturating_sub(1)).step_by(2) {
        let top = y * w;
        let bot = (y + 1) * w;
        pixels.copy_within(top..top + w, bot);
    }
}

/// Playback deinterlace modes matching common VLC options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeinterlaceMode {
    #[default]
    Off,
    Blend,
    Bob,
}

pub fn cycle_deinterlace(mode: DeinterlaceMode) -> DeinterlaceMode {
    match mode {
        DeinterlaceMode::Off => DeinterlaceMode::Blend,
        DeinterlaceMode::Blend => DeinterlaceMode::Bob,
        DeinterlaceMode::Bob => DeinterlaceMode::Off,
    }
}

pub fn deinterlace_label(mode: DeinterlaceMode) -> &'static str {
    match mode {
        DeinterlaceMode::Off => "Deint",
        DeinterlaceMode::Blend => "Blend",
        DeinterlaceMode::Bob => "Bob",
    }
}

pub fn apply_deinterlace_rgb(pixels: &mut [u32], width: u32, height: u32, mode: DeinterlaceMode) {
    match mode {
        DeinterlaceMode::Off => {}
        DeinterlaceMode::Blend => deinterlace_blend_rgb(pixels, width, height),
        DeinterlaceMode::Bob => deinterlace_bob_rgb(pixels, width, height),
    }
}

/// Short OSD line for volume changes (VLC-style feedback).
pub fn format_volume_osd(volume_milli: u32, muted: bool) -> String {
    if muted {
        "Volume muted".into()
    } else {
        format!("Volume {}%", volume_milli / 10)
    }
}

/// Default volume nudge (±5%).
pub const VOLUME_STEP_MILLI: i32 = 50;
/// Default rate nudge (±0.1×).
pub const RATE_STEP_MILLI: i32 = 100;

pub fn volume_step_milli(current: u32, delta: i32) -> u32 {
    clamp_volume_milli(current as i32 + delta)
}

pub fn rate_step_milli(current: u32, delta: i32) -> u32 {
    let next = current as i32 + delta;
    next.clamp(RATE_MIN_MILLI as i32, RATE_MAX_MILLI as i32) as u32
}

/// Human clock for OSD (`mm:ss` or `h:mm:ss`).
pub fn format_play_clock(us: i64) -> String {
    format_clock(us)
}

/// Rate line for OSD (`1.50x`).
pub fn format_rate_osd(rate_milli: u32) -> String {
    format_rate(rate_milli)
}

/// VLC digit jump: `1`→10% … `9`→90%, `0`→100% of duration.
pub fn position_us_from_digit(digit: u8, duration_us: i64) -> Option<i64> {
    if duration_us <= 0 || digit > 9 {
        return None;
    }
    let percent = if digit == 0 { 100i64 } else { i64::from(digit) * 10 };
    Some(duration_us.saturating_mul(percent) / 100)
}

pub fn media_fraction(now_us: i64, duration_us: i64) -> f32 {
    if duration_us <= 0 {
        0.0
    } else {
        ((now_us.max(0) as f64) / (duration_us as f64)).clamp(0.0, 1.0) as f32
    }
}

pub fn media_us_from_fraction(fraction: f32, duration_us: i64) -> i64 {
    if duration_us <= 0 {
        return 0;
    }
    let fraction = fraction.clamp(0.0, 1.0) as f64;
    (duration_us as f64 * fraction).round() as i64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStep {
    Forward,
    Backward,
}

pub fn frame_step_target_us(now_us: i64, frame_duration_us: i64, step: FrameStep) -> i64 {
    let duration = frame_duration_us.max(1);
    match step {
        FrameStep::Forward => now_us.saturating_add(duration),
        FrameStep::Backward => now_us.saturating_sub(duration).max(0),
    }
}

/// VLC-style coarse jump (±10 s).
pub const SEEK_COARSE_US: i64 = 10_000_000;
/// Fine jump (±3 s), typically Shift+arrows.
pub const SEEK_FINE_US: i64 = 3_000_000;

pub fn seek_step_us(fine: bool) -> i64 {
    if fine {
        SEEK_FINE_US
    } else {
        SEEK_COARSE_US
    }
}

/// Prefer container metadata title; otherwise the file stem / URL leaf.
pub fn media_display_title(path: &Path, metadata_title: Option<&str>) -> String {
    if let Some(title) = metadata_title.map(str::trim).filter(|title| !title.is_empty()) {
        return title.to_string();
    }
    if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("video")
        .to_string()
}

/// Display pipeline options for [`render_play_pixels`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayRenderOptions {
    pub brightness_milli: i32,
    pub contrast_milli: i32,
    pub saturation_milli: i32,
    pub hue_milli: i32,
    pub gamma_milli: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub rotate: RotateMode,
    pub deinterlace: DeinterlaceMode,
}

impl Default for PlayRenderOptions {
    fn default() -> Self {
        Self {
            brightness_milli: 1_000,
            contrast_milli: 1_000,
            saturation_milli: 1_000,
            hue_milli: 1_000,
            gamma_milli: 1_000,
            flip_h: false,
            flip_v: false,
            rotate: RotateMode::Deg0,
            deinterlace: DeinterlaceMode::Off,
        }
    }
}

/// Apply deinterlace, optional bitmap overlay, color adjust, and rotate.
/// Returns `(width, height, packed 0x00RRGGBB pixels)`.
pub fn render_play_pixels(
    width: u32,
    height: u32,
    pixels: &[u32],
    opts: &PlayRenderOptions,
    bitmap: Option<&BitmapSubtitle>,
) -> (u32, u32, Vec<u32>) {
    let w = width as usize;
    let h = height as usize;
    let mut source = pixels.to_vec();
    if source.len() < w.saturating_mul(h) {
        source.resize(w.saturating_mul(h), 0);
    }
    apply_deinterlace_rgb(&mut source, width, height, opts.deinterlace);
    if let Some(plane) = bitmap {
        blit_bitmap_subtitle(&mut source, width, height, plane);
    }
    let (out_w, out_h) = rotate_size(width, height, opts.rotate);
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for y in 0..h {
        for x in 0..w {
            let src_x = if opts.flip_h { w - 1 - x } else { x };
            let src_y = if opts.flip_v { h - 1 - y } else { y };
            let pixel = source[src_y * w + src_x];
            let (red, green, blue) = adjust_pixel(
                ((pixel >> 16) & 0xff) as u8,
                ((pixel >> 8) & 0xff) as u8,
                (pixel & 0xff) as u8,
                opts.brightness_milli,
                opts.contrast_milli,
                opts.saturation_milli,
                opts.hue_milli,
            );
            let (red, green, blue) = apply_gamma_pixel(red, green, blue, opts.gamma_milli);
            let (dx, dy) = rotate_pixel(x as u32, y as u32, width, height, opts.rotate);
            out[dy as usize * out_w as usize + dx as usize] =
                (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        }
    }
    (out_w, out_h, out)
}

pub fn reset_video_adjust() -> (i32, i32, i32, i32, i32) {
    (1_000, 1_000, 1_000, 1_000, 1_000)
}

pub fn adjust_step_milli(current: i32, delta: i32) -> i32 {
    clamp_adjust_milli(current + delta)
}

pub fn reset_zoom_pan() -> (u32, i32, i32) {
    (1_000, 0, 0)
}

pub fn format_adjust_osd(
    brightness_milli: i32,
    contrast_milli: i32,
    saturation_milli: i32,
    hue_milli: i32,
    gamma_milli: i32,
) -> String {
    format!(
        "Adjust B{} C{} S{} H{} G{}",
        clamp_adjust_milli(brightness_milli) / 10,
        clamp_adjust_milli(contrast_milli) / 10,
        clamp_adjust_milli(saturation_milli) / 10,
        clamp_adjust_milli(hue_milli) / 10,
        clamp_adjust_milli(gamma_milli) / 10
    )
}

pub fn format_zoom_osd(zoom_milli: u32) -> String {
    format!("zoom {}", zoom_label(zoom_milli))
}

pub fn format_aspect_osd(mode: AspectMode) -> String {
    format!("aspect {}", aspect_label(mode))
}

pub fn format_crop_osd(mode: AspectMode) -> String {
    format!("crop {}", aspect_label(mode))
}

pub fn format_deinterlace_osd(mode: DeinterlaceMode) -> String {
    format!("Deinterlace {}", deinterlace_label(mode))
}

pub fn eq_band_step_milli(current: i32, delta: i32) -> i32 {
    clamp_eq_milli(current + delta)
}

pub fn set_eq_gains_from_preset(preset: EqPreset) -> [i32; EQ_BAND_COUNT] {
    eq_preset_gains(preset)
}

pub fn format_eq_preset_osd(preset: EqPreset) -> String {
    format!("EQ {}", eq_preset_label(preset))
}

/// Arrow-key pan step when zoomed past 1×.
pub const PAN_STEP_PX: i32 = 40;

pub fn format_pan_osd(pan_x_px: i32, pan_y_px: i32) -> String {
    format!("Pan {pan_x_px},{pan_y_px}")
}

/// Parse a jump clock or leave raw microseconds; clamp into duration when known.
pub fn initial_seek_us(spec: &str, duration_us: i64) -> Option<i64> {
    let target = parse_play_clock(spec)?;
    Some(clamp_seek_us(target, duration_us))
}

pub fn initial_stop_us(spec: &str, duration_us: i64) -> Option<i64> {
    initial_seek_us(spec, duration_us)
}

pub fn should_stop_playback(now_us: i64, stop_us: Option<i64>) -> bool {
    match stop_us {
        Some(stop) if stop >= 0 => now_us >= stop,
        _ => false,
    }
}

/// Finer rate step for Ctrl+mouse wheel (±0.05×).
pub const RATE_WHEEL_STEP_MILLI: i32 = 50;

pub fn rate_from_wheel(current_milli: u32, scroll_lines: i32) -> u32 {
    if scroll_lines == 0 {
        return rate_step_milli(current_milli, 0);
    }
    rate_step_milli(
        current_milli,
        scroll_lines.saturating_mul(RATE_WHEEL_STEP_MILLI),
    )
}

pub fn stop_playback_us() -> i64 {
    0
}

pub fn format_stop_osd() -> &'static str {
    "Stopped"
}

pub fn format_rotate_osd(mode: RotateMode) -> String {
    format!("Rotate {}", rotate_label(mode))
}

pub fn format_flip_osd(flip_h: bool, flip_v: bool) -> String {
    match (flip_h, flip_v) {
        (false, false) => "Flip off".into(),
        (true, false) => "Flip H".into(),
        (false, true) => "Flip V".into(),
        (true, true) => "Flip HV".into(),
    }
}

/// Seek target for End: stop-time if set, otherwise media duration.
pub fn seek_end_us(duration_us: i64, stop_us: Option<i64>) -> i64 {
    match stop_us {
        Some(stop) if stop >= 0 => clamp_seek_us(stop, duration_us),
        _ => duration_us.max(0),
    }
}

/// Index of the chapter containing `now_us`, if any.
pub fn chapter_index(chapters: &[i64], now_us: i64) -> Option<usize> {
    if chapters.is_empty() {
        return None;
    }
    let mut index = 0usize;
    for (i, &start) in chapters.iter().enumerate() {
        if now_us >= start {
            index = i;
        } else {
            break;
        }
    }
    Some(index)
}

pub fn format_chapter_osd(index: usize, total: usize, start_us: i64) -> String {
    format!(
        "Chapter {}/{} {}",
        index.saturating_add(1),
        total.max(1),
        format_clock(start_us)
    )
}

pub fn format_pause_osd(paused: bool) -> &'static str {
    if paused {
        "Paused"
    } else {
        "Playing"
    }
}

#[derive(Clone, Debug)]
pub struct PlayOptions {
    /// Play the first audio stream on the default output device.
    pub audio: bool,
    /// Playback rate. 1.0 is normal speed. Clamped to 0.25..=4.0.
    pub rate: f32,
    /// Start muted. The volume slider is kept and restored when mute toggles off.
    pub muted: bool,
    /// Open the window already fullscreen.
    pub fullscreen: bool,
    /// Keep the window above other windows.
    pub on_top: bool,
    /// Audio stream ordinal among audio streams. Clamped to the file.
    pub audio_track: u32,
    /// Subtitle stream ordinal. `-1` hides subtitles. `0` is the first subtitle stream.
    /// `i32::MAX` selects the last track, including an external subtitle file.
    pub subtitle_track: i32,
    /// External SRT or ASS file. It becomes the last subtitle track.
    pub subtitles: Option<PathBuf>,
    /// Output device name from `audio_output_devices`. `None` is the host default.
    pub audio_device: Option<String>,
    /// Seek to this media time after open (VLC `--start-time`). Consumed once.
    pub start_us: Option<i64>,
    /// Stop (pause at end) when media time reaches this (VLC `--stop-time`).
    pub stop_us: Option<i64>,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            audio: true,
            rate: 1.0,
            muted: false,
            fullscreen: false,
            on_top: false,
            audio_track: 0,
            subtitle_track: 0,
            subtitles: None,
            audio_device: None,
            start_us: None,
            stop_us: None,
        }
    }
}

/// Clamp a playback rate into the VLC slider range, 0.25×..=4×, as thousandths.
pub fn clamp_rate_milli(rate: f32) -> u32 {
    if !rate.is_finite() || rate <= 0.0 {
        return 1_000;
    }
    let milli = (f64::from(rate) * 1000.0).round() as i64;
    milli.clamp(i64::from(RATE_MIN_MILLI), i64::from(RATE_MAX_MILLI)) as u32
}

/// Wall microseconds become media microseconds at `rate_milli` / 1000.
pub fn scale_elapsed_us(elapsed_us: i64, rate_milli: u32) -> i64 {
    let rate = i64::from(rate_milli.clamp(RATE_MIN_MILLI, RATE_MAX_MILLI));
    elapsed_us.saturating_mul(rate) / 1000
}

/// One output audio frame advances the media-frame phase.
/// `phase` is leftover thousandths. The first value is how many media frames to consume.
pub fn advance_rate_phase(phase: u32, rate_milli: u32) -> (u32, u32) {
    let rate = u64::from(rate_milli.clamp(RATE_MIN_MILLI, RATE_MAX_MILLI));
    let sum = u64::from(phase) + rate;
    ((sum / 1000) as u32, (sum % 1000) as u32)
}

/// Move inside a playlist. `None` means the step would leave the list.
pub fn playlist_step(len: usize, index: usize, delta: i32) -> Option<usize> {
    if len == 0 || index >= len {
        return None;
    }
    let next = i64::try_from(index).ok()?.checked_add(i64::from(delta))?;
    usize::try_from(next).ok().filter(|next| *next < len)
}

/// Playlist repeat. `Off` advances until the last item, then stops.
/// `One` replays the current item. `All` wraps to the first item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

pub fn cycle_repeat(mode: RepeatMode) -> RepeatMode {
    match mode {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    }
}

/// What playback does when the current item reaches the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackContinue {
    Next(usize),
    Restart,
    Stop,
}

pub fn playback_continue(len: usize, index: usize, mode: RepeatMode) -> PlaybackContinue {
    if len == 0 || index >= len {
        return PlaybackContinue::Stop;
    }
    match mode {
        RepeatMode::One => PlaybackContinue::Restart,
        RepeatMode::All => PlaybackContinue::Next(playlist_step(len, index, 1).unwrap_or(0)),
        RepeatMode::Off => match playlist_step(len, index, 1) {
            Some(next) => PlaybackContinue::Next(next),
            None => PlaybackContinue::Stop,
        },
    }
}

/// Deterministic Fisher–Yates order. The same seed always yields the same permutation.
pub fn shuffled_indices(len: usize, seed: u64) -> Vec<usize> {
    let mut items: Vec<usize> = (0..len).collect();
    if len < 2 {
        return items;
    }
    let mut state = seed | 1;
    for index in (1..len).rev() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let pick = (state as usize) % (index + 1);
        items.swap(index, pick);
    }
    items
}

/// Step inside a shuffled playlist. `cursor` is the position in `order`.
/// The pair is `(playlist index, new cursor)`.
pub fn order_step(
    order: &[usize],
    cursor: usize,
    delta: i32,
    wrap: bool,
) -> Option<(usize, usize)> {
    if order.is_empty() || cursor >= order.len() || delta == 0 {
        return None;
    }
    let len = i64::try_from(order.len()).ok()?;
    let current = i64::try_from(cursor).ok()?;
    let next = current.checked_add(i64::from(delta))?;
    let next = if wrap {
        next.rem_euclid(len) as usize
    } else if (0..len).contains(&next) {
        next as usize
    } else {
        return None;
    };
    Some((order[next], next))
}

/// `true` when the text is an HLS media playlist or master playlist, not a file list.
pub fn is_hls_playlist(text: &str) -> bool {
    text.lines().any(|line| line.trim().starts_with("#EXT-X-"))
}

fn playlist_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("m3u")
                || ext.eq_ignore_ascii_case("m3u8")
                || ext.eq_ignore_ascii_case("pls")
        })
}

fn resolve_playlist_entry(target: &str, base: &Path) -> PathBuf {
    if is_playback_url(target) {
        return PathBuf::from(target);
    }
    let path = Path::new(target);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Entries from an M3U or PLS file list. HLS text returns an empty list so the
/// caller can open the playlist itself through libavformat.
pub fn parse_playlist_text(text: &str, base: &Path) -> Vec<PathBuf> {
    let text = text.trim_start_matches('\u{feff}');
    if is_hls_playlist(text) {
        return Vec::new();
    }
    let mut entries = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || (line.starts_with('[') && line.ends_with(']')) {
            continue;
        }
        let target = if let Some(rest) = line.strip_prefix("File") {
            if !rest.starts_with(|ch: char| ch.is_ascii_digit()) {
                continue;
            }
            let Some((_, value)) = rest.split_once('=') else {
                continue;
            };
            value.trim()
        } else if line.contains('=')
            && !is_playback_url(line)
            && !line.contains(['/', '\\'])
        {
            continue;
        } else {
            line
        };
        if target.is_empty() {
            continue;
        }
        entries.push(resolve_playlist_entry(target, base));
    }
    entries
}

/// Replace local M3U/PLS inputs with their entries. Media files and HLS playlists stay as-is.
pub fn expand_play_inputs(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for path in paths {
        let text = path.to_str().unwrap_or("");
        if is_playback_url(text) || !playlist_extension(path) {
            expanded.push(path.clone());
            continue;
        }
        let bytes = std::fs::read(path).map_err(|err| format!("read playlist: {err}"))?;
        let body = String::from_utf8_lossy(&bytes);
        if is_hls_playlist(&body) {
            expanded.push(path.clone());
            continue;
        }
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let entries = parse_playlist_text(&body, base);
        if entries.is_empty() {
            return Err(format!(
                "playlist has no entries: {}",
                path.display()
            ));
        }
        expanded.extend(entries);
    }
    if expanded.is_empty() {
        return Err("play requires an input file".into());
    }
    Ok(expanded)
}

/// Step subtitle delay. Positive delay shows cues later. Each step is 50 ms.
pub fn subtitle_delay_us(current_us: i64, steps: i32) -> i64 {
    current_us.saturating_add(i64::from(steps).saturating_mul(50_000))
}

/// Clear subtitle and audio delay offsets (VLC “Reset Audio/Subtitle Synchronization”).
pub fn reset_av_delays() -> (i64, i64) {
    (0, 0)
}

pub fn format_delay_osd(label: &str, delay_us: i64) -> String {
    format!("{label} delay {} ms", delay_us / 1_000)
}

/// Finer volume step for mouse wheel (±2.5%).
pub const VOLUME_WHEEL_STEP_MILLI: i32 = 25;

pub fn volume_from_wheel(current_milli: u32, scroll_lines: i32) -> u32 {
    if scroll_lines == 0 {
        return clamp_volume_milli(current_milli as i32);
    }
    volume_step_milli(
        current_milli,
        scroll_lines.saturating_mul(VOLUME_WHEEL_STEP_MILLI),
    )
}

/// Clamp a seek target into `[0, duration]` when duration is known.
pub fn clamp_seek_us(target_us: i64, duration_us: i64) -> i64 {
    let target = target_us.max(0);
    if duration_us >= 0 {
        target.min(duration_us)
    } else {
        target
    }
}

pub fn format_ab_osd(ab: Option<AbLoop>) -> String {
    match ab {
        None => "A-B off".into(),
        Some(loop_) if loop_.b_us < 0 => format!("A-B point A {}", format_clock(loop_.a_us)),
        Some(loop_) => format!(
            "A-B {}–{}",
            format_clock(loop_.a_us),
            format_clock(loop_.b_us)
        ),
    }
}

pub fn format_repeat_osd(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "repeat off",
        RepeatMode::All => "repeat all",
        RepeatMode::One => "repeat one",
    }
}

pub fn format_shuffle_osd(on: bool) -> &'static str {
    if on {
        "shuffle on"
    } else {
        "shuffle off"
    }
}

pub fn format_subtitle_scale_osd(scale_milli: i32) -> String {
    format!("Subtitles {}%", clamp_subtitle_scale_milli(scale_milli) / 10)
}

pub fn format_jump_osd(target_us: i64) -> String {
    format!("Jump {}", format_clock(target_us))
}

pub fn format_window_title(
    name: &str,
    media_us: i64,
    duration_us: i64,
    paused: bool,
    rate_milli: u32,
) -> String {
    let now = format_clock(media_us);
    let paused = if paused { "  paused" } else { "" };
    let speed = if rate_milli != 1_000 {
        format!("  {}", format_rate(rate_milli))
    } else {
        String::new()
    };
    if duration_us >= 0 {
        format!(
            "{name}  {now} / {}{paused}{speed}",
            format_clock(duration_us)
        )
    } else {
        format!("{name}  {now}{paused}{speed}")
    }
}

/// Media clock used to pick a cue. A positive delay looks at an earlier time.
pub fn subtitle_clock_us(media_now_us: i64, delay_us: i64) -> i64 {
    media_now_us.saturating_sub(delay_us)
}

/// Media frames of audio/video skew. Positive means emit silence (audio later).
/// `sample_rate == 0` yields 0 so a device that is not open yet does not shift.
pub fn audio_delay_frames(delay_us: i64, sample_rate: u32) -> i64 {
    if sample_rate == 0 {
        return 0;
    }
    delay_us.saturating_mul(i64::from(sample_rate)) / 1_000_000
}

/// One block of output. `media_frames` is how many source frames this block represents.
/// Silence advances the media clock. Dropped frames do not.
pub fn step_audio_skew(skew_frames: i64, media_frames: u32, queued_frames: u64) -> (i64, bool, u64) {
    if media_frames == 0 || skew_frames == 0 {
        return (skew_frames, false, 0);
    }
    if skew_frames > 0 {
        let step = i64::from(media_frames).min(skew_frames);
        return (skew_frames - step, true, 0);
    }
    if queued_frames == 0 {
        return (skew_frames, false, 0);
    }
    let step = i64::from(media_frames)
        .min(-skew_frames)
        .min(queued_frames as i64);
    (skew_frames + step, false, step as u64)
}

/// Cycle a track ordinal. Subtitles include `-1` for off.
pub fn cycle_track(len: usize, current: i32, delta: i32, include_off: bool) -> i32 {
    if len == 0 {
        return -1;
    }
    let span = if include_off {
        len as i32 + 1
    } else {
        len as i32
    };
    let base = if include_off {
        current.clamp(-1, len as i32 - 1) + 1
    } else {
        current.clamp(0, len as i32 - 1)
    };
    let next = (base + delta).rem_euclid(span);
    if include_off { next - 1 } else { next }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleCue {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
}

/// Decoded PAL8 bitmap subtitle plane for overlay (VLC PGS/VobSub path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapSubtitle {
    pub start_us: i64,
    pub end_us: i64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Packed `0xAARRGGBB` top-left first.
    pub pixels: Vec<u32>,
}

/// Map one PAL8 index through a 256-entry RGBA palette (`[R,G,B,A] * 256`).
pub fn palette_rgba_pixel(index: u8, palette: &[u8]) -> u32 {
    let i = usize::from(index) * 4;
    if palette.len() < i + 4 {
        return 0;
    }
    let r = u32::from(palette[i]);
    let g = u32::from(palette[i + 1]);
    let b = u32::from(palette[i + 2]);
    let a = u32::from(palette[i + 3]);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Expand a tightly packed or strided PAL8 bitmap into `0xAARRGGBB` pixels.
pub fn pal8_to_rgba(
    width: u32,
    height: u32,
    bitmap: &[u8],
    stride: usize,
    palette: &[u8],
) -> Result<Vec<u32>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return Err("bitmap subtitle has empty size".into());
    }
    if stride < w {
        return Err("bitmap subtitle stride is too small".into());
    }
    if bitmap.len() < stride.saturating_mul(h.saturating_sub(1)).saturating_add(w) {
        return Err("bitmap subtitle buffer is incomplete".into());
    }
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            pixels.push(palette_rgba_pixel(bitmap[row + x], palette));
        }
    }
    Ok(pixels)
}

/// Alpha-blend `src` (`0xAARRGGBB`) onto opaque `dst` (`0x00RRGGBB`).
pub fn blend_rgba_over_rgb(dst: u32, src: u32) -> u32 {
    let a = ((src >> 24) & 0xff) as u32;
    if a == 0 {
        return dst;
    }
    if a == 255 {
        return src & 0x00ff_ffff;
    }
    let inv = 255 - a;
    let dr = (dst >> 16) & 0xff;
    let dg = (dst >> 8) & 0xff;
    let db = dst & 0xff;
    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;
    let r = (sr * a + dr * inv) / 255;
    let g = (sg * a + dg * inv) / 255;
    let b = (sb * a + db * inv) / 255;
    (r << 16) | (g << 8) | b
}

/// Composite one bitmap plane onto a packed RGB frame. Clips to frame bounds.
pub fn blit_bitmap_subtitle(frame: &mut [u32], frame_w: u32, frame_h: u32, sub: &BitmapSubtitle) {
    if frame_w == 0 || frame_h == 0 || sub.width == 0 || sub.height == 0 {
        return;
    }
    let fw = frame_w as i32;
    let fh = frame_h as i32;
    let sw = sub.width as i32;
    let sh = sub.height as i32;
    if frame.len() < (frame_w as usize) * (frame_h as usize) {
        return;
    }
    if sub.pixels.len() < (sub.width as usize) * (sub.height as usize) {
        return;
    }
    for sy in 0..sh {
        let dy = sub.y + sy;
        if dy < 0 || dy >= fh {
            continue;
        }
        for sx in 0..sw {
            let dx = sub.x + sx;
            if dx < 0 || dx >= fw {
                continue;
            }
            let src = sub.pixels[(sy as usize) * (sub.width as usize) + (sx as usize)];
            let di = (dy as usize) * (frame_w as usize) + (dx as usize);
            frame[di] = blend_rgba_over_rgb(frame[di], src);
        }
    }
}

pub fn active_bitmap_subtitle<'a>(
    planes: &'a [BitmapSubtitle],
    now_us: i64,
) -> Option<&'a BitmapSubtitle> {
    planes
        .iter()
        .rev()
        .find(|plane| plane.start_us <= now_us && now_us < plane.end_us)
}

/// Plain text from an SRT line or an ASS/SSA event.
pub fn plain_subtitle(raw: &str) -> String {
    let payload = subtitle_payload(raw.trim());
    let mut out = String::new();
    let mut chars = payload.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'\\') {
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
            }
            continue;
        }
        if ch == '\\' {
            match chars.peek().copied() {
                Some('N') | Some('n') | Some('h') => {
                    chars.next();
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out.trim().to_string()
}

fn subtitle_payload(raw: &str) -> &str {
    if let Some(rest) = raw.strip_prefix("Dialogue:")
        && let Some(text) = field_after_commas(rest.trim_start(), 9)
    {
        return text;
    }
    if let Some(head) = raw.split(',').next()
        && !head.is_empty()
        && head.chars().all(|ch| ch.is_ascii_digit())
        && let Some(text) = field_after_commas(raw, 8)
    {
        return text;
    }
    raw
}

fn field_after_commas(raw: &str, commas: usize) -> Option<&str> {
    let mut rest = raw;
    for _ in 0..commas {
        let index = rest.find(',')?;
        rest = &rest[index + 1..];
    }
    Some(rest)
}

/// Display window for one decoded cue. Times are media microseconds.
pub fn subtitle_window(packet_us: i64, start_ms: u32, end_ms: u32) -> (i64, i64) {
    let start = packet_us.saturating_add(i64::from(start_ms).saturating_mul(1_000));
    let mut end = packet_us.saturating_add(i64::from(end_ms).saturating_mul(1_000));
    if end <= start {
        end = start.saturating_add(5_000_000);
    }
    (start, end)
}

/// The latest cue that covers `now_us`.
pub fn active_subtitle(cues: &[SubtitleCue], now_us: i64) -> Option<&str> {
    cues.iter()
        .rev()
        .find(|cue| now_us >= cue.start_us && now_us < cue.end_us)
        .map(|cue| cue.text.as_str())
}

/// Parse `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, or ASS `H:MM:SS.cs` into microseconds.
pub fn parse_subtitle_clock(token: &str) -> Option<i64> {
    let token = token.trim();
    let (hms, frac) = token
        .split_once(',')
        .or_else(|| token.split_once('.'))
        .unwrap_or((token, "0"));
    let parts: Vec<&str> = hms.split(':').collect();
    let (hours, minutes, seconds) = match *parts.as_slice() {
        [hours, minutes, seconds] => (hours, minutes, seconds),
        [minutes, seconds] => ("0", minutes, seconds),
        _ => return None,
    };
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    let seconds: i64 = seconds.parse().ok()?;
    let digits: String = frac
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .take(3)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let mut millis: i64 = digits.parse().ok()?;
    if digits.len() == 1 {
        millis *= 100;
    } else if digits.len() == 2 {
        millis *= 10;
    }
    Some(((hours * 3600 + minutes * 60 + seconds) * 1000 + millis) * 1000)
}

/// Parse a jump-to-time string: `HH:MM:SS[.ms]`, `MM:SS[.ms]`, or whole/decimal seconds.
pub fn parse_play_clock(token: &str) -> Option<i64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(us) = parse_subtitle_clock(token) {
        return Some(us.max(0));
    }
    if let Ok(secs) = token.parse::<i64>() {
        return Some(secs.saturating_mul(1_000_000).max(0));
    }
    let secs: f64 = token.parse().ok()?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    Some((secs * 1_000_000.0).round() as i64)
}

/// Cues from an SRT document. Blocks without a timing line are skipped.
pub fn parse_srt(text: &str) -> Vec<SubtitleCue> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in normalized.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
        let Some(first) = lines.next() else {
            continue;
        };
        let timing = if first.contains("-->") {
            first
        } else if let Some(next) = lines.next() {
            next
        } else {
            continue;
        };
        let Some((start, end)) = timing.split_once("-->") else {
            continue;
        };
        let Some(start_us) = parse_subtitle_clock(start) else {
            continue;
        };
        let Some(end_token) = end.split_whitespace().next() else {
            continue;
        };
        let Some(end_us) = parse_subtitle_clock(end_token) else {
            continue;
        };
        let body = lines.collect::<Vec<_>>().join("\n");
        let text = plain_subtitle(&body);
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue {
            start_us,
            end_us: end_us.max(start_us.saturating_add(1)),
            text,
        });
    }
    cues
}

fn parse_ass(text: &str) -> Vec<SubtitleCue> {
    let mut cues = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Dialogue:") else {
            continue;
        };
        let fields: Vec<&str> = rest.splitn(10, ',').map(str::trim).collect();
        if fields.len() < 10 {
            continue;
        }
        let Some(start_us) = parse_subtitle_clock(fields[1]) else {
            continue;
        };
        let Some(end_us) = parse_subtitle_clock(fields[2]) else {
            continue;
        };
        let text = plain_subtitle(fields[9]);
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue {
            start_us,
            end_us: end_us.max(start_us.saturating_add(1)),
            text,
        });
    }
    cues
}

/// Parse an SRT or ASS/SSA document. SRT wins when a cue arrow is present.
pub fn parse_subtitle_text(text: &str) -> Result<Vec<SubtitleCue>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let cues = if text.contains("-->") {
        parse_srt(text)
    } else {
        parse_ass(text)
    };
    if cues.is_empty() {
        return Err("subtitle file has no cues".into());
    }
    Ok(cues)
}

pub fn load_subtitle_file(path: &Path) -> Result<Vec<SubtitleCue>> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    parse_subtitle_text(&String::from_utf8_lossy(&bytes))
}

/// Case-insensitive exact match against enumerated output device names.
pub fn find_audio_device<'a>(names: &'a [String], wanted: &str) -> Option<&'a str> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    names
        .iter()
        .find(|name| name.eq_ignore_ascii_case(wanted))
        .map(String::as_str)
}

/// Names of the host's output devices. Empty when the host reports none.
pub fn audio_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default()
}

/// Step through output device names. Empty list → `None`. Unknown `current` starts at index 0.
pub fn cycle_output_device<'a>(names: &'a [String], current: &str, delta: i32) -> Option<&'a str> {
    if names.is_empty() {
        return None;
    }
    let cur = names
        .iter()
        .position(|name| name.eq_ignore_ascii_case(current.trim()))
        .unwrap_or(0) as i32;
    let len = names.len() as i32;
    let next = (cur + delta).rem_euclid(len) as usize;
    Some(names[next].as_str())
}

/// Schemes libavformat may open for `fvid play`. Local paths stay on the `file` protocol.
pub const PLAYBACK_PROTOCOLS: &str =
    "file,ftp,http,https,mmsh,mmst,rist,rtp,rtmp,rtmps,rtsp,srt,tcp,tls,udp";

pub fn is_playback_url(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }
    PLAYBACK_PROTOCOLS
        .split(',')
        .any(|name| scheme.eq_ignore_ascii_case(name))
}

/// A-B repeat points in media microseconds. `b_us < 0` means only A is armed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbLoop {
    pub a_us: i64,
    pub b_us: i64,
}

/// Advance the A-B control the way VLC and mpv do: unset → A, A → B, A+B → off.
pub fn ab_mark(current: Option<AbLoop>, now_us: i64) -> Option<AbLoop> {
    let now_us = now_us.max(0);
    match current {
        None => Some(AbLoop {
            a_us: now_us,
            b_us: -1,
        }),
        Some(loop_) if loop_.b_us < 0 => {
            if now_us > loop_.a_us {
                Some(AbLoop {
                    a_us: loop_.a_us,
                    b_us: now_us,
                })
            } else if now_us < loop_.a_us {
                Some(AbLoop {
                    a_us: now_us,
                    b_us: loop_.a_us,
                })
            } else {
                Some(loop_)
            }
        }
        Some(_) => None,
    }
}

/// When playback has reached B, return A so the player can seek back. Idle otherwise.
pub fn ab_restart_us(loop_: AbLoop, now_us: i64) -> Option<i64> {
    if loop_.b_us > loop_.a_us && now_us >= loop_.b_us {
        Some(loop_.a_us)
    } else {
        None
    }
}

/// Next or previous chapter start. Previous returns the current chapter when
/// playback is more than 3 seconds past its start, matching VLC and mpv.
pub fn chapter_step(starts_us: &[i64], now_us: i64, delta: i32) -> Option<i64> {
    if starts_us.is_empty() || delta == 0 {
        return None;
    }
    let now_us = now_us.max(0);
    let started = starts_us.partition_point(|start| *start <= now_us);
    if delta > 0 {
        return starts_us.get(started).copied();
    }
    if started == 0 {
        return None;
    }
    let index = started - 1;
    if now_us.saturating_sub(starts_us[index]) > 3_000_000 {
        Some(starts_us[index])
    } else if index == 0 {
        Some(starts_us[0])
    } else {
        Some(starts_us[index - 1])
    }
}

#[derive(Serialize, Debug)]
pub struct PlayStats {
    pub presented_frames: u64,
    pub skipped_frames: u64,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub audio: bool,
    pub sample_rate: u32,
    pub channels: u32,
}

/// Human-readable playback statistics line (VLC-style Info overlay).
pub fn format_play_stats(stats: &PlayStats, media_us: i64, duration_us: i64) -> String {
    let media = format_clock(media_us.max(0));
    let duration = if duration_us >= 0 {
        format_clock(duration_us)
    } else {
        "--:--".into()
    };
    let size = if stats.source_width > 0 && stats.source_height > 0 {
        format!(
            "{}x{}→{}x{}",
            stats.source_width, stats.source_height, stats.width, stats.height
        )
    } else {
        format!("{}x{}", stats.width, stats.height)
    };
    let audio = if stats.audio && stats.sample_rate > 0 {
        format!("{} Hz {}ch", stats.sample_rate, stats.channels.max(1))
    } else {
        "no audio".into()
    };
    format!(
        "{media}/{duration}  {size}  shown {}  drop {}  {audio}",
        stats.presented_frames, stats.skipped_frames
    )
}

struct VideoFrame {
    pts_us: i64,
    duration_us: i64,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
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
        /// When true, graphic EQ is skipped in the audio path.
        eq_bypass: AtomicBool,
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

/// Display aspect. `Source` keeps the frame's own ratio. The others match VLC's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AspectMode {
    #[default]
    Source,
    Square,
    FourThree,
    SixteenNine,
    SixteenTen,
    Anamorphic,
    Cinema,
    Scope,
    FiveFour,
}

pub fn cycle_aspect(mode: AspectMode) -> AspectMode {
    match mode {
        AspectMode::Source => AspectMode::Square,
        AspectMode::Square => AspectMode::FourThree,
        AspectMode::FourThree => AspectMode::SixteenNine,
        AspectMode::SixteenNine => AspectMode::SixteenTen,
        AspectMode::SixteenTen => AspectMode::Anamorphic,
        AspectMode::Anamorphic => AspectMode::Cinema,
        AspectMode::Cinema => AspectMode::Scope,
        AspectMode::Scope => AspectMode::FiveFour,
        AspectMode::FiveFour => AspectMode::Source,
    }
}

pub fn aspect_label(mode: AspectMode) -> &'static str {
    match mode {
        AspectMode::Source => "Auto",
        AspectMode::Square => "1:1",
        AspectMode::FourThree => "4:3",
        AspectMode::SixteenNine => "16:9",
        AspectMode::SixteenTen => "16:10",
        AspectMode::Anamorphic => "2.21:1",
        AspectMode::Cinema => "2.35:1",
        AspectMode::Scope => "2.39:1",
        AspectMode::FiveFour => "5:4",
    }
}

/// Numerator and denominator. `None` means use the source frame.
pub fn aspect_ratio(mode: AspectMode) -> Option<(u32, u32)> {
    match mode {
        AspectMode::Source => None,
        AspectMode::Square => Some((1, 1)),
        AspectMode::FourThree => Some((4, 3)),
        AspectMode::SixteenNine => Some((16, 9)),
        AspectMode::SixteenTen => Some((16, 10)),
        AspectMode::Anamorphic => Some((221, 100)),
        AspectMode::Cinema => Some((235, 100)),
        AspectMode::Scope => Some((239, 100)),
        AspectMode::FiveFour => Some((5, 4)),
    }
}

pub fn frame_aspect(source_w: u32, source_h: u32, mode: AspectMode) -> (u32, u32) {
    aspect_ratio(mode).unwrap_or((source_w.max(1), source_h.max(1)))
}

/// Largest rectangle of `ratio_w:ratio_h` that fits in the bounds.
pub fn fit_aspect(bounds_w: u32, bounds_h: u32, ratio_w: u32, ratio_h: u32) -> (u32, u32) {
    if bounds_w == 0 || bounds_h == 0 || ratio_w == 0 || ratio_h == 0 {
        return (0, 0);
    }
    let wider = u64::from(ratio_w) * u64::from(bounds_h) > u64::from(ratio_h) * u64::from(bounds_w);
    if wider {
        let height = (u64::from(bounds_w) * u64::from(ratio_h) / u64::from(ratio_w))
            .clamp(1, u64::from(bounds_h));
        (bounds_w, height as u32)
    } else {
        let width = (u64::from(bounds_h) * u64::from(ratio_w) / u64::from(ratio_h))
            .clamp(1, u64::from(bounds_w));
        (width as u32, bounds_h)
    }
}

/// Centered crop to `ratio_w:ratio_h`. Returns `(x, y, width, height)` in source pixels.
/// `Source` callers pass the frame size and get the full frame back.
pub fn center_crop(source_w: u32, source_h: u32, ratio_w: u32, ratio_h: u32) -> (u32, u32, u32, u32) {
    if source_w == 0 || source_h == 0 || ratio_w == 0 || ratio_h == 0 {
        return (0, 0, source_w, source_h);
    }
    let wider = u64::from(source_w) * u64::from(ratio_h) > u64::from(source_h) * u64::from(ratio_w);
    if wider {
        let width = (u64::from(source_h) * u64::from(ratio_w) / u64::from(ratio_h))
            .clamp(1, u64::from(source_w));
        let x = (u64::from(source_w) - width) / 2;
        (x as u32, 0, width as u32, source_h)
    } else {
        let height = (u64::from(source_w) * u64::from(ratio_h) / u64::from(ratio_w))
            .clamp(1, u64::from(source_h));
        let y = (u64::from(source_h) - height) / 2;
        (0, y as u32, source_w, height as u32)
    }
}

/// Visible ratio after VLC's crop, then aspect. Aspect wins when it is not Auto.
pub fn display_ratio(
    source_w: u32,
    source_h: u32,
    aspect: AspectMode,
    crop: AspectMode,
) -> (u32, u32) {
    if aspect != AspectMode::Source {
        return frame_aspect(source_w, source_h, aspect);
    }
    frame_aspect(source_w, source_h, crop)
}

/// VLC zoom factors: 1:4, 1:2, 1:1, 2:1. `zoom_in` doubles, otherwise halves.
pub const ZOOM_MIN_MILLI: u32 = 250;
pub const ZOOM_MAX_MILLI: u32 = 2_000;

pub fn zoom_step(current_milli: u32, zoom_in: bool) -> u32 {
    let current = current_milli.clamp(ZOOM_MIN_MILLI, ZOOM_MAX_MILLI);
    if zoom_in {
        current.saturating_mul(2).min(ZOOM_MAX_MILLI)
    } else {
        (current / 2).max(ZOOM_MIN_MILLI)
    }
}

pub fn zoom_label(zoom_milli: u32) -> &'static str {
    match zoom_milli.clamp(ZOOM_MIN_MILLI, ZOOM_MAX_MILLI) {
        250 => "1:4",
        500 => "1:2",
        1_000 => "1:1",
        _ => "2:1",
    }
}

/// Fitted frame scaled by zoom. Values above 1× are clipped by the window.
pub fn zoom_size(fitted_w: u32, fitted_h: u32, zoom_milli: u32) -> (u32, u32) {
    let zoom = u64::from(zoom_milli.clamp(ZOOM_MIN_MILLI, ZOOM_MAX_MILLI));
    let width = (u64::from(fitted_w.max(1)).saturating_mul(zoom) / 1000).max(1);
    let height = (u64::from(fitted_h.max(1)).saturating_mul(zoom) / 1000).max(1);
    (width as u32, height as u32)
}

/// Max absolute pan so content stays fillable in the viewport. Zero when not zoomed past 1× fit.
pub fn clamp_pan_px(value: i32, viewport: u32, content: u32) -> i32 {
    if content <= viewport {
        return 0;
    }
    let max = ((content - viewport) / 2) as i32;
    value.clamp(-max, max)
}

pub fn pan_step_px(current: i32, delta: i32, viewport: u32, content: u32) -> i32 {
    clamp_pan_px(current + delta, viewport, content)
}

/// Apply pan offsets to a centered image rect inside the viewport.
pub fn zoom_pan_rect(
    viewport: (f32, f32, f32, f32),
    content_w: f32,
    content_h: f32,
    pan_x_px: i32,
    pan_y_px: i32,
) -> (f32, f32, f32, f32) {
    let (vx, vy, vw, vh) = viewport;
    let pan_x = clamp_pan_px(pan_x_px, vw.max(0.0) as u32, content_w.max(0.0) as u32) as f32;
    let pan_y = clamp_pan_px(pan_y_px, vh.max(0.0) as u32, content_h.max(0.0) as u32) as f32;
    let x = vx + (vw - content_w) * 0.5 + pan_x;
    let y = vy + (vh - content_h) * 0.5 + pan_y;
    (x, y, content_w, content_h)
}

/// A playback bookmark. Times are media microseconds and stay sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub media_us: i64,
}

/// Insert a bookmark. An existing time is left unchanged and returns false.
pub fn insert_bookmark(marks: &mut Vec<Bookmark>, media_us: i64) -> bool {
    let media_us = media_us.max(0);
    if marks.iter().any(|mark| mark.media_us == media_us) {
        return false;
    }
    marks.push(Bookmark { media_us });
    marks.sort_by_key(|mark| mark.media_us);
    true
}

/// Next bookmark after `now_us`, or the previous one when `delta` is negative.
pub fn bookmark_step(marks: &[Bookmark], now_us: i64, delta: i32) -> Option<i64> {
    if marks.is_empty() || delta == 0 {
        return None;
    }
    let now_us = now_us.max(0);
    if delta > 0 {
        marks
            .iter()
            .find(|mark| mark.media_us > now_us)
            .map(|mark| mark.media_us)
    } else {
        marks
            .iter()
            .rev()
            .find(|mark| mark.media_us < now_us)
            .map(|mark| mark.media_us)
    }
}

/// Pack BGRA rows (with stride) into `0x00RRGGBB` pixels.
pub(crate) fn pack_bgra(
    src: &[u8],
    linesize: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u32>> {
    let row_bytes = width.checked_mul(4).ok_or("frame too large")?;
    if linesize < row_bytes {
        return Err("display stride is shorter than the frame width".into());
    }
    let need = linesize.checked_mul(height).ok_or("frame too large")?;
    if src.len() < need || width == 0 || height == 0 {
        return Err("display frame is incomplete".into());
    }
    let mut out = vec![0u32; width.checked_mul(height).ok_or("frame too large")?];
    for y in 0..height {
        let row = &src[y * linesize..y * linesize + row_bytes];
        for x in 0..width {
            let i = x * 4;
            let b = u32::from(row[i]);
            let g = u32::from(row[i + 1]);
            let r = u32::from(row[i + 2]);
            out[y * width + x] = (r << 16) | (g << 8) | b;
        }
    }
    Ok(out)
}

/// 24-bit bottom-up BMP. Pixels are `0x00RRGGBB`, top row first.
pub fn encode_bmp(width: u32, height: u32, pixels: &[u32]) -> Result<Vec<u8>> {
    let width_us = width as usize;
    let height_us = height as usize;
    if width == 0 || height == 0 || pixels.len() != width_us.saturating_mul(height_us) {
        return Err("snapshot frame is incomplete".into());
    }
    let stride = width_us
        .checked_mul(3)
        .and_then(|bytes| bytes.checked_add(3))
        .map(|bytes| bytes / 4 * 4)
        .ok_or("frame too large")?;
    let image_size = stride.checked_mul(height_us).ok_or("frame too large")?;
    let offset = 54usize;
    let file_size = offset.checked_add(image_size).ok_or("frame too large")?;
    let mut out = vec![0u8; file_size];
    out[0] = b'B';
    out[1] = b'M';
    put_le_u32(&mut out, 2, file_size as u32);
    put_le_u32(&mut out, 10, offset as u32);
    put_le_u32(&mut out, 14, 40);
    put_le_u32(&mut out, 18, width);
    put_le_u32(&mut out, 22, height);
    put_le_u16(&mut out, 26, 1);
    put_le_u16(&mut out, 28, 24);
    put_le_u32(&mut out, 34, image_size as u32);
    for y in 0..height_us {
        let src_y = height_us - 1 - y;
        let row = offset + y * stride;
        for x in 0..width_us {
            let pixel = pixels[src_y * width_us + x];
            let index = row + x * 3;
            out[index] = pixel as u8;
            out[index + 1] = (pixel >> 8) as u8;
            out[index + 2] = (pixel >> 16) as u8;
        }
    }
    Ok(out)
}

/// Snapshot container. VLC-style BMP/PNG choice for `S`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SnapshotFormat {
    #[default]
    Bmp,
    Png,
}

pub fn cycle_snapshot_format(format: SnapshotFormat) -> SnapshotFormat {
    match format {
        SnapshotFormat::Bmp => SnapshotFormat::Png,
        SnapshotFormat::Png => SnapshotFormat::Bmp,
    }
}

pub fn snapshot_format_ext(format: SnapshotFormat) -> &'static str {
    match format {
        SnapshotFormat::Bmp => "bmp",
        SnapshotFormat::Png => "png",
    }
}

/// 8-bit RGB PNG (filter None, zlib stored blocks). Pixels are `0x00RRGGBB`, top row first.
pub fn encode_png(width: u32, height: u32, pixels: &[u32]) -> Result<Vec<u8>> {
    let width_us = width as usize;
    let height_us = height as usize;
    if width == 0 || height == 0 || pixels.len() != width_us.saturating_mul(height_us) {
        return Err("snapshot frame is incomplete".into());
    }
    let row_bytes = width_us
        .checked_mul(3)
        .and_then(|n| n.checked_add(1))
        .ok_or("frame too large")?;
    let raw_len = row_bytes.checked_mul(height_us).ok_or("frame too large")?;
    let mut raw = vec![0u8; raw_len];
    for y in 0..height_us {
        let row = y * row_bytes;
        raw[row] = 0; // filter None
        for x in 0..width_us {
            let pixel = pixels[y * width_us + x];
            let i = row + 1 + x * 3;
            raw[i] = (pixel >> 16) as u8;
            raw[i + 1] = (pixel >> 8) as u8;
            raw[i + 2] = pixel as u8;
        }
    }
    let zlib = zlib_store(&raw)?;
    let mut out = Vec::with_capacity(8 + 25 + zlib.len() + 12 + 12);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    write_png_chunk(&mut out, b"IHDR", &{
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 2; // RGB
        ihdr
    });
    write_png_chunk(&mut out, b"IDAT", &zlib);
    write_png_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn write_png_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffff;
    crc = crc32_png_update(crc, ty);
    crc = crc32_png_update(crc, data) ^ 0xffff_ffff;
    out.extend_from_slice(&crc.to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Result<Vec<u8>> {
    // CMF/FLG + stored DEFLATE blocks + Adler-32.
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 6);
    out.push(0x78);
    out.push(0x01);
    if data.is_empty() {
        out.push(0x01);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    } else {
        let mut offset = 0usize;
        while offset < data.len() {
            let remaining = data.len() - offset;
            let take = remaining.min(65535);
            let last = offset + take >= data.len();
            out.push(if last { 0x01 } else { 0x00 });
            out.extend_from_slice(&(take as u16).to_le_bytes());
            out.extend_from_slice(&(!take as u16).to_le_bytes());
            out.extend_from_slice(&data[offset..offset + take]);
            offset += take;
        }
    }
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());
    Ok(out)
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32_png_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (!(crc & 1)).wrapping_add(1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc
}

fn put_le_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_le_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn snapshot_path(video: &Path, index: u32) -> PathBuf {
    snapshot_path_with_ext(video, index, "bmp")
}

pub fn snapshot_path_with_ext(video: &Path, index: u32, ext: &str) -> PathBuf {
    let stem = video
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("frame");
    let name = format!("{stem}-fvid-{index}.{ext}");
    match video.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(name),
        _ => PathBuf::from(name),
    }
}

/// Align one resampled chunk to the video origin. Later chunks pass through unchanged.
pub(crate) fn take_aligned(
    aligned: &mut bool,
    origin_us: i64,
    rate: i32,
    channels: usize,
    pts_us: i64,
    samples: &[f32],
) -> Vec<f32> {
    if channels == 0 || rate <= 0 {
        return Vec::new();
    }
    if *aligned {
        let usable = samples.len() - samples.len() % channels;
        return samples[..usable].to_vec();
    }
    let usable = samples.len() - samples.len() % channels;
    let frames = usable / channels;
    if frames == 0 {
        return Vec::new();
    }
    let duration_us = (frames as i64).saturating_mul(1_000_000) / i64::from(rate);
    let out = if pts_us >= origin_us {
        let lead = pts_us - origin_us;
        let silence = (lead.saturating_mul(i64::from(rate)) / 1_000_000) as usize;
        let mut padded = vec![0.0; silence * channels];
        padded.extend_from_slice(&samples[..usable]);
        padded
    } else {
        let drop = ((origin_us - pts_us).saturating_mul(i64::from(rate)) / 1_000_000) as usize;
        if drop >= frames {
            Vec::new()
        } else {
            samples[drop * channels..usable].to_vec()
        }
    };
    if pts_us.saturating_add(duration_us) >= origin_us {
        *aligned = true;
    }
    out
}

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
    let chosen = device
        .name()
        .unwrap_or_else(|_| "default".into());
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
        _ => AudioChannelMode::Stereo,
    };
    let balance_milli = shared.balance_milli.load(Ordering::Relaxed);
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
        for channel in 0..channels {
            frame_buf[channel] = apply_eq(frame_buf[channel], channel, &mut tone);
        }
        apply_audio_balance(&mut frame_buf, balance_milli);
        apply_audio_channel(&mut frame_buf, channel_mode);
        for channel in 0..channels {
            write(
                frame_buf[channel],
                &mut data[frame_index * channels + channel],
            );
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
        shared
            .played_samples
            .fetch_add(consumed, Ordering::Relaxed);
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
    shared
        .subtitle_count
        .store(subtitle_streams.len() as u32 + subtitle_extra(shared), Ordering::Relaxed);
    let mut audio_ordinal = bind_ordinal(&audio_streams, shared.audio_ordinal.load(Ordering::Acquire), false, 0);
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
                shared
                    .duration_us
                    .store(next_video_pts, Ordering::Relaxed);
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
    *subtitle_ordinal = bind_ordinal(subtitle_streams, want_sub, true, subtitle_extra(shared) as i32);
    shared.audio_ordinal.store(*audio_ordinal, Ordering::Relaxed);
    shared.subtitle_ordinal.store(*subtitle_ordinal, Ordering::Relaxed);
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

fn playback_seek_ts(origin_us: i64, media_us: i64) -> i64 {
    origin_us.saturating_add(media_us.max(0))
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
    shared
        .audio_eof
        .store(audio.is_none(), Ordering::Release);
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
                unsafe { av_frame_apply_cropping(frame.0, AV_FRAME_CROP_UNALIGNED) },
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
    let absolute = unsafe {
        av_rescale_q(
            ticks,
            time_base,
            AVRational {
                num: 1,
                den: 1_000_000,
            },
        )
    };
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
    audio_ordinal: i32,
    subtitle_ordinal: i32,
    logged_sub: String,
    url_text: String,
    jump_text: String,
    ab: Option<AbLoop>,
    stop_us: Option<i64>,
    repeat: RepeatMode,
    subtitle_delay_us: i64,
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
    show_stats: bool,
    audio_channel: AudioChannelMode,
    balance_milli: i32,
    eq_bypass: bool,
    bass_milli: i32,
    mid_milli: i32,
    treble_milli: i32,
    subtitle_margin_px: i32,
    subtitle_scale_milli: i32,
    rotate: RotateMode,
    eq_gains_milli: [i32; EQ_BAND_COUNT],
    eq_preset: EqPreset,
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
            audio_ordinal,
            subtitle_ordinal,
            logged_sub: String::new(),
            url_text: String::new(),
            jump_text: String::new(),
            ab: None,
            stop_us,
            repeat: RepeatMode::Off,
            subtitle_delay_us: 0,
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
            eq_bypass: false,
            bass_milli: TONE_UNITY_MILLI,
            mid_milli: TONE_UNITY_MILLI,
            treble_milli: TONE_UNITY_MILLI,
            subtitle_margin_px: 0,
            subtitle_scale_milli: SUBTITLE_SCALE_UNITY_MILLI,
            rotate: RotateMode::Deg0,
            eq_gains_milli: eq_unity_gains(),
            eq_preset: EqPreset::Flat,
            outcome,
        };
        if let Err(err) = app.start_session(first) {
            app.error = Some(err);
        }
        eprintln!(
            "fvid play: Space pause, left/right seek, up/down volume, M mute, B audio, V subtitles, L A-B loop, R repeat, G/H subtitle delay, J/K audio delay, A aspect, C crop, Z zoom, Ctrl+B bookmark, Ctrl+R shuffle, T on-top, F fullscreen, [ ] speed, . step, S snapshot, Esc quit"
        );
        app
    }

    fn start_session(&mut self, path: PathBuf) -> Result<()> {
        self.stop_session();
        self.logged_sub.clear();
        self.error = None;
        self.notice = None;
        self.ab = None;
        self.subtitle_delay_us = 0;
        self.audio_delay_us = 0;
        self.bookmarks.clear();
        self.texture = None;
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
            }),
            balance_milli: AtomicI32::new(self.balance_milli),
            eq_bypass: AtomicBool::new(self.eq_bypass),
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
            let presented = self.session.as_ref().map(|session| session.presented).unwrap_or(0);
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
                eprintln!(
                    "fvid play: display {width}x{height} (source {source_w}x{source_h})"
                );
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
                        session.shared.use_audio_clock.store(false, Ordering::Release);
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
            } else if more_due {
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
                !focused && pressed(egui::Key::T),
            )
        });
        if keys.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if keys.1 {
            self.toggle_pause();
        }
        if keys.2 {
            if command {
                self.step_bookmark(-1);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(-PAN_STEP_PX, 0, 800, 450);
                } else {
                    self.request_seek(
                        self.shown_media_us()
                            .saturating_sub(seek_step_us(fine)),
                    );
                }
            }
        }
        if keys.3 {
            if command {
                self.step_bookmark(1);
            } else {
                let fine = ctx.input(|input| input.modifiers.shift);
                if fine && self.zoom_milli > 1_000 {
                    self.nudge_pan(PAN_STEP_PX, 0, 800, 450);
                } else {
                    self.request_seek(
                        self.shown_media_us()
                            .saturating_add(seek_step_us(fine)),
                    );
                }
            }
        }
        if keys.4 {
            let fine = ctx.input(|input| input.modifiers.shift);
            if fine && self.zoom_milli > 1_000 {
                self.nudge_pan(0, -PAN_STEP_PX, 800, 450);
            } else {
                self.nudge_volume(VOLUME_STEP_MILLI);
            }
        }
        if keys.5 {
            let fine = ctx.input(|input| input.modifiers.shift);
            if fine && self.zoom_milli > 1_000 {
                self.nudge_pan(0, PAN_STEP_PX, 800, 450);
            } else {
                self.nudge_volume(-VOLUME_STEP_MILLI);
            }
        }
        if keys.6 {
            self.nudge_rate(-RATE_STEP_MILLI);
        }
        if keys.7 {
            self.nudge_rate(RATE_STEP_MILLI);
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
        if keys.11 {
            if ctx.input(|input| input.modifiers.shift) {
                self.snapshot_format = cycle_snapshot_format(self.snapshot_format);
                self.notice = Some(format!(
                    "Snapshot {}",
                    snapshot_format_ext(self.snapshot_format).to_ascii_uppercase()
                ));
            } else {
                self.save_snapshot();
            }
        }
        if keys.12 {
            if command {
                self.add_bookmark();
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
            self.step_chapter(-1);
        }
        if keys.16 {
            self.step_chapter(1);
        }
        if keys.17 {
            if command {
                self.toggle_shuffle();
            } else {
                self.cycle_repeat_mode();
            }
        }
        if keys.18 {
            self.nudge_subtitle_delay(-1);
        }
        if keys.19 {
            self.nudge_subtitle_delay(1);
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
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::N)) {
            self.step_playlist(1);
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::P)) {
            self.step_playlist(-1);
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Slash)) {
            self.reset_av_sync();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Z)) {
            let zoom_in = !ctx.input(|input| input.modifiers.shift);
            self.nudge_zoom(zoom_in);
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::D)) {
            self.deinterlace = cycle_deinterlace(self.deinterlace);
            self.adjust_dirty = true;
            self.notice = Some(format_deinterlace_osd(self.deinterlace));
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::I)) {
            self.show_stats = !self.show_stats;
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::ArrowUp) && input.modifiers.alt)
        {
            self.subtitle_margin_px = clamp_subtitle_margin(self.subtitle_margin_px + 10);
        }
        if !focused
            && ctx.input(|input| input.key_pressed(egui::Key::ArrowDown) && input.modifiers.alt)
        {
            self.subtitle_margin_px = clamp_subtitle_margin(self.subtitle_margin_px - 10);
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
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::E)) {
            self.toggle_eq_bypass();
        }
        if !focused && ctx.input(|input| input.key_pressed(egui::Key::Y)) {
            self.cycle_audio_channel_mode();
        }
        if !focused
            && ctx.input(|input| {
                input.modifiers.alt
                    && (input.key_pressed(egui::Key::Equals) || input.key_pressed(egui::Key::Plus))
            })
        {
            self.nudge_subtitle_scale(SUBTITLE_SCALE_STEP_MILLI);
        }
        if !focused
            && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::Minus))
        {
            self.nudge_subtitle_scale(-SUBTITLE_SCALE_STEP_MILLI);
        }
        if !focused {
            let (scroll, ctrl) = ctx.input(|input| {
                (input.smooth_scroll_delta.y, input.modifiers.command)
            });
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
        let title = window_title(&name, &session.shared, session.media_now);
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
        let mut frac = self.scrub.unwrap_or(self.progress());
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
                format!("Audio {}/{audio_count}", self.audio_ordinal.saturating_add(1))
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
            if ui
                .button(audio_channel_label(self.audio_channel))
                .clicked()
            {
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

    fn nudge_audio_delay(&mut self, steps: i32) {
        let next = subtitle_delay_us(self.audio_delay_us, steps);
        let Some(session) = &self.session else {
            self.audio_delay_us = next;
            return;
        };
        let rate = session.shared.sample_rate.load(Ordering::Relaxed);
        let delta = audio_delay_frames(next, rate) - audio_delay_frames(self.audio_delay_us, rate);
        self.audio_delay_us = next;
        session
            .shared
            .audio_delay_us
            .store(next, Ordering::Relaxed);
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

    fn add_bookmark(&mut self) {
        let now = self.shown_media_us().max(0);
        let added = insert_bookmark(&mut self.bookmarks, now);
        let notice = if added {
            format!(
                "bookmark {} ({})",
                format_clock(now),
                self.bookmarks.len()
            )
        } else {
            format!("bookmark {} already set", format_clock(now))
        };
        eprintln!("fvid play: {notice}");
        self.notice = Some(notice);
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
        session
            .shared
            .audio_eof
            .store(false, Ordering::Release);
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
            session.shared.rate_milli.store(rate_milli, Ordering::Relaxed);
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
                },
                Ordering::Relaxed,
            );
        }
        self.notice = Some(format_audio_channel_osd(self.audio_channel));
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
    }

    fn cycle_subtitle(&mut self, delta: i32) {
        let Some(session) = &self.session else {
            return;
        };
        let count = session.shared.subtitle_count.load(Ordering::Relaxed) as usize;
        let next = cycle_track(count, self.subtitle_ordinal, delta, true);
        self.subtitle_ordinal = next;
        session.shared.subtitle_ordinal.store(next, Ordering::Release);
        session.shared.track_gen.fetch_add(1, Ordering::Release);
        lock(&session.shared.cues).clear();
        self.logged_sub.clear();
        session.shared.video_cv.notify_all();
        session.shared.audio_cv.notify_all();
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
                        session
                            .shared
                            .subtitle_count
                            .fetch_add(1, Ordering::AcqRel);
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
            };
            let (width, height, pixels) =
                render_play_pixels(frame.width, frame.height, &frame.pixels, &opts, bitmap.as_ref());
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
            let path = snapshot_path_with_ext(
                &session.path,
                self.snapshots + 1,
                snapshot_format_ext(self.snapshot_format),
            );
            (path, bytes)
        };
        match std::fs::write(&encoded.0, &encoded.1) {
            Ok(()) => {
                self.snapshots += 1;
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
            if ui.add_sized([72.0, 28.0], egui::Button::new("Stop")).clicked() {
                self.stop_playback();
            }
            let can_prev = if self.shuffle {
                order_step(&self.order, self.order_cursor, -1, self.repeat == RepeatMode::All).is_some()
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
                order_step(&self.order, self.order_cursor, 1, self.repeat == RepeatMode::All).is_some()
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
        if let Some(notice) = &self.notice {
            ui.colored_label(egui::Color32::from_rgb(255, 186, 92), notice);
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
        if self.zoom_milli > 1_000 && response.dragged() {
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
        );
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
                    }
                } else {
                    match playback_continue(self.playlist.len(), self.playlist_index, self.repeat)
                    {
                        PlaybackContinue::Next(index) => self.goto_playlist(index),
                        PlaybackContinue::Restart => {
                            self.advance_guard = true;
                            self.request_seek(0);
                            self.set_paused(false);
                        }
                        PlaybackContinue::Stop => {}
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
        egui::Panel::bottom("controls").show(ui, |ui| self.controls(ui));
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

fn paint_subtitle(ui: &egui::Ui, rect: egui::Rect, text: &str, margin_px: i32, scale_milli: i32) {
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        return;
    }
    let size = subtitle_font_px(22.0, scale_milli);
    let line_h = size + 4.0;
    let font = egui::FontId::proportional(size);
    let margin = subtitle_margin_px(12, margin_px) as f32;
    let mut y = rect.bottom() - margin - lines.len() as f32 * line_h;
    for line in lines {
        let pos = egui::pos2(rect.center().x, y);
        ui.painter().text(
            pos + egui::vec2(1.5, 1.5),
            egui::Align2::CENTER_TOP,
            line,
            font.clone(),
            egui::Color32::BLACK,
        );
        ui.painter().text(
            pos,
            egui::Align2::CENTER_TOP,
            line,
            font.clone(),
            egui::Color32::WHITE,
        );
        y += line_h;
    }
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

fn window_title(name: &str, shared: &Shared, media_now: i64) -> String {
    format_window_title(
        name,
        media_now,
        shared.duration_us.load(Ordering::Relaxed),
        shared.paused.load(Ordering::Relaxed),
        shared.rate_milli.load(Ordering::Relaxed),
    )
}

fn format_rate(rate_milli: u32) -> String {
    format!("{:.2}x", rate_milli as f32 / 1000.0)
}

fn format_clock(us: i64) -> String {
    let total = (us.max(0) / 1_000_000) as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
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
    swr: *mut SwrContext,
    layout: AVChannelLayout,
    rate: i32,
    channels: i32,
}

impl Drop for PlayResampler {
    fn drop(&mut self) {
        unsafe {
            av_channel_layout_uninit(&mut self.layout);
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
            av_channel_layout_default(&mut layout, channels);
            if layout.nb_channels != channels {
                return Err("failed to build playback channel layout".into());
            }
            let mut swr = ptr::null_mut();
            let mut built = Self {
                swr: ptr::null_mut(),
                layout,
                rate,
                channels,
            };
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
            let dst = Frame::new()?;
            let out_samples = if frame.is_null() {
                let delay = swr_get_delay(self.swr, i64::from(self.rate));
                if delay <= 0 {
                    return Ok(Vec::new());
                }
                delay
            } else {
                let source = &*frame;
                let delay = swr_get_delay(self.swr, i64::from(self.rate));
                av_rescale_rnd(
                    delay + i64::from(source.nb_samples),
                    i64::from(self.rate),
                    i64::from(source.sample_rate.max(1)),
                    AVRounding_AV_ROUND_UP,
                )
            };
            if out_samples <= 0 || out_samples > i64::from(i32::MAX) {
                return Err("resampled audio size overflow".into());
            }
            (*dst.0).format = AVSampleFormat_AV_SAMPLE_FMT_FLT;
            (*dst.0).sample_rate = self.rate;
            (*dst.0).nb_samples = out_samples as i32;
            check(
                av_channel_layout_copy(&mut (*dst.0).ch_layout, &self.layout),
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
