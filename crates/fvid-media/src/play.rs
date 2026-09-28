//! Local window playback. Software decode to a display-sized BGRA buffer, optional
//! device audio, and an egui window. Not a streaming server and not a hardware presenter.
use super::*;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

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

/// Soft-clip a sample after boost so peaks above unity stay bounded (VLC-style).
pub fn soft_clip_sample(sample: f32) -> f32 {
    if sample.abs() <= 1.0 {
        sample.clamp(-1.0, 1.0)
    } else {
        sample.tanh()
    }
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
        frame[i] = tone_step(
            frame[i],
            &mut states[i],
            bass_milli,
            mid_milli,
            treble_milli,
        );
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
pub const EQ_BAND_HZ: [u32; EQ_BAND_COUNT] = [
    60, 170, 310, 600, 1_000, 3_000, 6_000, 12_000, 14_000, 16_000,
];

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
    /// Mid-side karaoke: attenuate centered vocals.
    Karaoke,
}

pub fn cycle_audio_channel(mode: AudioChannelMode) -> AudioChannelMode {
    match mode {
        AudioChannelMode::Stereo => AudioChannelMode::Left,
        AudioChannelMode::Left => AudioChannelMode::Right,
        AudioChannelMode::Right => AudioChannelMode::Mono,
        AudioChannelMode::Mono => AudioChannelMode::Reverse,
        AudioChannelMode::Reverse => AudioChannelMode::Karaoke,
        AudioChannelMode::Karaoke => AudioChannelMode::Stereo,
    }
}

pub fn audio_channel_label(mode: AudioChannelMode) -> &'static str {
    match mode {
        AudioChannelMode::Stereo => "Stereo",
        AudioChannelMode::Left => "Left",
        AudioChannelMode::Right => "Right",
        AudioChannelMode::Mono => "Mono",
        AudioChannelMode::Reverse => "Reverse",
        AudioChannelMode::Karaoke => "Karaoke",
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
        AudioChannelMode::Karaoke => {
            let mid = (left + right) * 0.5;
            frame[0] = (left - mid).clamp(-1.0, 1.0);
            frame[1] = (right - mid).clamp(-1.0, 1.0);
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

/// Stereo width (mid/side): `1000` = normal, `0` = mono, `2000` = 2× wide.
pub const WIDTH_UNITY_MILLI: i32 = 1_000;
pub const WIDTH_MIN_MILLI: i32 = 0;
pub const WIDTH_MAX_MILLI: i32 = 2_000;
pub const WIDTH_STEP_MILLI: i32 = 100;

pub fn clamp_width_milli(value: i32) -> i32 {
    value.clamp(WIDTH_MIN_MILLI, WIDTH_MAX_MILLI)
}

pub fn width_step_milli(current: i32, delta: i32) -> i32 {
    clamp_width_milli(current + delta)
}

/// Mid/side stereo width. No-op when `frame.len() < 2` or width is unity.
pub fn apply_stereo_width(frame: &mut [f32], width_milli: i32) {
    if frame.len() < 2 {
        return;
    }
    let width = clamp_width_milli(width_milli);
    if width == WIDTH_UNITY_MILLI {
        return;
    }
    let scale = width as f32 / 1_000.0;
    let mid = (frame[0] + frame[1]) * 0.5;
    let side = (frame[0] - frame[1]) * 0.5 * scale;
    frame[0] = mid + side;
    frame[1] = mid - side;
}

pub fn format_width_osd(width_milli: i32) -> String {
    let width = clamp_width_milli(width_milli);
    if width == WIDTH_UNITY_MILLI {
        "Stereo width 1.00x".into()
    } else {
        format!("Stereo width {:.2}x", width as f32 / 1_000.0)
    }
}

/// Simple peak compressor: threshold 0..=1, ratio ≥1 (1 = bypass).
pub fn compress_sample(sample: f32, threshold: f32, ratio: f32) -> f32 {
    let threshold = threshold.clamp(0.05, 1.0);
    let ratio = ratio.max(1.0);
    if ratio <= 1.0 {
        return sample;
    }
    let abs = sample.abs();
    if abs <= threshold {
        return sample;
    }
    let compressed = threshold + (abs - threshold) / ratio;
    sample.signum() * compressed
}

pub fn apply_compressor(frame: &mut [f32], enabled: bool, threshold: f32, ratio: f32) {
    if !enabled {
        return;
    }
    for sample in frame.iter_mut() {
        *sample = compress_sample(*sample, threshold, ratio);
    }
}

pub fn format_compressor_osd(enabled: bool) -> &'static str {
    if enabled {
        "Compressor on"
    } else {
        "Compressor off"
    }
}

/// Headphone crossfeed strength: `0` off, `1000` full blend toward mono.
pub const CROSSFEED_MAX_MILLI: i32 = 1_000;
pub const CROSSFEED_STEP_MILLI: i32 = 100;

pub fn clamp_crossfeed_milli(value: i32) -> i32 {
    value.clamp(0, CROSSFEED_MAX_MILLI)
}

pub fn crossfeed_step_milli(current: i32, delta: i32) -> i32 {
    clamp_crossfeed_milli(current + delta)
}

/// Blend L↔R for headphone listening. No-op when strength is 0 or mono.
pub fn apply_crossfeed(frame: &mut [f32], strength_milli: i32) {
    if frame.len() < 2 {
        return;
    }
    let strength = clamp_crossfeed_milli(strength_milli);
    if strength == 0 {
        return;
    }
    let mix = strength as f32 / 1_000.0;
    let left = frame[0];
    let right = frame[1];
    frame[0] = left * (1.0 - mix * 0.5) + right * (mix * 0.5);
    frame[1] = right * (1.0 - mix * 0.5) + left * (mix * 0.5);
}

pub fn format_crossfeed_osd(strength_milli: i32) -> String {
    let strength = clamp_crossfeed_milli(strength_milli);
    if strength == 0 {
        "Crossfeed off".into()
    } else {
        format!("Crossfeed {}%", strength / 10)
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

/// Subtitle opacity. `1000` is fully opaque, `0` is invisible.
pub const SUBTITLE_OPACITY_MIN_MILLI: i32 = 100;
pub const SUBTITLE_OPACITY_MAX_MILLI: i32 = 1_000;
pub const SUBTITLE_OPACITY_UNITY_MILLI: i32 = 1_000;
pub const SUBTITLE_OPACITY_STEP_MILLI: i32 = 100;

pub fn clamp_subtitle_opacity_milli(value: i32) -> i32 {
    value.clamp(SUBTITLE_OPACITY_MIN_MILLI, SUBTITLE_OPACITY_MAX_MILLI)
}

pub fn subtitle_opacity_step_milli(current: i32, delta: i32) -> i32 {
    clamp_subtitle_opacity_milli(current + delta)
}

pub fn format_subtitle_opacity_osd(opacity_milli: i32) -> String {
    format!(
        "Subtitles opacity {}%",
        clamp_subtitle_opacity_milli(opacity_milli) / 10
    )
}

/// Map opacity milli to 0..=255 alpha for painted subtitle glyphs.
pub fn subtitle_opacity_u8(opacity_milli: i32) -> u8 {
    ((clamp_subtitle_opacity_milli(opacity_milli) as i64 * 255) / 1_000).clamp(0, 255) as u8
}

/// Vertical placement for painted subtitle text (VLC-style).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitlePosition {
    #[default]
    Bottom,
    Center,
    Top,
}

pub fn cycle_subtitle_position(position: SubtitlePosition) -> SubtitlePosition {
    match position {
        SubtitlePosition::Bottom => SubtitlePosition::Center,
        SubtitlePosition::Center => SubtitlePosition::Top,
        SubtitlePosition::Top => SubtitlePosition::Bottom,
    }
}

pub fn subtitle_position_label(position: SubtitlePosition) -> &'static str {
    match position {
        SubtitlePosition::Bottom => "bottom",
        SubtitlePosition::Center => "center",
        SubtitlePosition::Top => "top",
    }
}

pub fn format_subtitle_position_osd(position: SubtitlePosition) -> String {
    format!("Subtitles {}", subtitle_position_label(position))
}

/// Y of the top of a subtitle block inside a viewport of `height`.
pub fn subtitle_block_top_y(
    height: f32,
    line_count: usize,
    line_h: f32,
    margin: f32,
    position: SubtitlePosition,
) -> f32 {
    let block_h = line_count as f32 * line_h;
    match position {
        SubtitlePosition::Bottom => (height - margin - block_h).max(0.0),
        SubtitlePosition::Center => ((height - block_h) * 0.5).max(0.0),
        SubtitlePosition::Top => margin.max(0.0),
    }
}

/// Default OSD auto-clear timeout (VLC-style).
pub const OSD_TIMEOUT_DEFAULT_MS: u64 = 3_000;
pub const OSD_TIMEOUT_MIN_MS: u64 = 500;
pub const OSD_TIMEOUT_MAX_MS: u64 = 30_000;

pub fn clamp_osd_timeout_ms(ms: u64) -> u64 {
    ms.clamp(OSD_TIMEOUT_MIN_MS, OSD_TIMEOUT_MAX_MS)
}

/// Whether an OSD notice shown `elapsed_ms` ago should clear.
pub fn osd_should_clear(elapsed_ms: u64, timeout_ms: u64) -> bool {
    elapsed_ms >= clamp_osd_timeout_ms(timeout_ms)
}

/// Hide the mouse cursor after idle (VLC `--mouse-hide-timeout`).
pub const MOUSE_HIDE_DEFAULT_MS: u64 = 1_000;

pub fn mouse_should_hide(idle_ms: u64, timeout_ms: u64) -> bool {
    idle_ms >= timeout_ms.max(1)
}

/// Peak absolute sample amplitude as milli (1000 = 0 dBFS).
pub fn audio_peak_milli(samples: &[f32]) -> u32 {
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0f32, f32::max);
    ((peak * 1_000.0).round() as u32).min(2_000)
}

pub fn format_vu_osd(peak_milli: u32) -> String {
    format!("VU {}%", peak_milli.min(1_000) / 10)
}

/// Fill levels 0..=100 for an N-bar VU meter from peak milli.
pub fn vu_bar_fills(peak_milli: u32, bars: usize) -> Vec<u8> {
    if bars == 0 {
        return Vec::new();
    }
    let peak = peak_milli.min(1_000) as usize;
    let mut fills = Vec::with_capacity(bars);
    for i in 0..bars {
        let threshold = ((i + 1) * 1_000) / bars;
        if peak >= threshold {
            fills.push(100);
        } else {
            let prev = (i * 1_000) / bars;
            if peak <= prev {
                fills.push(0);
            } else {
                let span = (threshold - prev).max(1);
                fills.push((((peak - prev) * 100) / span).min(100) as u8);
            }
        }
    }
    fills
}

/// Quick rate presets: 1× → 0.5× → 2× → 1× (VLC-style speed cycle).
pub fn cycle_rate_preset_milli(current: u32) -> u32 {
    let rate = clamp_rate_milli(current as f32 / 1_000.0);
    match rate {
        750..=1_499 => 500,
        250..=749 => 2_000,
        _ => 1_000,
    }
}

pub fn format_rate_preset_osd(rate_milli: u32) -> String {
    format_rate_osd(clamp_rate_milli(rate_milli as f32 / 1_000.0))
}

/// Vertical marquee / logo text placement (VLC video-title / marquee).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarqueePosition {
    #[default]
    Top,
    Center,
    Bottom,
}

pub fn cycle_marquee_position(position: MarqueePosition) -> MarqueePosition {
    match position {
        MarqueePosition::Top => MarqueePosition::Center,
        MarqueePosition::Center => MarqueePosition::Bottom,
        MarqueePosition::Bottom => MarqueePosition::Top,
    }
}

pub fn marquee_position_label(position: MarqueePosition) -> &'static str {
    match position {
        MarqueePosition::Top => "top",
        MarqueePosition::Center => "center",
        MarqueePosition::Bottom => "bottom",
    }
}

pub fn format_marquee_osd(text: &str, position: MarqueePosition) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        format!("Marquee off ({})", marquee_position_label(position))
    } else {
        format!(
            "Marquee {} \"{}\"",
            marquee_position_label(position),
            trimmed.chars().take(32).collect::<String>()
        )
    }
}

pub fn marquee_block_top_y(
    height: f32,
    line_h: f32,
    margin: f32,
    position: MarqueePosition,
) -> f32 {
    match position {
        MarqueePosition::Top => margin.max(0.0),
        MarqueePosition::Center => ((height - line_h) * 0.5).max(0.0),
        MarqueePosition::Bottom => (height - margin - line_h).max(0.0),
    }
}

pub fn format_title_osd(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Title".into()
    } else {
        format!("Title {}", trimmed.chars().take(48).collect::<String>())
    }
}

/// Late-frame drop policy for play clock catch-up (VLC `--drop-late-frames`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DropFrameMode {
    Off,
    #[default]
    Late,
}

pub fn cycle_drop_frame(mode: DropFrameMode) -> DropFrameMode {
    match mode {
        DropFrameMode::Off => DropFrameMode::Late,
        DropFrameMode::Late => DropFrameMode::Off,
    }
}

pub fn drop_frame_label(mode: DropFrameMode) -> &'static str {
    match mode {
        DropFrameMode::Off => "off",
        DropFrameMode::Late => "late",
    }
}

pub fn format_drop_frame_osd(mode: DropFrameMode) -> String {
    format!("Drop frames {}", drop_frame_label(mode))
}

/// Whether a decoded frame should be skipped to catch up when late by `lateness_us`.
pub fn should_drop_late_frame(mode: DropFrameMode, lateness_us: i64, threshold_us: i64) -> bool {
    matches!(mode, DropFrameMode::Late) && lateness_us >= threshold_us.max(1)
}

pub fn cycle_show_osd(show: bool) -> bool {
    !show
}

pub fn format_show_osd(show: bool) -> &'static str {
    if show { "OSD on" } else { "OSD off" }
}

/// VLC-style video color effects for play display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayEffect {
    #[default]
    Off,
    Invert,
    Sepia,
    Grayscale,
}

pub fn cycle_display_effect(effect: DisplayEffect) -> DisplayEffect {
    match effect {
        DisplayEffect::Off => DisplayEffect::Invert,
        DisplayEffect::Invert => DisplayEffect::Sepia,
        DisplayEffect::Sepia => DisplayEffect::Grayscale,
        DisplayEffect::Grayscale => DisplayEffect::Off,
    }
}

pub fn display_effect_label(effect: DisplayEffect) -> &'static str {
    match effect {
        DisplayEffect::Off => "off",
        DisplayEffect::Invert => "invert",
        DisplayEffect::Sepia => "sepia",
        DisplayEffect::Grayscale => "grayscale",
    }
}

pub fn format_display_effect_osd(effect: DisplayEffect) -> String {
    format!("Effect {}", display_effect_label(effect))
}

pub fn apply_display_effect_pixel(
    red: u8,
    green: u8,
    blue: u8,
    effect: DisplayEffect,
) -> (u8, u8, u8) {
    match effect {
        DisplayEffect::Off => (red, green, blue),
        DisplayEffect::Invert => (255 - red, 255 - green, 255 - blue),
        DisplayEffect::Grayscale => {
            let y =
                ((u16::from(red) * 77 + u16::from(green) * 150 + u16::from(blue) * 29) / 256) as u8;
            (y, y, y)
        }
        DisplayEffect::Sepia => {
            let r = ((u32::from(red) * 393 + u32::from(green) * 769 + u32::from(blue) * 189) / 1000)
                .min(255) as u8;
            let g = ((u32::from(red) * 349 + u32::from(green) * 686 + u32::from(blue) * 168) / 1000)
                .min(255) as u8;
            let b = ((u32::from(red) * 272 + u32::from(green) * 534 + u32::from(blue) * 131) / 1000)
                .min(255) as u8;
            (r, g, b)
        }
    }
}

/// Pick the first track whose language tag matches `prefer` (case-insensitive prefix).
pub fn prefer_track_index(languages: &[&str], prefer: &str, current: usize) -> usize {
    let prefer = prefer.trim();
    if prefer.is_empty() || languages.is_empty() {
        return current.min(languages.len().saturating_sub(1));
    }
    let needle = prefer.to_ascii_lowercase();
    languages
        .iter()
        .enumerate()
        .find(|(_, lang)| {
            let lang = lang.trim().to_ascii_lowercase();
            !lang.is_empty() && (lang.starts_with(&needle) || needle.starts_with(&lang))
        })
        .map(|(i, _)| i)
        .unwrap_or_else(|| current.min(languages.len().saturating_sub(1)))
}

/// Autohide playback controls after idle (VLC qt-fs-controller-autohide).
pub const CONTROLS_AUTOHIDE_DEFAULT_MS: u64 = 3_000;
pub const CONTROLS_AUTOHIDE_MIN_MS: u64 = 500;
pub const CONTROLS_AUTOHIDE_MAX_MS: u64 = 60_000;

pub fn clamp_controls_autohide_ms(ms: u64) -> u64 {
    ms.clamp(CONTROLS_AUTOHIDE_MIN_MS, CONTROLS_AUTOHIDE_MAX_MS)
}

pub fn controls_should_hide(idle_ms: u64, timeout_ms: u64, fullscreen: bool) -> bool {
    fullscreen && idle_ms >= clamp_controls_autohide_ms(timeout_ms)
}

/// Persist resume positions as `path=media_us` lines (VLC media-library style).
pub fn format_resume_positions(entries: &[(String, i64)]) -> String {
    let mut out = String::from("#EXTFVID-RESUME\n");
    for (path, us) in entries {
        if path.trim().is_empty() || *us < 0 {
            continue;
        }
        out.push_str(path.trim());
        out.push('=');
        out.push_str(&us.to_string());
        out.push('\n');
    }
    out
}

pub fn parse_resume_positions(text: &str) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((path, us)) = line.rsplit_once('=') else {
            continue;
        };
        let Ok(us) = us.trim().parse::<i64>() else {
            continue;
        };
        if us < 0 || path.trim().is_empty() {
            continue;
        }
        out.push((path.trim().to_string(), us));
    }
    out
}

pub fn resume_seek_us(entries: &[(String, i64)], path: &str) -> Option<i64> {
    let path = path.trim();
    entries
        .iter()
        .rev()
        .find(|(p, _)| p == path)
        .map(|(_, us)| *us)
}

/// HTTP/network reconnect attempts (VLC `--http-reconnect` style budget).
pub const HTTP_RECONNECT_DEFAULT: u32 = 3;
pub const HTTP_RECONNECT_MAX: u32 = 100;

pub fn clamp_http_reconnect(attempts: u32) -> u32 {
    attempts.min(HTTP_RECONNECT_MAX)
}

pub fn format_http_reconnect_osd(attempts: u32) -> String {
    let n = clamp_http_reconnect(attempts);
    if n == 0 {
        "HTTP reconnect off".into()
    } else {
        format!("HTTP reconnect {n}")
    }
}

/// Whether another reconnect is allowed after `failures` failures.
pub fn http_should_reconnect(failures: u32, max_attempts: u32) -> bool {
    let max = clamp_http_reconnect(max_attempts);
    max > 0 && failures < max
}

/// Scaletempo-style tempo without pitch change: output duration scale for rate.
pub fn scaletempo_duration_us(input_us: i64, rate_milli: u32) -> i64 {
    let rate = clamp_rate_milli(rate_milli as f32 / 1_000.0).max(1);
    input_us
        .saturating_mul(1_000)
        .saturating_div(i64::from(rate))
}

/// Display peak luminance for HDR tonemap (nits). Used to scale Hable/Reinhard output.
pub const HDR_NITS_DEFAULT: u32 = 100;
pub const HDR_NITS_MIN: u32 = 50;
pub const HDR_NITS_MAX: u32 = 10_000;

pub fn clamp_hdr_nits(nits: u32) -> u32 {
    nits.clamp(HDR_NITS_MIN, HDR_NITS_MAX)
}

pub fn format_hdr_nits_osd(nits: u32) -> String {
    format!("HDR peak {} nits", clamp_hdr_nits(nits))
}

/// Scale an SDR channel after tonemap toward a brighter display peak (>100 nits).
pub fn scale_hdr_display_channel(value: u8, nits: u32) -> u8 {
    let nits = clamp_hdr_nits(nits);
    if nits <= HDR_NITS_DEFAULT {
        return value;
    }
    let gain = (nits as f32 / HDR_NITS_DEFAULT as f32).min(4.0);
    ((f32::from(value) * gain).round() as u32).min(255) as u8
}

pub fn cycle_eq_bypass(bypassed: bool) -> bool {
    !bypassed
}

pub fn format_eq_bypass_osd(bypassed: bool) -> &'static str {
    if bypassed { "EQ off" } else { "EQ on" }
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
    /// Average odd/even lines (VLC Linear).
    Linear,
    /// Discard one field by doubling the other (VLC Discard/Mean-like).
    Mean,
}

pub fn cycle_deinterlace(mode: DeinterlaceMode) -> DeinterlaceMode {
    match mode {
        DeinterlaceMode::Off => DeinterlaceMode::Blend,
        DeinterlaceMode::Blend => DeinterlaceMode::Bob,
        DeinterlaceMode::Bob => DeinterlaceMode::Linear,
        DeinterlaceMode::Linear => DeinterlaceMode::Mean,
        DeinterlaceMode::Mean => DeinterlaceMode::Off,
    }
}

pub fn deinterlace_label(mode: DeinterlaceMode) -> &'static str {
    match mode {
        DeinterlaceMode::Off => "Deint",
        DeinterlaceMode::Blend => "Blend",
        DeinterlaceMode::Bob => "Bob",
        DeinterlaceMode::Linear => "Linear",
        DeinterlaceMode::Mean => "Mean",
    }
}

pub fn deinterlace_linear_rgb(pixels: &mut [u32], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h < 2 || pixels.len() < w * h {
        return;
    }
    for y in (0..h.saturating_sub(1)).step_by(2) {
        let top = y * w;
        let bot = (y + 1) * w;
        for x in 0..w {
            pixels[bot + x] = average_rgb_pixel(pixels[top + x], pixels[bot + x]);
        }
    }
}

pub fn deinterlace_mean_rgb(pixels: &mut [u32], width: u32, height: u32) {
    // Mean ≈ keep even lines, copy into odd (field doubling).
    deinterlace_bob_rgb(pixels, width, height);
}

pub fn apply_deinterlace_rgb(pixels: &mut [u32], width: u32, height: u32, mode: DeinterlaceMode) {
    match mode {
        DeinterlaceMode::Off => {}
        DeinterlaceMode::Blend => deinterlace_blend_rgb(pixels, width, height),
        DeinterlaceMode::Bob => deinterlace_bob_rgb(pixels, width, height),
        DeinterlaceMode::Linear => deinterlace_linear_rgb(pixels, width, height),
        DeinterlaceMode::Mean => deinterlace_mean_rgb(pixels, width, height),
    }
}

/// Packed 3D layout → display view for `fvid play` (VLC-style stereo3d subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayStereo3D {
    #[default]
    Off,
    /// Side-by-side left|right → red/cyan anaglyph.
    SbslAnaglyph,
    /// Above-below left/right → red/cyan anaglyph.
    AblAnaglyph,
    /// Show left eye only (SBS half).
    MonoLeft,
    /// Show right eye only (SBS half).
    MonoRight,
}

pub fn cycle_play_stereo3d(mode: PlayStereo3D) -> PlayStereo3D {
    match mode {
        PlayStereo3D::Off => PlayStereo3D::SbslAnaglyph,
        PlayStereo3D::SbslAnaglyph => PlayStereo3D::AblAnaglyph,
        PlayStereo3D::AblAnaglyph => PlayStereo3D::MonoLeft,
        PlayStereo3D::MonoLeft => PlayStereo3D::MonoRight,
        PlayStereo3D::MonoRight => PlayStereo3D::Off,
    }
}

pub fn play_stereo3d_label(mode: PlayStereo3D) -> &'static str {
    match mode {
        PlayStereo3D::Off => "off",
        PlayStereo3D::SbslAnaglyph => "sbsl→anaglyph",
        PlayStereo3D::AblAnaglyph => "abl→anaglyph",
        PlayStereo3D::MonoLeft => "mono-left",
        PlayStereo3D::MonoRight => "mono-right",
    }
}

pub fn format_play_stereo3d_osd(mode: PlayStereo3D) -> String {
    format!("3D {}", play_stereo3d_label(mode))
}

fn anaglyph_rc(left: u32, right: u32) -> u32 {
    let lr = (left >> 16) & 0xff;
    let rg = (right >> 8) & 0xff;
    let rb = right & 0xff;
    (lr << 16) | (rg << 8) | rb
}

/// Convert packed stereo layout into a single-view RGB frame.
pub fn apply_play_stereo3d(
    width: u32,
    height: u32,
    pixels: &[u32],
    mode: PlayStereo3D,
) -> (u32, u32, Vec<u32>) {
    let w = width as usize;
    let h = height as usize;
    if matches!(mode, PlayStereo3D::Off) || w == 0 || h == 0 || pixels.len() < w * h {
        return (width, height, pixels.to_vec());
    }
    match mode {
        PlayStereo3D::SbslAnaglyph | PlayStereo3D::MonoLeft | PlayStereo3D::MonoRight if w >= 2 => {
            let out_w = w / 2;
            let mut out = vec![0u32; out_w * h];
            for y in 0..h {
                for x in 0..out_w {
                    let left = pixels[y * w + x];
                    let right = pixels[y * w + out_w + x];
                    out[y * out_w + x] = match mode {
                        PlayStereo3D::MonoLeft => left,
                        PlayStereo3D::MonoRight => right,
                        _ => anaglyph_rc(left, right),
                    };
                }
            }
            (out_w as u32, height, out)
        }
        PlayStereo3D::AblAnaglyph if h >= 2 => {
            let out_h = h / 2;
            let mut out = vec![0u32; w * out_h];
            for y in 0..out_h {
                for x in 0..w {
                    let left = pixels[y * w + x];
                    let right = pixels[(y + out_h) * w + x];
                    out[y * w + x] = anaglyph_rc(left, right);
                }
            }
            (width, out_h as u32, out)
        }
        _ => (width, height, pixels.to_vec()),
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

/// VLC-style position OSD: elapsed, remaining, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PositionDisplay {
    #[default]
    Elapsed,
    Remaining,
    Both,
}

pub fn cycle_position_display(mode: PositionDisplay) -> PositionDisplay {
    match mode {
        PositionDisplay::Elapsed => PositionDisplay::Remaining,
        PositionDisplay::Remaining => PositionDisplay::Both,
        PositionDisplay::Both => PositionDisplay::Elapsed,
    }
}

/// Remaining media time; `0` when duration is unknown or already past the end.
pub fn remaining_media_us(now_us: i64, duration_us: i64) -> i64 {
    if duration_us < 0 {
        return 0;
    }
    duration_us.saturating_sub(now_us.max(0)).max(0)
}

/// Position line for OSD / title (`01:00 / 02:00`, `-01:00`, or both).
pub fn format_position_osd(now_us: i64, duration_us: i64, mode: PositionDisplay) -> String {
    let now = format_clock(now_us.max(0));
    match mode {
        PositionDisplay::Elapsed => {
            if duration_us >= 0 {
                format!("{now} / {}", format_clock(duration_us))
            } else {
                now
            }
        }
        PositionDisplay::Remaining => {
            if duration_us >= 0 {
                format!("-{}", format_clock(remaining_media_us(now_us, duration_us)))
            } else {
                now
            }
        }
        PositionDisplay::Both => {
            if duration_us >= 0 {
                format!(
                    "{now} / {} (-{})",
                    format_clock(duration_us),
                    format_clock(remaining_media_us(now_us, duration_us))
                )
            } else {
                now
            }
        }
    }
}

/// Peak-follower for VLC-style volume normalizer (`attack`/`release` in 0..=1).
pub fn normalizer_peak_step(prev_peak: f32, sample_peak: f32, attack: f32, release: f32) -> f32 {
    let sample_peak = sample_peak.abs().max(0.0);
    let prev_peak = prev_peak.max(0.0);
    if sample_peak > prev_peak {
        prev_peak + (sample_peak - prev_peak) * attack.clamp(0.0, 1.0)
    } else {
        prev_peak + (sample_peak - prev_peak) * release.clamp(0.0, 1.0)
    }
}

/// Makeup gain so `peak` reaches `target` (capped at 4×).
pub fn normalizer_gain_milli(peak: f32, target: f32) -> u32 {
    let peak = peak.abs().max(1e-6);
    let target = target.clamp(0.1, 1.0);
    let gain = (target / peak).clamp(0.0, 4.0);
    (gain * 1_000.0).round() as u32
}

pub fn apply_normalizer_sample(sample: f32, gain_milli: u32) -> f32 {
    soft_clip_sample(sample * (gain_milli as f32 / 1_000.0))
}

pub fn format_normalizer_osd(enabled: bool, gain_milli: u32) -> String {
    if enabled {
        format!("Vol normalizer {:.2}x", gain_milli as f32 / 1_000.0)
    } else {
        "Vol normalizer off".into()
    }
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
    let percent = if digit == 0 {
        100i64
    } else {
        i64::from(digit) * 10
    };
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekJump {
    pub coarse_us: i64,
    pub fine_us: i64,
}

impl Default for SeekJump {
    fn default() -> Self {
        Self {
            coarse_us: SEEK_COARSE_US,
            fine_us: SEEK_FINE_US,
        }
    }
}

pub fn cycle_seek_jump(jump: SeekJump) -> SeekJump {
    match (jump.coarse_us, jump.fine_us) {
        (SEEK_COARSE_US, SEEK_FINE_US) => SeekJump {
            coarse_us: 30_000_000,
            fine_us: 5_000_000,
        },
        (30_000_000, _) => SeekJump {
            coarse_us: 60_000_000,
            fine_us: 10_000_000,
        },
        _ => SeekJump::default(),
    }
}

pub fn format_seek_jump_osd(jump: SeekJump) -> String {
    format!(
        "Jump ±{}s / ±{}s",
        jump.coarse_us / 1_000_000,
        jump.fine_us / 1_000_000
    )
}

pub fn seek_step_us(fine: bool) -> i64 {
    seek_step_us_ex(fine, SeekJump::default())
}

pub fn seek_step_us_ex(fine: bool, jump: SeekJump) -> i64 {
    if fine {
        jump.fine_us.max(1)
    } else {
        jump.coarse_us.max(1)
    }
}

/// VLC-like video post filters for the play path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VideoPostFx {
    #[default]
    Off,
    Blur,
    Sharpen,
    Grain,
    MotionBlur,
}

pub fn cycle_video_post_fx(fx: VideoPostFx) -> VideoPostFx {
    match fx {
        VideoPostFx::Off => VideoPostFx::Blur,
        VideoPostFx::Blur => VideoPostFx::Sharpen,
        VideoPostFx::Sharpen => VideoPostFx::Grain,
        VideoPostFx::Grain => VideoPostFx::MotionBlur,
        VideoPostFx::MotionBlur => VideoPostFx::Off,
    }
}

pub fn video_post_fx_label(fx: VideoPostFx) -> &'static str {
    match fx {
        VideoPostFx::Off => "PostFX Off",
        VideoPostFx::Blur => "Blur",
        VideoPostFx::Sharpen => "Sharpen",
        VideoPostFx::Grain => "Grain",
        VideoPostFx::MotionBlur => "Motion blur",
    }
}

pub fn format_video_post_fx_osd(fx: VideoPostFx) -> String {
    format!("Video {}", video_post_fx_label(fx))
}

fn unpack_rgb(pixel: u32) -> (u8, u8, u8) {
    (
        ((pixel >> 16) & 0xff) as u8,
        ((pixel >> 8) & 0xff) as u8,
        (pixel & 0xff) as u8,
    )
}

fn pack_rgb(red: u8, green: u8, blue: u8) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub fn apply_video_post_fx(pixels: &mut [u32], width: u32, height: u32, fx: VideoPostFx) {
    match fx {
        VideoPostFx::Off => {}
        VideoPostFx::Blur => apply_box_blur_rgb(pixels, width, height),
        VideoPostFx::Sharpen => apply_sharpen_rgb(pixels, width, height),
        VideoPostFx::Grain => apply_grain_rgb(pixels, width, height),
        VideoPostFx::MotionBlur => {}
    }
}

/// Apply motion blur against a previous frame when `VideoPostFx::MotionBlur` is selected.
pub fn apply_video_post_fx_with_prev(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    fx: VideoPostFx,
    previous: Option<&[u32]>,
) {
    match fx {
        VideoPostFx::MotionBlur => {
            if let Some(prev) = previous {
                apply_motion_blur_rgb(pixels, width, height, prev);
            }
        }
        other => apply_video_post_fx(pixels, width, height, other),
    }
}

fn apply_box_blur_rgb(pixels: &mut [u32], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || pixels.len() < w * h {
        return;
    }
    let src = pixels.to_vec();
    for y in 0..h {
        for x in 0..w {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut n = 0u32;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx < 0 || yy < 0 || xx as usize >= w || yy as usize >= h {
                        continue;
                    }
                    let (rr, gg, bb) = unpack_rgb(src[yy as usize * w + xx as usize]);
                    r += u32::from(rr);
                    g += u32::from(gg);
                    b += u32::from(bb);
                    n += 1;
                }
            }
            if n > 0 {
                pixels[y * w + x] = pack_rgb((r / n) as u8, (g / n) as u8, (b / n) as u8);
            }
        }
    }
}

fn apply_sharpen_rgb(pixels: &mut [u32], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    if w < 3 || h < 3 || pixels.len() < w * h {
        return;
    }
    let src = pixels.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let (cr, cg, cb) = unpack_rgb(src[y * w + x]);
            let (nr, ng, nb) = unpack_rgb(src[(y - 1) * w + x]);
            let (sr, sg, sb) = unpack_rgb(src[(y + 1) * w + x]);
            let (wr, wg, wb) = unpack_rgb(src[y * w + (x - 1)]);
            let (er, eg, eb) = unpack_rgb(src[y * w + (x + 1)]);
            let sharpen = |c: u8, n: u8, s: u8, ww: u8, e: u8| -> u8 {
                let v =
                    i32::from(c) * 5 - i32::from(n) - i32::from(s) - i32::from(ww) - i32::from(e);
                v.clamp(0, 255) as u8
            };
            pixels[y * w + x] = pack_rgb(
                sharpen(cr, nr, sr, wr, er),
                sharpen(cg, ng, sg, wg, eg),
                sharpen(cb, nb, sb, wb, eb),
            );
        }
    }
}

fn apply_grain_rgb(pixels: &mut [u32], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || pixels.len() < w * h {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let mut hash =
                ((x as u32).wrapping_mul(374761393)) ^ ((y as u32).wrapping_mul(668265263));
            hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
            let noise = ((hash >> 24) as i32) - 128;
            let delta = noise / 12;
            let (r, g, b) = unpack_rgb(pixels[y * w + x]);
            pixels[y * w + x] = pack_rgb(
                (i32::from(r) + delta).clamp(0, 255) as u8,
                (i32::from(g) + delta).clamp(0, 255) as u8,
                (i32::from(b) + delta).clamp(0, 255) as u8,
            );
        }
    }
}

/// Fold 5.1/7.1 interleaved frames down to stereo (VLC headphone/stereo downmix).
pub fn downmix_surround_to_stereo(frame: &mut [f32], channels: usize) {
    if channels < 3 || frame.len() < channels {
        return;
    }
    let left = frame[0];
    let right = frame[1.min(channels - 1)];
    let center = if channels >= 3 { frame[2] * 0.707 } else { 0.0 };
    let lfe = if channels >= 4 { frame[3] * 0.5 } else { 0.0 };
    let ls = if channels >= 5 { frame[4] * 0.707 } else { 0.0 };
    let rs = if channels >= 6 { frame[5] * 0.707 } else { 0.0 };
    let out_l = (left + center + lfe + ls).clamp(-1.0, 1.0);
    let out_r = (right + center + lfe + rs).clamp(-1.0, 1.0);
    frame[0] = out_l;
    if channels > 1 {
        frame[1] = out_r;
    }
}

pub fn format_downmix_osd(enabled: bool) -> &'static str {
    if enabled {
        "Surround downmix On"
    } else {
        "Surround downmix Off"
    }
}

pub fn format_scaletempo_osd(enabled: bool) -> &'static str {
    if enabled {
        "Scaletempo On"
    } else {
        "Scaletempo Off"
    }
}

pub fn format_minimal_interface_osd(enabled: bool) -> &'static str {
    if enabled {
        "Minimal interface On"
    } else {
        "Minimal interface Off"
    }
}

/// Independent audio pitch ratio (VLC Advanced pitch), milli-units around unity.
pub const AUDIO_PITCH_UNITY_MILLI: i32 = 1_000;
pub const AUDIO_PITCH_MIN_MILLI: i32 = 500;
pub const AUDIO_PITCH_MAX_MILLI: i32 = 2_000;
pub const AUDIO_PITCH_STEP_MILLI: i32 = 50;

pub fn clamp_audio_pitch_milli(value: i32) -> i32 {
    value.clamp(AUDIO_PITCH_MIN_MILLI, AUDIO_PITCH_MAX_MILLI)
}

pub fn audio_pitch_step_milli(current: i32, delta: i32) -> i32 {
    clamp_audio_pitch_milli(current.saturating_add(delta))
}

pub fn apply_audio_pitch_sample_index(index: u64, pitch_milli: i32) -> u64 {
    let pitch = clamp_audio_pitch_milli(pitch_milli) as u64;
    index.saturating_mul(AUDIO_PITCH_UNITY_MILLI as u64) / pitch.max(1)
}

pub fn format_audio_pitch_osd(pitch_milli: i32) -> String {
    format!(
        "Audio pitch {:.2}×",
        clamp_audio_pitch_milli(pitch_milli) as f32 / 1_000.0
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VisualizationMode {
    #[default]
    Off,
    Spectrum,
    Scope,
    VUMeter,
}

pub fn cycle_visualization(mode: VisualizationMode) -> VisualizationMode {
    match mode {
        VisualizationMode::Off => VisualizationMode::Spectrum,
        VisualizationMode::Spectrum => VisualizationMode::Scope,
        VisualizationMode::Scope => VisualizationMode::VUMeter,
        VisualizationMode::VUMeter => VisualizationMode::Off,
    }
}

pub fn visualization_label(mode: VisualizationMode) -> &'static str {
    match mode {
        VisualizationMode::Off => "Off",
        VisualizationMode::Spectrum => "Spectrum",
        VisualizationMode::Scope => "Scope",
        VisualizationMode::VUMeter => "VU meter",
    }
}

pub fn format_visualization_osd(mode: VisualizationMode) -> String {
    format!("Visualization {}", visualization_label(mode))
}

/// Scope polyline samples normalized to 0..=255 for overlay drawing.
pub fn scope_samples_u8(samples: &[f32], points: usize) -> Vec<u8> {
    let points = points.max(1).min(512);
    let mut out = vec![128u8; points];
    if samples.is_empty() {
        return out;
    }
    let chunk = (samples.len() / points).max(1);
    for (i, slot) in out.iter_mut().enumerate() {
        let start = i * chunk;
        let end = (start + chunk).min(samples.len());
        if start >= samples.len() {
            break;
        }
        let mut acc = 0.0f32;
        let mut n = 0u32;
        for sample in &samples[start..end] {
            acc += *sample;
            n += 1;
        }
        let mean = if n == 0 { 0.0 } else { acc / n as f32 };
        *slot = ((mean * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Audio bargraph overlay levels (VLC audiobargraph).
pub fn audio_bargraph_fills(peaks_milli: &[u32], bars: usize) -> Vec<u8> {
    let bars = bars.max(1).min(32);
    let mut out = vec![0u8; bars];
    if peaks_milli.is_empty() {
        return out;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let peak = peaks_milli[i % peaks_milli.len()].min(2_000);
        *slot = ((peak * 255) / 2_000) as u8;
    }
    out
}

pub fn format_bargraph_osd(fills: &[u8]) -> String {
    let lit = fills.iter().filter(|v| **v > 8).count();
    format!("Bargraph {lit}/{}", fills.len().max(1))
}

pub fn apply_motion_blur_rgb(pixels: &mut [u32], width: u32, height: u32, previous: &[u32]) {
    let n = (width as usize).saturating_mul(height as usize);
    if n == 0 || pixels.len() < n || previous.len() < n {
        return;
    }
    for i in 0..n {
        pixels[i] = average_rgb_pixel(pixels[i], previous[i]);
    }
}

pub fn format_motion_blur_osd(enabled: bool) -> &'static str {
    if enabled {
        "Motion blur On"
    } else {
        "Motion blur Off"
    }
}

/// Still-image playlist dwell (VLC `--image-duration`), seconds.
pub const IMAGE_DURATION_DEFAULT_SECS: u32 = 10;
pub const IMAGE_DURATION_MIN_SECS: u32 = 1;
pub const IMAGE_DURATION_MAX_SECS: u32 = 3_600;

pub fn clamp_image_duration_secs(secs: u32) -> u32 {
    secs.clamp(IMAGE_DURATION_MIN_SECS, IMAGE_DURATION_MAX_SECS)
}

pub fn image_duration_us(secs: u32) -> i64 {
    i64::from(clamp_image_duration_secs(secs)).saturating_mul(1_000_000)
}

pub fn format_image_duration_osd(secs: u32) -> String {
    format!("Image duration {} s", clamp_image_duration_secs(secs))
}

/// CEA-608 / closed-caption channels (VLC CC1–CC4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ClosedCaptionChannel {
    #[default]
    Off,
    Cc1,
    Cc2,
    Cc3,
    Cc4,
}

pub fn cycle_closed_caption(channel: ClosedCaptionChannel) -> ClosedCaptionChannel {
    match channel {
        ClosedCaptionChannel::Off => ClosedCaptionChannel::Cc1,
        ClosedCaptionChannel::Cc1 => ClosedCaptionChannel::Cc2,
        ClosedCaptionChannel::Cc2 => ClosedCaptionChannel::Cc3,
        ClosedCaptionChannel::Cc3 => ClosedCaptionChannel::Cc4,
        ClosedCaptionChannel::Cc4 => ClosedCaptionChannel::Off,
    }
}

pub fn closed_caption_label(channel: ClosedCaptionChannel) -> &'static str {
    match channel {
        ClosedCaptionChannel::Off => "CC Off",
        ClosedCaptionChannel::Cc1 => "CC1",
        ClosedCaptionChannel::Cc2 => "CC2",
        ClosedCaptionChannel::Cc3 => "CC3",
        ClosedCaptionChannel::Cc4 => "CC4",
    }
}

pub fn format_closed_caption_osd(channel: ClosedCaptionChannel) -> String {
    format!("Captions {}", closed_caption_label(channel))
}

/// Custom crop rectangle in source pixels (VLC Video Crop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CropPixels {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

pub fn clamp_crop_pixels(crop: CropPixels, width: u32, height: u32) -> CropPixels {
    let max_x = width.saturating_sub(1);
    let max_y = height.saturating_sub(1);
    let left = crop.left.min(max_x);
    let right = crop.right.min(width.saturating_sub(left).saturating_sub(1));
    let top = crop.top.min(max_y);
    let bottom = crop
        .bottom
        .min(height.saturating_sub(top).saturating_sub(1));
    CropPixels {
        left,
        top,
        right,
        bottom,
    }
}

pub fn crop_output_size(width: u32, height: u32, crop: CropPixels) -> (u32, u32) {
    let crop = clamp_crop_pixels(crop, width, height);
    (
        width.saturating_sub(crop.left + crop.right).max(1),
        height.saturating_sub(crop.top + crop.bottom).max(1),
    )
}

pub fn format_crop_pixels_osd(crop: CropPixels) -> String {
    format!(
        "Crop L{} T{} R{} B{}",
        crop.left, crop.top, crop.right, crop.bottom
    )
}

/// Audio/video desync in milliseconds (VLC `--audio-desync`).
pub fn audio_desync_us(ms: i32) -> i64 {
    i64::from(ms).saturating_mul(1_000)
}

pub fn audio_desync_ms_from_us(us: i64) -> i32 {
    (us / 1_000).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub fn format_audio_desync_osd(ms: i32) -> String {
    format!("Audio desync {ms} ms")
}

/// Desktop wallpaper / video desktop mode (VLC Video wallpaper).
pub fn format_wallpaper_osd(enabled: bool) -> &'static str {
    if enabled {
        "Wallpaper mode On"
    } else {
        "Wallpaper mode Off"
    }
}

/// MPEG-TS program / service id selection.
pub fn prefer_program_index(program_ids: &[u32], prefer: u32, current: usize) -> usize {
    if program_ids.is_empty() {
        return current;
    }
    if let Some(idx) = program_ids.iter().position(|id| *id == prefer) {
        return idx;
    }
    current.min(program_ids.len() - 1)
}

pub fn format_program_osd(program_id: u32) -> String {
    format!("Program {program_id}")
}

/// Subtitle text encoding / codepage label (VLC `--subsdec-encoding`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleEncoding {
    #[default]
    Utf8,
    Cp1251,
    Cp1252,
    Latin1,
    ShiftJis,
}

pub fn cycle_subtitle_encoding(enc: SubtitleEncoding) -> SubtitleEncoding {
    match enc {
        SubtitleEncoding::Utf8 => SubtitleEncoding::Cp1251,
        SubtitleEncoding::Cp1251 => SubtitleEncoding::Cp1252,
        SubtitleEncoding::Cp1252 => SubtitleEncoding::Latin1,
        SubtitleEncoding::Latin1 => SubtitleEncoding::ShiftJis,
        SubtitleEncoding::ShiftJis => SubtitleEncoding::Utf8,
    }
}

pub fn subtitle_encoding_label(enc: SubtitleEncoding) -> &'static str {
    match enc {
        SubtitleEncoding::Utf8 => "UTF-8",
        SubtitleEncoding::Cp1251 => "CP1251",
        SubtitleEncoding::Cp1252 => "CP1252",
        SubtitleEncoding::Latin1 => "Latin-1",
        SubtitleEncoding::ShiftJis => "Shift-JIS",
    }
}

pub fn format_subtitle_encoding_osd(enc: SubtitleEncoding) -> String {
    format!("Subs encoding {}", subtitle_encoding_label(enc))
}

/// Teletext page selection (VLC Teletext).
pub const TELETEXT_PAGE_DEFAULT: u32 = 100;
pub const TELETEXT_PAGE_MIN: u32 = 100;
pub const TELETEXT_PAGE_MAX: u32 = 899;

pub fn clamp_teletext_page(page: u32) -> u32 {
    page.clamp(TELETEXT_PAGE_MIN, TELETEXT_PAGE_MAX)
}

pub fn teletext_page_step(current: u32, delta: i32) -> u32 {
    let next = (clamp_teletext_page(current) as i64).saturating_add(i64::from(delta));
    clamp_teletext_page(
        next.clamp(i64::from(TELETEXT_PAGE_MIN), i64::from(TELETEXT_PAGE_MAX)) as u32,
    )
}

pub fn format_teletext_osd(page: u32, enabled: bool) -> String {
    if enabled {
        format!("Teletext {}", clamp_teletext_page(page))
    } else {
        "Teletext Off".into()
    }
}

/// Keep video aspect locked when window is resized (VLC "Keep original AR").
pub fn format_aspect_lock_osd(locked: bool) -> &'static str {
    if locked {
        "Aspect lock On"
    } else {
        "Aspect lock Off"
    }
}

/// Fit window size to content while respecting a locked aspect.
pub fn locked_window_size(
    video_w: u32,
    video_h: u32,
    max_w: u32,
    max_h: u32,
    locked: bool,
) -> (u32, u32) {
    let vw = video_w.max(1);
    let vh = video_h.max(1);
    if !locked {
        return (vw.min(max_w.max(1)), vh.min(max_h.max(1)));
    }
    let max_w = max_w.max(1);
    let max_h = max_h.max(1);
    let scale_w = max_w as f64 / vw as f64;
    let scale_h = max_h as f64 / vh as f64;
    let scale = scale_w.min(scale_h);
    (
        ((vw as f64) * scale).round().max(1.0) as u32,
        ((vh as f64) * scale).round().max(1.0) as u32,
    )
}

/// Sequential snapshot counter with width padding (VLC snapshot-sequential).
pub fn format_snapshot_sequential_name(prefix: &str, index: u32, width: u32, ext: &str) -> String {
    let prefix = prefix.trim();
    let width = width.clamp(1, 8) as usize;
    let num = format!("{index:0width$}");
    if prefix.is_empty() {
        format!("vlcsnap-{num}.{ext}")
    } else {
        format!("{prefix}{num}.{ext}")
    }
}

pub fn format_snapshot_sequential_osd(enabled: bool, index: u32) -> String {
    if enabled {
        format!("Snapshot sequential #{index}")
    } else {
        "Snapshot sequential Off".into()
    }
}

/// Hardware decode preference flag (oracle; software path remains the default).
pub fn format_hw_decode_osd(enabled: bool) -> &'static str {
    if enabled {
        "HW decode preferred"
    } else {
        "HW decode Off"
    }
}

/// Fingerprinting / Next / AcoustID style media id stub for play library hooks.
pub fn format_media_fingerprint_osd(fingerprint: &str) -> String {
    let trimmed = fingerprint.trim();
    if trimmed.is_empty() {
        "Fingerprint none".into()
    } else if trimmed.len() <= 12 {
        format!("Fingerprint {trimmed}")
    } else {
        format!("Fingerprint {}…", &trimmed[..12])
    }
}

/// Logo overlay opacity / position (VLC logo filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LogoPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

pub fn cycle_logo_position(pos: LogoPosition) -> LogoPosition {
    match pos {
        LogoPosition::TopLeft => LogoPosition::TopRight,
        LogoPosition::TopRight => LogoPosition::BottomLeft,
        LogoPosition::BottomLeft => LogoPosition::BottomRight,
        LogoPosition::BottomRight => LogoPosition::Center,
        LogoPosition::Center => LogoPosition::TopLeft,
    }
}

pub fn logo_position_label(pos: LogoPosition) -> &'static str {
    match pos {
        LogoPosition::TopLeft => "Top-Left",
        LogoPosition::TopRight => "Top-Right",
        LogoPosition::BottomLeft => "Bottom-Left",
        LogoPosition::BottomRight => "Bottom-Right",
        LogoPosition::Center => "Center",
    }
}

pub fn clamp_logo_opacity_milli(value: i32) -> i32 {
    value.clamp(0, 1_000)
}

pub fn format_logo_osd(pos: LogoPosition, opacity_milli: i32) -> String {
    format!(
        "Logo {} {}%",
        logo_position_label(pos),
        clamp_logo_opacity_milli(opacity_milli) / 10
    )
}

pub fn logo_anchor_xy(
    video_w: u32,
    video_h: u32,
    logo_w: u32,
    logo_h: u32,
    pos: LogoPosition,
    margin: u32,
) -> (u32, u32) {
    let margin = margin.min(video_w.min(video_h) / 2);
    let max_x = video_w.saturating_sub(logo_w);
    let max_y = video_h.saturating_sub(logo_h);
    match pos {
        LogoPosition::TopLeft => (margin.min(max_x), margin.min(max_y)),
        LogoPosition::TopRight => (max_x.saturating_sub(margin), margin.min(max_y)),
        LogoPosition::BottomLeft => (margin.min(max_x), max_y.saturating_sub(margin)),
        LogoPosition::BottomRight => (max_x.saturating_sub(margin), max_y.saturating_sub(margin)),
        LogoPosition::Center => (max_x / 2, max_y / 2),
    }
}

/// Mosaic tile grid for multi-input preview (VLC mosaic).
pub fn mosaic_tile_rect(
    canvas_w: u32,
    canvas_h: u32,
    cols: u32,
    rows: u32,
    index: u32,
) -> (u32, u32, u32, u32) {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let tile_w = canvas_w / cols;
    let tile_h = canvas_h / rows;
    let index = index % (cols * rows);
    let col = index % cols;
    let row = index / cols;
    (col * tile_w, row * tile_h, tile_w.max(1), tile_h.max(1))
}

pub fn format_mosaic_osd(cols: u32, rows: u32) -> String {
    format!("Mosaic {cols}×{rows}")
}

/// Parametric audio filter gain (VLC param_eq style single-band).
pub fn clamp_param_eq_milli(value: i32) -> i32 {
    value.clamp(-2_000, 2_000)
}

pub fn apply_param_eq_sample(sample: f32, gain_milli: i32) -> f32 {
    let gain = 1.0 + (clamp_param_eq_milli(gain_milli) as f32 / 1_000.0);
    (sample * gain).clamp(-1.0, 1.0)
}

pub fn format_param_eq_osd(gain_milli: i32) -> String {
    format!("Param EQ {:+} dB", clamp_param_eq_milli(gain_milli) / 100)
}

/// Audio amplifier (VLC volume amp beyond slider).
pub fn clamp_amplifier_milli(value: i32) -> i32 {
    value.clamp(0, 4_000)
}

pub fn apply_amplifier_sample(sample: f32, amp_milli: i32) -> f32 {
    let gain = clamp_amplifier_milli(amp_milli) as f32 / 1_000.0;
    soft_clip_sample(sample * gain)
}

pub fn format_amplifier_osd(amp_milli: i32) -> String {
    format!("Amplifier {}%", clamp_amplifier_milli(amp_milli) / 10)
}

/// Record-while-playing destination naming (VLC record).
pub fn format_record_path(dir: Option<&Path>, stem: &str, index: u32, ext: &str) -> PathBuf {
    let name = format!("{stem}-rec-{index}.{ext}");
    match dir {
        Some(folder) if !folder.as_os_str().is_empty() => folder.join(name),
        _ => PathBuf::from(name),
    }
}

pub fn format_record_osd(recording: bool, path: Option<&Path>) -> String {
    if !recording {
        return "Record Off".into();
    }
    match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some(name) => format!("Recording {name}"),
        None => "Recording".into(),
    }
}

/// HTTP basic-auth credential presence (oracle; secrets stay out of logs).
pub fn format_http_auth_osd(has_user: bool, has_password: bool) -> &'static str {
    match (has_user, has_password) {
        (false, false) => "HTTP auth Off",
        (true, false) => "HTTP auth user",
        (true, true) => "HTTP auth user+pass",
        (false, true) => "HTTP auth pass-only",
    }
}

/// Network proxy mode for play inputs (VLC `--http-proxy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProxyMode {
    #[default]
    Off,
    Http,
    Socks,
}

pub fn cycle_proxy_mode(mode: ProxyMode) -> ProxyMode {
    match mode {
        ProxyMode::Off => ProxyMode::Http,
        ProxyMode::Http => ProxyMode::Socks,
        ProxyMode::Socks => ProxyMode::Off,
    }
}

pub fn proxy_mode_label(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::Off => "Off",
        ProxyMode::Http => "HTTP",
        ProxyMode::Socks => "SOCKS",
    }
}

pub fn format_proxy_osd(mode: ProxyMode, host: &str) -> String {
    let host = host.trim();
    if matches!(mode, ProxyMode::Off) || host.is_empty() {
        format!("Proxy {}", proxy_mode_label(mode))
    } else {
        format!("Proxy {} {host}", proxy_mode_label(mode))
    }
}

/// Adaptive streaming quality ladder pick (HLS/DASH style).
pub fn prefer_stream_quality_index(bitrates_kbps: &[u32], prefer_kbps: u32) -> usize {
    if bitrates_kbps.is_empty() {
        return 0;
    }
    let mut best = 0usize;
    let mut best_delta = u32::MAX;
    for (i, &rate) in bitrates_kbps.iter().enumerate() {
        let delta = rate.abs_diff(prefer_kbps);
        if delta < best_delta || (delta == best_delta && rate > bitrates_kbps[best]) {
            best = i;
            best_delta = delta;
        }
    }
    best
}

pub fn format_stream_quality_osd(bitrate_kbps: u32) -> String {
    format!("Quality {bitrate_kbps} kbps")
}

/// Subtitle autodetect fuzzy match for external files beside the media.
pub fn prefer_external_subtitle_path(media: &Path, candidates: &[&Path]) -> Option<PathBuf> {
    let stem = media.file_stem()?.to_str()?.to_ascii_lowercase();
    let mut best: Option<(usize, PathBuf)> = None;
    for candidate in candidates {
        let name = candidate
            .file_stem()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase())?;
        let score = if name == stem {
            0
        } else if name.starts_with(&stem) {
            1
        } else if name.contains(&stem) {
            2
        } else {
            continue;
        };
        match &best {
            Some((prev, _)) if *prev <= score => {}
            _ => best = Some((score, candidate.to_path_buf())),
        }
    }
    best.map(|(_, path)| path)
}

pub fn format_external_subtitle_osd(path: Option<&Path>) -> String {
    match path.and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some(name) => format!("Ext sub {name}"),
        None => "Ext sub none".into(),
    }
}

/// 360° source layout / projection mode beyond plain equirect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SphericalProjection {
    #[default]
    Equirect,
    DualFisheye,
    Cubemap,
    LittlePlanet,
    /// YouTube Equi-Angular Cubemap (EAC).
    Eac,
    /// Panini projection (architectural / wide FOV 360).
    Panini,
    /// Cylindrical panorama.
    Cylindrical,
    /// Mercator map projection.
    Mercator,
    /// Dual fisheye top-bottom layout.
    DualFisheyeTb,
    /// Octahedral environment map.
    Octahedral,
    /// Equisolid fisheye viewport over equirect.
    Equisolid,
    /// Orthographic globe projection.
    Orthographic,
    /// Gnomonic (rectilinear tangent) projection.
    Gnomonic,
    /// Sinusoidal equal-area projection.
    Sinusoidal,
    /// Miller cylindrical projection.
    Miller,
    /// Azimuthal equidistant projection.
    AzimuthalEquidistant,
}

pub fn cycle_spherical_projection(mode: SphericalProjection) -> SphericalProjection {
    match mode {
        SphericalProjection::Equirect => SphericalProjection::DualFisheye,
        SphericalProjection::DualFisheye => SphericalProjection::Cubemap,
        SphericalProjection::Cubemap => SphericalProjection::LittlePlanet,
        SphericalProjection::LittlePlanet => SphericalProjection::Eac,
        SphericalProjection::Eac => SphericalProjection::Panini,
        SphericalProjection::Panini => SphericalProjection::Cylindrical,
        SphericalProjection::Cylindrical => SphericalProjection::Mercator,
        SphericalProjection::Mercator => SphericalProjection::DualFisheyeTb,
        SphericalProjection::DualFisheyeTb => SphericalProjection::Octahedral,
        SphericalProjection::Octahedral => SphericalProjection::Equisolid,
        SphericalProjection::Equisolid => SphericalProjection::Orthographic,
        SphericalProjection::Orthographic => SphericalProjection::Gnomonic,
        SphericalProjection::Gnomonic => SphericalProjection::Sinusoidal,
        SphericalProjection::Sinusoidal => SphericalProjection::Miller,
        SphericalProjection::Miller => SphericalProjection::AzimuthalEquidistant,
        SphericalProjection::AzimuthalEquidistant => SphericalProjection::Equirect,
    }
}

pub fn spherical_projection_label(mode: SphericalProjection) -> &'static str {
    match mode {
        SphericalProjection::Equirect => "Equirect",
        SphericalProjection::DualFisheye => "Dual fisheye",
        SphericalProjection::Cubemap => "Cubemap",
        SphericalProjection::LittlePlanet => "Little planet",
        SphericalProjection::Eac => "EAC",
        SphericalProjection::Panini => "Panini",
        SphericalProjection::Cylindrical => "Cylindrical",
        SphericalProjection::Mercator => "Mercator",
        SphericalProjection::DualFisheyeTb => "Dual fisheye TB",
        SphericalProjection::Octahedral => "Octahedral",
        SphericalProjection::Equisolid => "Equisolid",
        SphericalProjection::Orthographic => "Orthographic",
        SphericalProjection::Gnomonic => "Gnomonic",
        SphericalProjection::Sinusoidal => "Sinusoidal",
        SphericalProjection::Miller => "Miller",
        SphericalProjection::AzimuthalEquidistant => "Azimuthal EQ",
    }
}

pub fn format_spherical_projection_osd(mode: SphericalProjection) -> String {
    format!("360° {}", spherical_projection_label(mode))
}

/// Remap dual-fisheye (side-by-side) into a temporary equirect for viewpoint projection.
pub fn dual_fisheye_to_equirect(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    if src_w < 2 || src_h == 0 || src.is_empty() {
        return out;
    }
    let eye_w = src_w / 2;
    for y in 0..out_h {
        let lat = (0.5 - (y as f32 + 0.5) / out_h as f32) * std::f32::consts::PI;
        for x in 0..out_w {
            let lon = ((x as f32 + 0.5) / out_w as f32 * 2.0 - 1.0) * std::f32::consts::PI;
            let use_right = lon >= 0.0;
            let eye_lon = if use_right {
                lon
            } else {
                lon + std::f32::consts::PI
            };
            let r = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0) * 0.5;
            let angle = eye_lon;
            let fx = 0.5 + r * angle.cos();
            let fy = 0.5 + r * angle.sin();
            let ox = ((fx.clamp(0.0, 1.0) * (eye_w.saturating_sub(1) as f32)).round() as u32)
                + if use_right { eye_w } else { 0 };
            let oy = (fy.clamp(0.0, 1.0) * (src_h.saturating_sub(1) as f32)).round() as u32;
            let idx = (oy as usize) * (src_w as usize) + (ox.min(src_w - 1) as usize);
            out[(y * out_w + x) as usize] = src.get(idx).copied().unwrap_or(0);
        }
    }
    out
}

/// Sample a horizontal cubemap strip (6 square faces: +X -X +Y -Y +Z -Z).
pub fn sample_cubemap_pixel(
    pixels: &[u32],
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
    dz: f32,
) -> u32 {
    let ax = dx.abs();
    let ay = dy.abs();
    let az = dz.abs();
    let (face, u, v) = if ax >= ay && ax >= az {
        if dx > 0.0 {
            (0u32, -dz / ax, -dy / ax)
        } else {
            (1, dz / ax, -dy / ax)
        }
    } else if ay >= ax && ay >= az {
        if dy > 0.0 {
            (2, dx / ay, dz / ay)
        } else {
            (3, dx / ay, -dz / ay)
        }
    } else if dz > 0.0 {
        (4, dx / az, -dy / az)
    } else {
        (5, -dx / az, -dy / az)
    };
    let face_w = (width / 6).max(1);
    let face_h = height.max(1);
    let uu = ((u * 0.5 + 0.5).clamp(0.0, 1.0) * (face_w.saturating_sub(1) as f32)).round() as u32;
    let vv = ((v * 0.5 + 0.5).clamp(0.0, 1.0) * (face_h.saturating_sub(1) as f32)).round() as u32;
    let x = face * face_w + uu.min(face_w - 1);
    let y = vv.min(face_h - 1);
    let idx = (y as usize) * (width as usize) + (x.min(width - 1) as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Sample EAC (equi-angular cubemap) strip using atan face UVs.
pub fn sample_eac_pixel(pixels: &[u32], width: u32, height: u32, dx: f32, dy: f32, dz: f32) -> u32 {
    let (face, u, v) = eac_face_uv_from_dir(dx, dy, dz);
    let face_w = (width / 6).max(1);
    let face_h = height.max(1);
    let uu = (u.clamp(0.0, 1.0) * (face_w.saturating_sub(1) as f32)).round() as u32;
    let vv = (v.clamp(0.0, 1.0) * (face_h.saturating_sub(1) as f32)).round() as u32;
    let x = u32::from(face) * face_w + uu.min(face_w - 1);
    let y = vv.min(face_h - 1);
    let idx = (y as usize) * (width as usize) + (x.min(width - 1) as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Stereographic "little planet" remap from equirectangular source.
pub fn project_little_planet(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    let cx = out_w as f32 * 0.5;
    let cy = out_h as f32 * 0.5;
    let radius = cx.min(cy).max(1.0);
    for y in 0..out_h {
        for x in 0..out_w {
            let dx = (x as f32 + 0.5) - cx;
            let dy = (y as f32 + 0.5) - cy;
            let r = (dx * dx + dy * dy).sqrt() / radius;
            let lon = dx.atan2(-dy) + yaw;
            let lat = (1.0 - r).clamp(-1.0, 1.0) * std::f32::consts::FRAC_PI_2;
            out[(y * out_w + x) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Project a spherical view using the selected source projection.
pub fn project_spherical_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    projection: SphericalProjection,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    match projection {
        SphericalProjection::Equirect => project_equirect_view_ex(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::DualFisheye => {
            let equirect = dual_fisheye_to_equirect(src_w, src_h, src, src_w.max(2), src_h.max(1));
            project_equirect_view_ex(
                src_w.max(2),
                src_h.max(1),
                &equirect,
                out_w,
                out_h,
                yaw_deg_milli,
                pitch_deg_milli,
                roll_deg_milli,
                fov_deg_milli,
            )
        }
        SphericalProjection::Cubemap => {
            // Reuse equirect camera rays, sampling cubemap faces instead.
            let out_w = out_w.max(1);
            let out_h = out_h.max(1);
            let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
            let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
            let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
            let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
            let aspect = out_w as f32 / out_h as f32;
            let tan_half = (fov * 0.5).tan();
            let (sin_y, cos_y) = yaw.sin_cos();
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_r, cos_r) = roll.sin_cos();
            let mut out = vec![0u32; out_w as usize * out_h as usize];
            for oy in 0..out_h {
                let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
                for ox in 0..out_w {
                    let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
                    let nx = nx0 * cos_r - ny0 * sin_r;
                    let ny = nx0 * sin_r + ny0 * cos_r;
                    let x1 = nx;
                    let y1 = ny * cos_p - 1.0 * sin_p;
                    let z1 = ny * sin_p + 1.0 * cos_p;
                    let x2 = x1 * cos_y + z1 * sin_y;
                    let y2 = y1;
                    let z2 = -x1 * sin_y + z1 * cos_y;
                    let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
                    out[(oy * out_w + ox) as usize] =
                        sample_cubemap_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
                }
            }
            out
        }
        SphericalProjection::Eac => {
            let out_w = out_w.max(1);
            let out_h = out_h.max(1);
            let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
            let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
            let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
            let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
            let aspect = out_w as f32 / out_h as f32;
            let tan_half = (fov * 0.5).tan();
            let (sin_y, cos_y) = yaw.sin_cos();
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_r, cos_r) = roll.sin_cos();
            let mut out = vec![0u32; out_w as usize * out_h as usize];
            for oy in 0..out_h {
                let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
                for ox in 0..out_w {
                    let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
                    let nx = nx0 * cos_r - ny0 * sin_r;
                    let ny = nx0 * sin_r + ny0 * cos_r;
                    let x1 = nx;
                    let y1 = ny * cos_p - 1.0 * sin_p;
                    let z1 = ny * sin_p + 1.0 * cos_p;
                    let x2 = x1 * cos_y + z1 * sin_y;
                    let y2 = y1;
                    let z2 = -x1 * sin_y + z1 * cos_y;
                    let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
                    out[(oy * out_w + ox) as usize] =
                        sample_eac_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
                }
            }
            out
        }
        SphericalProjection::LittlePlanet => {
            project_little_planet(src_w, src_h, src, out_w, out_h, yaw_deg_milli)
        }
        SphericalProjection::Panini => project_panini_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
            1_000,
        ),
        SphericalProjection::Cylindrical => project_cylindrical_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Mercator => project_mercator_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::DualFisheyeTb => {
            let equirect =
                dual_fisheye_tb_to_equirect(src_w, src_h, src, src_w.max(2), src_h.max(1));
            project_equirect_view_ex(
                src_w.max(2),
                src_h.max(1),
                &equirect,
                out_w,
                out_h,
                yaw_deg_milli,
                pitch_deg_milli,
                roll_deg_milli,
                fov_deg_milli,
            )
        }
        SphericalProjection::Octahedral => project_octahedral_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Equisolid => project_equisolid_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Orthographic => project_orthographic_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
        ),
        SphericalProjection::Gnomonic => project_gnomonic_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Sinusoidal => project_sinusoidal_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Miller => project_miller_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::AzimuthalEquidistant => project_azimuthal_equidistant_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
        ),
    }
}

/// Sample octahedral environment map.
pub fn sample_octahedral_pixel(
    pixels: &[u32],
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
    dz: f32,
) -> u32 {
    let len = (dx.abs() + dy.abs() + dz.abs()).max(1e-6);
    let mut x = dx / len;
    let mut y = dy / len;
    let z = dz / len;
    if z < 0.0 {
        let ox = x;
        x = (1.0 - y.abs()) * if ox >= 0.0 { 1.0 } else { -1.0 };
        y = (1.0 - ox.abs()) * if y >= 0.0 { 1.0 } else { -1.0 };
    }
    let u = x * 0.5 + 0.5;
    let v = y * 0.5 + 0.5;
    let px = ((u.clamp(0.0, 1.0) * (width.saturating_sub(1) as f32)).round() as u32)
        .min(width.saturating_sub(1));
    let py = ((v.clamp(0.0, 1.0) * (height.saturating_sub(1) as f32)).round() as u32)
        .min(height.saturating_sub(1));
    let idx = (py as usize) * (width as usize) + (px as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

pub fn project_octahedral_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let x1 = nx;
            let y1 = ny * cos_p - sin_p;
            let z1 = ny * sin_p + cos_p;
            let x2 = x1 * cos_y + z1 * sin_y;
            let y2 = y1;
            let z2 = -x1 * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
            out[(oy * out_w + ox) as usize] =
                sample_octahedral_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
        }
    }
    out
}

/// Equisolid fisheye viewport sampling equirect.
pub fn project_equisolid_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let r_max = 2.0 * (fov * 0.5).sin().max(1e-6);
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let r = (nx * nx + ny * ny).sqrt();
            if r > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let theta = 2.0 * ((r * r_max * 0.5).clamp(0.0, 1.0)).asin();
            let phi = ny.atan2(nx);
            let x_cam = theta.sin() * phi.cos();
            let y_cam = theta.sin() * phi.sin();
            let z_cam = theta.cos();
            // Apply yaw/pitch to camera forward
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_y, cos_y) = yaw.sin_cos();
            let y1 = y_cam * cos_p - z_cam * sin_p;
            let z1 = y_cam * sin_p + z_cam * cos_p;
            let x2 = x_cam * cos_y + z1 * sin_y;
            let z2 = -x_cam * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y1 * y1 + z2 * z2).sqrt().max(1e-6);
            let lon = (z2 / len).atan2(x2 / len);
            let lat = (y1 / len).asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Panini wide-FOV projection from equirect (d_milli = 1000 → classic d=1).
pub fn project_panini_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
    d_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let d = (d_milli.clamp(100, 5_000) as f32) / 1_000.0;
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let lon_off = (nx / (d + 1.0)).atan();
            let lat_off = (ny * (d + lon_off.cos()).max(0.05) / (d + 1.0)).atan();
            let lon = yaw + lon_off;
            let lat = (pitch + lat_off).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Cylindrical panorama projection from equirect.
pub fn project_cylindrical_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let lon = yaw + nx; // linear azimuth
            let lat = (pitch + ny.atan()).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// HDR10 static metadata peak luminance (MaxCLL / MaxFALL), nits.
pub fn clamp_hdr_maxcll(nits: u32) -> u32 {
    nits.min(10_000)
}

pub fn format_hdr_metadata_osd(maxcll: u32, maxfall: u32, color_trc: u32) -> String {
    let trc = if color_trc == COLOR_TRC_SMPTE2084 {
        "PQ"
    } else if color_trc == COLOR_TRC_HLG {
        "HLG"
    } else {
        "SDR"
    };
    format!(
        "HDR {trc} MaxCLL {} MaxFALL {}",
        clamp_hdr_maxcll(maxcll),
        clamp_hdr_maxcll(maxfall)
    )
}

pub const COLOR_PRIMARIES_BT709: u32 = 1;
pub const COLOR_PRIMARIES_BT2020: u32 = 9;

pub fn color_primaries_label(primaries: u32) -> &'static str {
    match primaries {
        COLOR_PRIMARIES_BT709 => "BT.709",
        COLOR_PRIMARIES_BT2020 => "BT.2020",
        _ => "Unspecified",
    }
}

pub fn format_color_primaries_osd(primaries: u32) -> String {
    format!("Color {}", color_primaries_label(primaries))
}

/// Skip forward over near-silence (podcast / lecture players).
pub fn silence_skip_target_us(
    now_us: i64,
    duration_us: i64,
    silence_spans: &[(i64, i64)],
    min_silence_us: i64,
) -> Option<i64> {
    let now_us = now_us.max(0);
    for &(start, end) in silence_spans {
        if end <= start || end - start < min_silence_us {
            continue;
        }
        if now_us >= start && now_us < end {
            return Some(end.min(duration_us.max(0)));
        }
    }
    None
}

pub fn format_silence_skip_osd(target_us: i64) -> String {
    format!("Skip silence → {}", format_play_clock(target_us))
}

/// Cycle among video streams (multi-angle / alternate camera).
pub fn cycle_video_track(count: u32, current: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    (current + 1) % count
}

pub fn format_video_track_osd(index: u32, count: u32) -> String {
    if count == 0 {
        "Video none".into()
    } else {
        format!("Video {}/{}", index + 1, count)
    }
}

/// Prefer hearing-impaired / SDH subtitle disposition.
pub fn prefer_hearing_impaired_subtitle_index(hi_flags: &[bool], current: usize) -> usize {
    if hi_flags.is_empty() {
        return current;
    }
    if let Some(idx) = hi_flags.iter().position(|flag| *flag) {
        return idx;
    }
    current.min(hi_flags.len() - 1)
}

pub fn format_hearing_impaired_osd(index: usize) -> String {
    format!("SDH subtitle #{}", index + 1)
}

/// Intro / credits skip markers (Netflix-style chapter helpers).
pub fn skip_marker_target_us(
    now_us: i64,
    intro_end_us: Option<i64>,
    credits_start_us: Option<i64>,
) -> Option<i64> {
    if let Some(end) = intro_end_us {
        if now_us < end {
            return Some(end.max(0));
        }
    }
    if let Some(start) = credits_start_us {
        if now_us < start {
            return Some(start.max(0));
        }
    }
    None
}

pub fn format_skip_marker_osd(kind: &str, target_us: i64) -> String {
    format!("Skip {kind} → {}", format_play_clock(target_us))
}

/// Case-insensitive playlist path filter (library / finder players).
pub fn filter_playlist_paths(paths: &[PathBuf], query: &str) -> Vec<PathBuf> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return paths.to_vec();
    }
    paths
        .iter()
        .filter(|path| path.to_string_lossy().to_ascii_lowercase().contains(&query))
        .cloned()
        .collect()
}

pub fn format_playlist_filter_osd(query: &str, matched: usize, total: usize) -> String {
    let query = query.trim();
    if query.is_empty() {
        format!("Filter off ({total})")
    } else {
        format!("Filter \"{query}\" {matched}/{total}")
    }
}

/// Favorites / starred media paths (capped).
pub const FAVORITES_MAX: usize = 256;

pub fn toggle_favorite(favorites: &mut Vec<PathBuf>, path: PathBuf) -> bool {
    if let Some(idx) = favorites.iter().position(|entry| entry == &path) {
        favorites.remove(idx);
        false
    } else {
        favorites.insert(0, path);
        if favorites.len() > FAVORITES_MAX {
            favorites.truncate(FAVORITES_MAX);
        }
        true
    }
}

pub fn format_favorite_osd(starred: bool) -> &'static str {
    if starred {
        "Favorite On"
    } else {
        "Favorite Off"
    }
}

/// Remote-control / HTTP interface enable flag (VLC `--extraintf=http` style).
pub fn format_remote_control_osd(enabled: bool, port: u16) -> String {
    if enabled {
        format!("RC http :{port}")
    } else {
        "RC Off".into()
    }
}

pub const REMOTE_CONTROL_DEFAULT_PORT: u16 = 8080;

/// Bit-perfect / exclusive output preference (WASAPI exclusive style).
pub fn format_bitperfect_osd(enabled: bool) -> &'static str {
    if enabled {
        "Bit-perfect On"
    } else {
        "Bit-perfect Off"
    }
}

/// Double-click fullscreen toggle helper.
pub fn should_toggle_fullscreen_on_click(click_count: u32, on_video: bool) -> bool {
    on_video && click_count >= 2
}

/// Scrub-preview media time from a normalized slider fraction.
pub fn scrub_preview_us(fraction: f32, duration_us: i64) -> i64 {
    media_us_from_fraction(fraction.clamp(0.0, 1.0), duration_us)
}

pub fn format_scrub_preview_osd(us: i64) -> String {
    format!("Preview {}", format_play_clock(us))
}

/// Night-mode / soft volume curve (compress loud peaks for late listening).
pub fn apply_night_mode_sample(sample: f32, enabled: bool) -> f32 {
    if !enabled {
        return sample.clamp(-1.0, 1.0);
    }
    compress_sample(sample, 0.25, 3.0)
}

pub fn format_night_mode_osd(enabled: bool) -> &'static str {
    if enabled {
        "Night mode On"
    } else {
        "Night mode Off"
    }
}

/// Frame-rate OSD from frame duration (VLC stats / Codec info).
pub fn frame_rate_milli_from_duration_us(frame_duration_us: i64) -> u32 {
    if frame_duration_us <= 0 {
        return 0;
    }
    ((1_000_000_000i64) / frame_duration_us.max(1)).clamp(0, 240_000) as u32
}

pub fn format_frame_rate_osd(frame_duration_us: i64) -> String {
    let milli = frame_rate_milli_from_duration_us(frame_duration_us);
    format!("FPS {:.3}", milli as f32 / 1_000.0)
}

/// Multi-angle selection (DVD-style angle index among video streams).
pub fn cycle_angle(count: u32, current: u32) -> u32 {
    cycle_video_track(count, current)
}

pub fn format_angle_osd(index: u32, count: u32) -> String {
    if count <= 1 {
        "Angle 1".into()
    } else {
        format!("Angle {}/{}", index + 1, count)
    }
}

/// Mouse-wheel seek when Ctrl is held (fine scrub).
pub fn seek_from_wheel(now_us: i64, scroll_lines: i32, step_us: i64) -> i64 {
    now_us
        .saturating_add(i64::from(scroll_lines) * step_us)
        .max(0)
}

/// Playlist queue insert (play next without reshuffling order).
pub fn queue_insert(queue: &mut Vec<usize>, index: usize, at_front: bool) {
    queue.retain(|item| *item != index);
    if at_front {
        queue.insert(0, index);
    } else {
        queue.push(index);
    }
}

pub fn format_queue_osd(len: usize) -> String {
    format!("Queue {len}")
}

/// Soft subtitle / forced-only display preference.
pub fn format_forced_only_osd(enabled: bool) -> &'static str {
    if enabled {
        "Forced subs only"
    } else {
        "All subs"
    }
}

/// Audio device exclusivity latency hint (ms) for bit-perfect paths.
pub fn clamp_exclusive_latency_ms(ms: u32) -> u32 {
    ms.clamp(1, 500)
}

pub fn format_exclusive_latency_osd(ms: u32) -> String {
    format!("Exclusive latency {} ms", clamp_exclusive_latency_ms(ms))
}

/// Chapter thumbnail time grid for scrubber UI.
pub fn chapter_thumbnail_times(chapters: &[i64], duration_us: i64, max_thumbs: usize) -> Vec<i64> {
    let max_thumbs = max_thumbs.max(1).min(64);
    if !chapters.is_empty() {
        return chapters.iter().copied().take(max_thumbs).collect();
    }
    if duration_us <= 0 {
        return Vec::new();
    }
    let step = duration_us / max_thumbs as i64;
    (0..max_thumbs)
        .map(|i| (i as i64 * step).clamp(0, duration_us))
        .collect()
}

pub fn format_chapter_thumbs_osd(count: usize) -> String {
    format!("Thumbs {count}")
}

/// VR Cardboard / headset interpupillary distance (mm ×1000).
pub const IPD_DEFAULT_MILLI: i32 = 63_000;
pub const IPD_MIN_MILLI: i32 = 50_000;
pub const IPD_MAX_MILLI: i32 = 80_000;
pub const IPD_STEP_MILLI: i32 = 1_000;

pub fn clamp_ipd_milli(value: i32) -> i32 {
    value.clamp(IPD_MIN_MILLI, IPD_MAX_MILLI)
}

pub fn ipd_step_milli(current: i32, delta: i32) -> i32 {
    clamp_ipd_milli(current.saturating_add(delta))
}

pub fn format_ipd_osd(ipd_milli: i32) -> String {
    format!("IPD {:.1} mm", clamp_ipd_milli(ipd_milli) as f32 / 1_000.0)
}

/// Stereo eye offset in yaw milli-degrees from IPD (Cardboard-style).
pub fn cardboard_eye_yaw_offset_milli(ipd_milli: i32, fov_deg_milli: i32) -> i32 {
    let ipd_mm = clamp_ipd_milli(ipd_milli) as f32 / 1_000.0;
    let fov_scale = (clamp_fov_milli(fov_deg_milli) as f32 / 90_000.0).clamp(0.5, 2.0);
    // ~0.05° yaw per mm of IPD at reference FOV, scaled by FOV.
    ((ipd_mm * 50.0 * fov_scale).round() as i32).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VrDisplayMode {
    #[default]
    Off,
    Cardboard,
    Mono,
}

pub fn cycle_vr_display(mode: VrDisplayMode) -> VrDisplayMode {
    match mode {
        VrDisplayMode::Off => VrDisplayMode::Cardboard,
        VrDisplayMode::Cardboard => VrDisplayMode::Mono,
        VrDisplayMode::Mono => VrDisplayMode::Off,
    }
}

pub fn vr_display_label(mode: VrDisplayMode) -> &'static str {
    match mode {
        VrDisplayMode::Off => "Off",
        VrDisplayMode::Cardboard => "Cardboard",
        VrDisplayMode::Mono => "Mono VR",
    }
}

pub fn format_vr_display_osd(mode: VrDisplayMode) -> String {
    format!("VR {}", vr_display_label(mode))
}

/// Ambisonic / binaural decode preference (360 audio players).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AmbisonicMode {
    #[default]
    Off,
    FirstOrder,
    Binaural,
}

pub fn cycle_ambisonic(mode: AmbisonicMode) -> AmbisonicMode {
    match mode {
        AmbisonicMode::Off => AmbisonicMode::FirstOrder,
        AmbisonicMode::FirstOrder => AmbisonicMode::Binaural,
        AmbisonicMode::Binaural => AmbisonicMode::Off,
    }
}

pub fn ambisonic_label(mode: AmbisonicMode) -> &'static str {
    match mode {
        AmbisonicMode::Off => "Off",
        AmbisonicMode::FirstOrder => "FOA",
        AmbisonicMode::Binaural => "Binaural",
    }
}

pub fn format_ambisonic_osd(mode: AmbisonicMode) -> String {
    format!("Ambisonic {}", ambisonic_label(mode))
}

/// Parse a WebVTT timestamp (`HH:MM:SS.mmm` or `MM:SS.mmm`) into media microseconds.
pub fn parse_webvtt_timestamp(spec: &str) -> Option<i64> {
    let spec = spec.trim();
    let (hms, frac) = spec.split_once('.').unwrap_or((spec, "0"));
    let parts: Vec<&str> = hms.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => (0i64, m.parse::<i64>().ok()?, s.parse::<i64>().ok()?),
        [h, m, s] => (
            h.parse::<i64>().ok()?,
            m.parse::<i64>().ok()?,
            s.parse::<i64>().ok()?,
        ),
        _ => return None,
    };
    let mut millis = frac.as_bytes().iter().take(3).fold(0i64, |acc, b| {
        if b.is_ascii_digit() {
            acc * 10 + i64::from(*b - b'0')
        } else {
            acc
        }
    });
    for _ in frac.len()..3 {
        millis *= 10;
    }
    Some(
        hours
            .saturating_mul(3_600_000_000)
            .saturating_add(minutes.saturating_mul(60_000_000))
            .saturating_add(seconds.saturating_mul(1_000_000))
            .saturating_add(millis.saturating_mul(1_000)),
    )
}

pub fn format_webvtt_timestamp(us: i64) -> String {
    let us = us.max(0);
    let total_ms = us / 1_000;
    let ms = total_ms % 1_000;
    let total_s = total_ms / 1_000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

/// Cast / AirPlay / Chromecast session presence (oracle OSD).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CastProtocol {
    #[default]
    Off,
    Chromecast,
    AirPlay,
    Dlna,
}

pub fn cycle_cast_protocol(mode: CastProtocol) -> CastProtocol {
    match mode {
        CastProtocol::Off => CastProtocol::Chromecast,
        CastProtocol::Chromecast => CastProtocol::AirPlay,
        CastProtocol::AirPlay => CastProtocol::Dlna,
        CastProtocol::Dlna => CastProtocol::Off,
    }
}

pub fn cast_protocol_label(mode: CastProtocol) -> &'static str {
    match mode {
        CastProtocol::Off => "Off",
        CastProtocol::Chromecast => "Chromecast",
        CastProtocol::AirPlay => "AirPlay",
        CastProtocol::Dlna => "DLNA",
    }
}

pub fn format_cast_osd(mode: CastProtocol, device: &str) -> String {
    let device = device.trim();
    if matches!(mode, CastProtocol::Off) {
        "Cast Off".into()
    } else if device.is_empty() {
        format!("Cast {}", cast_protocol_label(mode))
    } else {
        format!("Cast {} → {device}", cast_protocol_label(mode))
    }
}

/// Media library folder scan: collect playable extensions under a root (non-recursive oracle).
pub fn media_library_entries(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in names {
        let path = root.join(name);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            ext.as_str(),
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4a" | "mp3" | "flac" | "ogg" | "opus"
        ) {
            out.push(path);
        }
    }
    out.sort();
    out
}

pub fn format_media_library_osd(count: usize) -> String {
    format!("Library {count} items")
}

/// Podcast chapter art / image URL presence.
pub fn format_chapter_art_osd(url: Option<&str>) -> String {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) if url.len() <= 48 => format!("Chapter art {url}"),
        Some(url) => format!("Chapter art {}…", &url[..48]),
        None => "Chapter art none".into(),
    }
}

/// SMIL / soft playlist entry (src + begin offset).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmilClip {
    pub src: String,
    pub begin_us: i64,
}

pub fn parse_smil_clip_line(line: &str) -> Option<SmilClip> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // Minimal: `src=foo.mp4 begin=12.5` or `clip.mp4@12.5`
    if let Some((src, begin)) = line.split_once('@') {
        let secs: f64 = begin.trim().parse().ok()?;
        return Some(SmilClip {
            src: src.trim().to_string(),
            begin_us: (secs * 1_000_000.0).round() as i64,
        });
    }
    let mut src = None;
    let mut begin_us = 0i64;
    for part in line.split_whitespace() {
        if let Some(value) = part.strip_prefix("src=") {
            src = Some(value.trim_matches('"').to_string());
        } else if let Some(value) = part.strip_prefix("begin=") {
            let secs: f64 = value.parse().ok()?;
            begin_us = (secs * 1_000_000.0).round() as i64;
        }
    }
    Some(SmilClip {
        src: src?,
        begin_us,
    })
}

pub fn format_smil_clip_osd(clip: &SmilClip) -> String {
    format!("SMIL {} @ {}", clip.src, format_play_clock(clip.begin_us))
}

/// Named bookmark helper (title + time).
pub fn format_named_bookmark_osd(title: &str, media_us: i64) -> String {
    format_bookmark_label(media_us, Some(title))
}

/// HDR mastering display luminance range (cd/m²), oracle for HDR10 metadata panels.
pub fn format_hdr_mastering_osd(min_nits_milli: u32, max_nits: u32) -> String {
    format!(
        "Mastering {:.4}–{} nits",
        min_nits_milli as f32 / 1_000.0,
        clamp_hdr_maxcll(max_nits)
    )
}

/// BT.2020 vs BT.709 gamut clip warning for SDR displays.
pub fn hdr_gamut_warning(color_primaries: u32, display_is_bt709: bool) -> bool {
    display_is_bt709 && color_primaries == COLOR_PRIMARIES_BT2020
}

pub fn format_hdr_gamut_osd(warn: bool) -> &'static str {
    if warn {
        "Gamut BT.2020→709"
    } else {
        "Gamut OK"
    }
}

/// 360° stereo SBS cardboard layout size for dual-eye render.
pub fn cardboard_eye_rect(canvas_w: u32, canvas_h: u32, eye: u32) -> (u32, u32, u32, u32) {
    let half = (canvas_w / 2).max(1);
    let x = if eye == 0 { 0 } else { half };
    (x, 0, half, canvas_h.max(1))
}

/// Apply Cardboard eye yaw offset to a base yaw.
pub fn cardboard_view_yaw_milli(
    base_yaw_milli: i32,
    ipd_milli: i32,
    fov_deg_milli: i32,
    left_eye: bool,
) -> i32 {
    let offset = cardboard_eye_yaw_offset_milli(ipd_milli, fov_deg_milli);
    if left_eye {
        clamp_yaw_milli(base_yaw_milli.saturating_sub(offset))
    } else {
        clamp_yaw_milli(base_yaw_milli.saturating_add(offset))
    }
}

/// Lyrics / timed text line for karaoke / music video players.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    pub start_us: i64,
    pub text: String,
}

pub fn active_lyric_line<'a>(lines: &'a [LyricLine], now_us: i64) -> Option<&'a LyricLine> {
    let mut best = None;
    for line in lines {
        if line.start_us <= now_us {
            best = Some(line);
        } else {
            break;
        }
    }
    best
}

pub fn format_lyric_osd(line: Option<&LyricLine>) -> String {
    match line {
        Some(line) if !line.text.trim().is_empty() => format!("♪ {}", line.text.trim()),
        _ => "♪".into(),
    }
}

/// A-B loop memory slots (multi-marker players).
pub fn ab_slot_store(
    slots: &mut [(Option<i64>, Option<i64>)],
    index: usize,
    a: i64,
    b: i64,
) -> bool {
    if index >= slots.len() || b <= a {
        return false;
    }
    slots[index] = (Some(a.max(0)), Some(b.max(0)));
    true
}

pub fn ab_slot_load(slots: &[(Option<i64>, Option<i64>)], index: usize) -> Option<(i64, i64)> {
    slots.get(index).and_then(|(a, b)| Some(((*a)?, (*b)?)))
}

pub fn format_ab_slot_osd(index: usize, pair: Option<(i64, i64)>) -> String {
    match pair {
        Some((a, b)) => format!(
            "A-B slot {} {}–{}",
            index + 1,
            format_play_clock(a),
            format_play_clock(b)
        ),
        None => format!("A-B slot {} empty", index + 1),
    }
}

/// Playback statistics export line (CSV-ish) for analytics players.
pub fn format_play_stats_csv(
    presented: u64,
    skipped: u64,
    duration_us: i64,
    rate_milli: u32,
) -> String {
    format!(
        "presented={presented},skipped={skipped},duration_us={duration_us},rate_milli={rate_milli}"
    )
}

/// Film-grain / deband strength for HDR delivery (VLC video filter style).
pub const DEBAND_DEFAULT_MILLI: i32 = 0;
pub const DEBAND_MAX_MILLI: i32 = 1_000;

pub fn clamp_deband_milli(value: i32) -> i32 {
    value.clamp(0, DEBAND_MAX_MILLI)
}

pub fn apply_deband_pixel(
    red: u8,
    green: u8,
    blue: u8,
    strength_milli: i32,
    x: u32,
    y: u32,
) -> (u8, u8, u8) {
    let strength = clamp_deband_milli(strength_milli);
    if strength == 0 {
        return (red, green, blue);
    }
    let mut hash = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
    hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
    let noise = ((hash >> 24) as i32) - 128;
    let delta = noise * strength / 8_000;
    (
        (i32::from(red) + delta).clamp(0, 255) as u8,
        (i32::from(green) + delta).clamp(0, 255) as u8,
        (i32::from(blue) + delta).clamp(0, 255) as u8,
    )
}

pub fn format_deband_osd(strength_milli: i32) -> String {
    if clamp_deband_milli(strength_milli) == 0 {
        "Deband Off".into()
    } else {
        format!("Deband {}%", clamp_deband_milli(strength_milli) / 10)
    }
}

/// Parse HDR MaxCLL/MaxFALL pair from CLI (`1000,400`).
pub fn parse_hdr_maxcll_maxfall(spec: &str) -> Result<(u32, u32)> {
    let (a, b) = spec
        .split_once(',')
        .ok_or_else(|| format!("expected MaxCLL,MaxFALL got `{spec}`"))?;
    let maxcll: u32 = a
        .trim()
        .parse()
        .map_err(|_| format!("invalid MaxCLL `{a}`"))?;
    let maxfall: u32 = b
        .trim()
        .parse()
        .map_err(|_| format!("invalid MaxFALL `{b}`"))?;
    Ok((clamp_hdr_maxcll(maxcll), clamp_hdr_maxcll(maxfall)))
}

/// Dolby Vision profile tag presence (oracle; full DV bitstream remains OOS).
pub fn format_dolby_vision_osd(profile: Option<u32>) -> String {
    match profile {
        Some(p) => format!("Dolby Vision profile {p}"),
        None => "Dolby Vision Off".into(),
    }
}

/// Snapshot WYSIWYG includes OSD overlay flag.
pub fn format_snapshot_with_osd(enabled: bool) -> &'static str {
    if enabled {
        "Snapshot with OSD"
    } else {
        "Snapshot video only"
    }
}

/// Loop filter for still-image / GIF-style playlists (count + delay).
pub fn image_loop_remaining(count: u32, played: u32) -> Option<u32> {
    if count == 0 {
        return None; // infinite
    }
    if played >= count {
        Some(0)
    } else {
        Some(count - played)
    }
}

pub fn format_image_loop_osd(count: u32, played: u32) -> String {
    match image_loop_remaining(count, played) {
        None => format!("Image loop ∞ ({played})"),
        Some(0) => "Image loop done".into(),
        Some(left) => format!("Image loop {left} left"),
    }
}

/// CMX/EDL-style cut list entry for multi-clip play (VLC / NLEs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdlClip {
    pub src: String,
    pub in_us: i64,
    pub out_us: i64,
}

pub fn parse_edl_line(line: &str) -> Option<EdlClip> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
        return None;
    }
    // `title.mp4 10.0 25.5` or `title.mp4 in=10 out=25.5`
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 3 && !parts[1].contains('=') {
        let in_us = (parts[1].parse::<f64>().ok()? * 1_000_000.0).round() as i64;
        let out_us = (parts[2].parse::<f64>().ok()? * 1_000_000.0).round() as i64;
        if out_us > in_us {
            return Some(EdlClip {
                src: parts[0].to_string(),
                in_us: in_us.max(0),
                out_us: out_us.max(0),
            });
        }
        return None;
    }
    let mut src = None;
    let mut in_us = 0i64;
    let mut out_us = 0i64;
    for part in parts {
        if let Some(v) = part.strip_prefix("in=") {
            in_us = (v.parse::<f64>().ok()? * 1_000_000.0).round() as i64;
        } else if let Some(v) = part.strip_prefix("out=") {
            out_us = (v.parse::<f64>().ok()? * 1_000_000.0).round() as i64;
        } else if !part.contains('=') {
            src = Some(part.to_string());
        }
    }
    let src = src?;
    if out_us > in_us {
        Some(EdlClip {
            src,
            in_us: in_us.max(0),
            out_us: out_us.max(0),
        })
    } else {
        None
    }
}

pub fn format_edl_clip_osd(clip: &EdlClip) -> String {
    format!(
        "EDL {} {}–{}",
        clip.src,
        format_play_clock(clip.in_us),
        format_play_clock(clip.out_us)
    )
}

/// Picture-in-picture overlay rectangle (secondary video players).
pub fn pip_rect(
    canvas_w: u32,
    canvas_h: u32,
    pip_w: u32,
    pip_h: u32,
    margin: u32,
    bottom_right: bool,
) -> (u32, u32, u32, u32) {
    let pip_w = pip_w.min(canvas_w.max(1)).max(1);
    let pip_h = pip_h.min(canvas_h.max(1)).max(1);
    let margin = margin.min(canvas_w.min(canvas_h) / 2);
    let x = if bottom_right {
        canvas_w.saturating_sub(pip_w).saturating_sub(margin)
    } else {
        margin
    };
    let y = if bottom_right {
        canvas_h.saturating_sub(pip_h).saturating_sub(margin)
    } else {
        margin
    };
    (x, y, pip_w, pip_h)
}

pub fn format_pip_osd(enabled: bool) -> &'static str {
    if enabled { "PiP On" } else { "PiP Off" }
}

/// Thumbnail-strip seek: map hover index to media time.
pub fn thumbnail_seek_us(index: usize, count: usize, duration_us: i64) -> i64 {
    if count == 0 || duration_us <= 0 {
        return 0;
    }
    let count = count as i64;
    let index = (index as i64).clamp(0, count - 1);
    (duration_us.saturating_mul(index) / count).clamp(0, duration_us)
}

pub fn format_thumbnail_seek_osd(us: i64) -> String {
    format!("Thumb {}", format_play_clock(us))
}

/// Horizontal drag gesture → seek delta (px → microseconds).
pub fn seek_from_drag_px(dx_px: i32, px_per_second: i32) -> i64 {
    let pps = px_per_second.max(1) as i64;
    i64::from(dx_px).saturating_mul(1_000_000) / pps
}

pub fn format_drag_seek_osd(delta_us: i64) -> String {
    let sign = if delta_us < 0 { "-" } else { "+" };
    format!("Drag {sign}{}", format_play_clock(delta_us.abs()))
}

/// Interactive crop box from drag corners (normalized 0..=1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CropBoxMilli {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

pub fn clamp_crop_box_milli(box_: CropBoxMilli) -> CropBoxMilli {
    let x0 = box_.x0.clamp(0, 1_000);
    let y0 = box_.y0.clamp(0, 1_000);
    let x1 = box_.x1.clamp(0, 1_000).max(x0 + 1).min(1_000);
    let y1 = box_.y1.clamp(0, 1_000).max(y0 + 1).min(1_000);
    CropBoxMilli { x0, y0, x1, y1 }
}

pub fn crop_box_to_pixels(box_: CropBoxMilli, width: u32, height: u32) -> CropPixels {
    let box_ = clamp_crop_box_milli(box_);
    let w = width.max(1) as i64;
    let h = height.max(1) as i64;
    let left = (w * i64::from(box_.x0) / 1_000) as u32;
    let top = (h * i64::from(box_.y0) / 1_000) as u32;
    let right_edge = (w * i64::from(box_.x1) / 1_000) as u32;
    let bottom_edge = (h * i64::from(box_.y1) / 1_000) as u32;
    CropPixels {
        left,
        top,
        right: width.saturating_sub(right_edge),
        bottom: height.saturating_sub(bottom_edge),
    }
}

pub fn format_crop_box_osd(box_: CropBoxMilli) -> String {
    let box_ = clamp_crop_box_milli(box_);
    format!(
        "Crop box {:.0}%×{:.0}%",
        (box_.x1 - box_.x0) as f32 / 10.0,
        (box_.y1 - box_.y0) as f32 / 10.0
    )
}

/// 360° horizon / zenith lock (stabilize pitch while looking around).
pub fn format_horizon_lock_osd(locked: bool) -> &'static str {
    if locked {
        "Horizon lock On"
    } else {
        "Horizon lock Off"
    }
}

pub fn locked_pitch_milli(current: i32, locked: bool, locked_value: i32) -> i32 {
    if locked {
        clamp_pitch_milli(locked_value)
    } else {
        clamp_pitch_milli(current)
    }
}

/// Estimate frame peak luminance (0..=1000 milli) for HDR auto-nits assist.
pub fn estimate_frame_peak_milli(pixels: &[u32], sample_stride: usize) -> u32 {
    if pixels.is_empty() {
        return 0;
    }
    let stride = sample_stride.max(1);
    let mut peak = 0u32;
    for pixel in pixels.iter().step_by(stride) {
        let r = (pixel >> 16) & 0xff;
        let g = (pixel >> 8) & 0xff;
        let b = pixel & 0xff;
        let y = (54 * r + 183 * g + 19 * b) / 256;
        peak = peak.max(y);
    }
    (peak * 1_000 / 255).min(1_000)
}

pub fn suggest_hdr_nits_from_peak(peak_milli: u32, base_nits: u32) -> u32 {
    let peak = peak_milli.min(1_000);
    let base = clamp_hdr_nits(base_nits).max(100);
    if peak < 200 {
        base
    } else if peak < 600 {
        clamp_hdr_nits(base.saturating_mul(2))
    } else {
        clamp_hdr_nits(base.saturating_mul(4))
    }
}

pub fn format_hdr_peak_osd(peak_milli: u32, suggested_nits: u32) -> String {
    format!("Frame peak {}% → {} nits", peak_milli / 10, suggested_nits)
}

/// HLS/DASH rendition label from bandwidth + resolution.
pub fn format_stream_rendition_osd(width: u32, height: u32, bandwidth_bps: u32) -> String {
    if width == 0 || height == 0 {
        format!("Rendition {} kbps", bandwidth_bps / 1_000)
    } else {
        format!("Rendition {width}×{height} {} kbps", bandwidth_bps / 1_000)
    }
}

/// Stereo packing inside a 360° sphere (YouTube VR / GoPro VR / Spatial Media).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SphericalStereoLayout {
    #[default]
    Mono,
    TopBottom,
    SideBySide,
}

pub fn cycle_spherical_stereo(layout: SphericalStereoLayout) -> SphericalStereoLayout {
    match layout {
        SphericalStereoLayout::Mono => SphericalStereoLayout::TopBottom,
        SphericalStereoLayout::TopBottom => SphericalStereoLayout::SideBySide,
        SphericalStereoLayout::SideBySide => SphericalStereoLayout::Mono,
    }
}

pub fn spherical_stereo_label(layout: SphericalStereoLayout) -> &'static str {
    match layout {
        SphericalStereoLayout::Mono => "Mono 360",
        SphericalStereoLayout::TopBottom => "TB 360",
        SphericalStereoLayout::SideBySide => "SBS 360",
    }
}

pub fn format_spherical_stereo_osd(layout: SphericalStereoLayout) -> String {
    format!("Spherical {}", spherical_stereo_label(layout))
}

/// UV rect (normalized 0..=1000) for left/right eye within a stereo 360 frame.
pub fn spherical_stereo_uv_rect(
    layout: SphericalStereoLayout,
    right_eye: bool,
) -> (i32, i32, i32, i32) {
    match layout {
        SphericalStereoLayout::Mono => (0, 0, 1_000, 1_000),
        SphericalStereoLayout::TopBottom => {
            if right_eye {
                (0, 500, 1_000, 1_000)
            } else {
                (0, 0, 1_000, 500)
            }
        }
        SphericalStereoLayout::SideBySide => {
            if right_eye {
                (500, 0, 1_000, 1_000)
            } else {
                (0, 0, 500, 1_000)
            }
        }
    }
}

/// Reset look-around to north / level horizon (VLC / YouTube VR recenter).
pub fn recenter_spherical_view() -> (i32, i32, i32) {
    (0, 0, 0)
}

pub fn format_recenter_osd() -> &'static str {
    "View recentered"
}

/// Compass heading from yaw (0° = North, clockwise).
pub fn compass_heading_deg(yaw_deg_milli: i32) -> u32 {
    let yaw = clamp_yaw_milli(yaw_deg_milli);
    let deg = ((yaw as i64).rem_euclid(360_000) / 1_000) as u32;
    deg % 360
}

pub fn format_compass_osd(yaw_deg_milli: i32) -> String {
    let deg = compass_heading_deg(yaw_deg_milli);
    let label = match deg {
        0..=22 | 338..=359 => "N",
        23..=67 => "NE",
        68..=112 => "E",
        113..=157 => "SE",
        158..=202 => "S",
        203..=247 => "SW",
        248..=292 => "W",
        _ => "NW",
    };
    format!("Compass {label} {deg}°")
}

/// Cardboard / VR lens barrel distortion: map output UV → sample UV (milli 0..=1000).
pub fn barrel_distort_uv_milli(u_milli: i32, v_milli: i32, k_milli: i32) -> (i32, i32) {
    let k = k_milli.clamp(0, 2_000) as f32 / 1_000.0;
    let u = (u_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let v = (v_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let r2 = u * u + v * v;
    let scale = 1.0 + k * r2;
    let ou = ((u * scale + 1.0) * 500.0).round() as i32;
    let ov = ((v * scale + 1.0) * 500.0).round() as i32;
    (ou.clamp(0, 1_000), ov.clamp(0, 1_000))
}

pub fn format_barrel_osd(k_milli: i32) -> String {
    if k_milli <= 0 {
        "Lens barrel Off".into()
    } else {
        format!("Lens barrel {:.2}", k_milli as f32 / 1_000.0)
    }
}

/// Blend strength for HDR tonemap toward identity (0 = off/passthrough intent, 1000 = full).
pub const TONEMAP_STRENGTH_DEFAULT_MILLI: i32 = 1_000;

pub fn clamp_tonemap_strength_milli(value: i32) -> i32 {
    value.clamp(0, 1_000)
}

pub fn blend_tonemap_channel(mapped: u8, original: u8, strength_milli: i32) -> u8 {
    let s = clamp_tonemap_strength_milli(strength_milli);
    if s >= 1_000 {
        return mapped;
    }
    if s <= 0 {
        return original;
    }
    let m = i32::from(mapped);
    let o = i32::from(original);
    ((m * s + o * (1_000 - s)) / 1_000).clamp(0, 255) as u8
}

pub fn format_tonemap_strength_osd(strength_milli: i32) -> String {
    format!(
        "Tonemap strength {}%",
        clamp_tonemap_strength_milli(strength_milli) / 10
    )
}

/// Desaturate highlights after HDR→SDR for display comfort (mpv-style).
pub fn apply_hdr_highlight_desat_pixel(
    red: u8,
    green: u8,
    blue: u8,
    amount_milli: i32,
) -> (u8, u8, u8) {
    let amount = amount_milli.clamp(0, 1_000);
    if amount == 0 {
        return (red, green, blue);
    }
    let y = (54 * u32::from(red) + 183 * u32::from(green) + 19 * u32::from(blue)) / 256;
    let highlight = y.saturating_sub(160).min(95) as i32;
    if highlight == 0 {
        return (red, green, blue);
    }
    let t = amount * highlight / 95;
    let lerp = |c: u8| -> u8 {
        let c = i32::from(c);
        let y = y as i32;
        ((c * (1_000 - t) + y * t) / 1_000).clamp(0, 255) as u8
    };
    (lerp(red), lerp(green), lerp(blue))
}

pub fn format_hdr_highlight_desat_osd(amount_milli: i32) -> String {
    if amount_milli <= 0 {
        "HDR highlight desat Off".into()
    } else {
        format!("HDR highlight desat {}%", amount_milli.clamp(0, 1_000) / 10)
    }
}

/// Parse mastering display luminance pair (`0.005,1000` → milli-min, max nits).
pub fn parse_hdr_mastering_nits(spec: &str) -> Result<(u32, u32)> {
    let (a, b) = spec
        .split_once(',')
        .ok_or_else(|| format!("expected min,max nits got `{spec}`"))?;
    let min_f: f64 = a
        .trim()
        .parse()
        .map_err(|_| format!("invalid min nits `{a}`"))?;
    let max_nits: u32 = b
        .trim()
        .parse()
        .map_err(|_| format!("invalid max nits `{b}`"))?;
    if !(0.0..100.0).contains(&min_f) {
        return Err(format!("min nits out of range `{a}`"));
    }
    let min_milli = (min_f * 1_000.0).round().clamp(0.0, 100_000.0) as u32;
    Ok((min_milli, clamp_hdr_maxcll(max_nits)))
}

/// White-balance / color temperature shift in Kelvin (6500 = daylight / identity).
pub const COLOR_TEMP_DAYLIGHT_K: i32 = 6_500;

pub fn clamp_color_temp_kelvin(value: i32) -> i32 {
    value.clamp(2_000, 12_000)
}

pub fn apply_white_balance_pixel(red: u8, green: u8, blue: u8, kelvin: i32) -> (u8, u8, u8) {
    let k = clamp_color_temp_kelvin(kelvin);
    // Coarse Planckian tilt: warmer boosts R, cooler boosts B.
    let delta = (k - COLOR_TEMP_DAYLIGHT_K) as f32 / 4_000.0;
    let r_mul = (1.0 - delta * 0.35).clamp(0.55, 1.45);
    let b_mul = (1.0 + delta * 0.35).clamp(0.55, 1.45);
    (
        (f32::from(red) * r_mul).round().clamp(0.0, 255.0) as u8,
        green,
        (f32::from(blue) * b_mul).round().clamp(0.0, 255.0) as u8,
    )
}

pub fn format_color_temp_osd(kelvin: i32) -> String {
    format!("{} K", clamp_color_temp_kelvin(kelvin))
}

/// Detect letterbox / pillarbox black bars (mean luma threshold).
pub fn detect_letterbox_bars(
    width: u32,
    height: u32,
    pixels: &[u32],
    luma_threshold: u8,
) -> (u32, u32, u32, u32) {
    if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        return (0, 0, 0, 0);
    }
    let thr = u32::from(luma_threshold);
    let row_dark = |y: u32| -> bool {
        let mut sum = 0u64;
        let row = (y * width) as usize;
        for x in 0..width as usize {
            let p = pixels[row + x];
            let r = (p >> 16) & 0xff;
            let g = (p >> 8) & 0xff;
            let b = p & 0xff;
            sum += (54 * r + 183 * g + 19 * b) as u64 / 256;
        }
        (sum / u64::from(width.max(1))) <= u64::from(thr)
    };
    let col_dark = |x: u32| -> bool {
        let mut sum = 0u64;
        for y in 0..height as usize {
            let p = pixels[y * width as usize + x as usize];
            let r = (p >> 16) & 0xff;
            let g = (p >> 8) & 0xff;
            let b = p & 0xff;
            sum += (54 * r + 183 * g + 19 * b) as u64 / 256;
        }
        (sum / u64::from(height.max(1))) <= u64::from(thr)
    };
    let mut top = 0u32;
    while top < height / 3 && row_dark(top) {
        top += 1;
    }
    let mut bottom = 0u32;
    while bottom < height / 3 && row_dark(height - 1 - bottom) {
        bottom += 1;
    }
    let mut left = 0u32;
    while left < width / 3 && col_dark(left) {
        left += 1;
    }
    let mut right = 0u32;
    while right < width / 3 && col_dark(width - 1 - right) {
        right += 1;
    }
    (left, top, right, bottom)
}

pub fn format_letterbox_osd(left: u32, top: u32, right: u32, bottom: u32) -> String {
    if left == 0 && top == 0 && right == 0 && bottom == 0 {
        "Letterbox none".into()
    } else {
        format!("Letterbox L{left} T{top} R{right} B{bottom}")
    }
}

/// Playlist edge fade envelope (1_000 = full; fades in/out near item boundaries).
pub fn playlist_edge_fade_gain_milli(position_us: i64, duration_us: i64, fade_us: i64) -> i32 {
    if duration_us <= 0 || fade_us <= 0 {
        return 1_000;
    }
    let fade = fade_us.min(duration_us / 2).max(1);
    let pos = position_us.clamp(0, duration_us);
    let fade_in = if pos < fade {
        ((pos * 1_000) / fade) as i32
    } else {
        1_000
    };
    let remaining = duration_us - pos;
    let fade_out = if remaining < fade {
        ((remaining * 1_000) / fade) as i32
    } else {
        1_000
    };
    fade_in.min(fade_out).clamp(0, 1_000)
}

pub fn format_playlist_fade_osd(fade_us: i64) -> String {
    if fade_us <= 0 {
        "Playlist fade Off".into()
    } else {
        format!("Playlist fade {}", format_play_clock(fade_us))
    }
}

/// Duck background when dialogue is present (voice-over / podcast players).
pub fn audio_duck_gain_milli(dialogue_active: bool, duck_milli: i32) -> i32 {
    if dialogue_active {
        duck_milli.clamp(50, 1_000)
    } else {
        1_000
    }
}

pub fn format_audio_duck_osd(enabled: bool, duck_milli: i32) -> String {
    if !enabled {
        "Audio duck Off".into()
    } else {
        format!("Audio duck {}%", duck_milli.clamp(50, 1_000) / 10)
    }
}

/// Luma waveform column fills (0..=1000) for scopes (mpv / Resolve-style).
pub fn waveform_column_fills(width: u32, height: u32, pixels: &[u32], columns: usize) -> Vec<i32> {
    let columns = columns.clamp(1, 256);
    let mut fills = vec![0i32; columns];
    if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        return fills;
    }
    let mut counts = vec![0u32; columns];
    let mut sums = vec![0u64; columns];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let col = x * columns / width as usize;
            let p = pixels[y * width as usize + x];
            let r = (p >> 16) & 0xff;
            let g = (p >> 8) & 0xff;
            let b = p & 0xff;
            let yv = (54 * r + 183 * g + 19 * b) / 256;
            sums[col] += u64::from(yv);
            counts[col] += 1;
        }
    }
    for i in 0..columns {
        if counts[i] > 0 {
            fills[i] = ((sums[i] * 1_000) / (u64::from(counts[i]) * 255)) as i32;
        }
    }
    fills
}

pub fn format_waveform_osd(enabled: bool) -> &'static str {
    if enabled {
        "Waveform On"
    } else {
        "Waveform Off"
    }
}

/// HDR10+ / dynamic metadata presence (oracle; full bitstream remains OOS).
pub fn format_hdr10_plus_osd(present: bool) -> &'static str {
    if present {
        "HDR10+ dynamic"
    } else {
        "HDR10+ Off"
    }
}

/// HLG OOTF display gamma for SDR preview (BT.2100 simplified).
pub fn hlg_ootf_channel(scene: f32, gamma: f32) -> f32 {
    let y = scene.clamp(0.0, 1.0);
    let g = gamma.clamp(1.0, 2.4);
    if y <= 0.5 {
        (3.0 * y * y).powf(g)
    } else {
        y.powf(g)
    }
    .clamp(0.0, 1.0)
}

pub fn apply_hlg_ootf_pixel(red: u8, green: u8, blue: u8, gamma_milli: i32) -> (u8, u8, u8) {
    let gamma = gamma_milli.clamp(1_000, 2_400) as f32 / 1_000.0;
    let map = |c: u8| -> u8 {
        let v = hlg_ootf_channel(f32::from(c) / 255.0, gamma);
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (map(red), map(green), map(blue))
}

pub fn format_hlg_ootf_osd(gamma_milli: i32) -> String {
    format!(
        "HLG OOTF γ{:.2}",
        gamma_milli.clamp(1_000, 2_400) as f32 / 1_000.0
    )
}

/// FOV presets for 360 / VR (narrow / cinema / wide / super-wide).
pub const FOV_PRESET_MILLI: [i32; 4] = [60_000, 90_000, 110_000, 140_000];

pub fn cycle_fov_preset_milli(current: i32) -> i32 {
    let cur = clamp_fov_milli(current);
    let mut best = 0usize;
    let mut best_dist = i32::MAX;
    for (i, &p) in FOV_PRESET_MILLI.iter().enumerate() {
        let d = (p - cur).abs();
        if d < best_dist {
            best_dist = d;
            best = i;
        }
    }
    FOV_PRESET_MILLI[(best + 1) % FOV_PRESET_MILLI.len()]
}

pub fn format_fov_preset_osd(fov_milli: i32) -> String {
    format!("FOV {:.0}°", clamp_fov_milli(fov_milli) as f32 / 1_000.0)
}

/// Parse spherical stereo layout CLI token.
pub fn parse_spherical_stereo(spec: &str) -> Result<SphericalStereoLayout> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "mono" | "off" => Ok(SphericalStereoLayout::Mono),
        "tb" | "top-bottom" | "over-under" => Ok(SphericalStereoLayout::TopBottom),
        "sbs" | "side-by-side" => Ok(SphericalStereoLayout::SideBySide),
        other => Err(format!(
            "spherical stereo expected mono|tb|sbs got `{other}`"
        )),
    }
}

/// Display vs content peak nits comparison for HDR headroom OSD.
pub fn format_hdr_headroom_osd(content_nits: u32, display_nits: u32) -> String {
    let content = clamp_hdr_nits(content_nits);
    let display = clamp_hdr_nits(display_nits);
    if display == 0 {
        format!("HDR headroom n/a (content {content})")
    } else if content <= display {
        format!("HDR headroom +{} nits", display - content)
    } else {
        format!("HDR clip −{} nits", content - display)
    }
}

/// SMPTE timecode from media time + fps milli (e.g. 24000 = 24.000 fps).
pub fn format_timecode_osd(position_us: i64, fps_milli: u32) -> String {
    let pos = position_us.max(0);
    let fps = fps_milli.max(1) as i64;
    let total_frames = (pos * fps) / 1_000_000_000;
    let fps_i = (fps_milli.max(1) / 1_000).max(1) as i64;
    let frames = total_frames % fps_i;
    let total_secs = total_frames / fps_i;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    format!("{hours:02}:{mins:02}:{secs:02}:{frames:02}")
}

/// Export chapter times as newline-separated clocks (VLC chapter list style).
pub fn format_chapter_list_export(chapters_us: &[i64]) -> String {
    chapters_us
        .iter()
        .map(|&t| format_play_clock(t.max(0)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prefer sidecar album/cover art next to media (cover.jpg / folder.png).
pub fn prefer_album_art_path(media: &Path, candidates: &[&str]) -> Option<String> {
    let parent = media.parent()?;
    for name in candidates {
        let path = parent.join(name);
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

pub fn format_album_art_osd(path: Option<&str>) -> String {
    match path {
        Some(p) => format!("Album art {p}"),
        None => "Album art none".into(),
    }
}

/// Simple 3×3 box denoise (VLC / PotPlayer video filter style).
pub fn apply_box_denoise_pixel(
    width: u32,
    height: u32,
    pixels: &[u32],
    x: u32,
    y: u32,
    strength_milli: i32,
) -> u32 {
    let strength = strength_milli.clamp(0, 1_000);
    if strength == 0 || width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        let idx = (y * width + x) as usize;
        return pixels.get(idx).copied().unwrap_or(0);
    }
    let mut r_sum = 0u32;
    let mut g_sum = 0u32;
    let mut b_sum = 0u32;
    let mut n = 0u32;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let xx = (x as i32 + dx).clamp(0, width as i32 - 1) as u32;
            let yy = (y as i32 + dy).clamp(0, height as i32 - 1) as u32;
            let p = pixels[(yy * width + xx) as usize];
            r_sum += (p >> 16) & 0xff;
            g_sum += (p >> 8) & 0xff;
            b_sum += p & 0xff;
            n += 1;
        }
    }
    let avg_r = r_sum / n.max(1);
    let avg_g = g_sum / n.max(1);
    let avg_b = b_sum / n.max(1);
    let src = pixels[(y * width + x) as usize];
    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;
    let blend =
        |a: u32, b: u32| -> u32 { (a * (1_000 - strength as u32) + b * strength as u32) / 1_000 };
    let r = blend(sr, avg_r);
    let g = blend(sg, avg_g);
    let b = blend(sb, avg_b);
    (r << 16) | (g << 8) | b
}

pub fn format_box_denoise_osd(strength_milli: i32) -> String {
    if strength_milli <= 0 {
        "Denoise Off".into()
    } else {
        format!("Denoise {}%", strength_milli.clamp(0, 1_000) / 10)
    }
}

/// Dialogue enhance: mild mid-band boost oracle (speech intelligibility).
pub fn apply_dialogue_enhance_sample(sample: f32, amount_milli: i32) -> f32 {
    let amount = amount_milli.clamp(0, 1_000) as f32 / 1_000.0;
    soft_clip_sample(sample * (1.0 + 0.45 * amount))
}

pub fn format_dialogue_enhance_osd(amount_milli: i32) -> String {
    if amount_milli <= 0 {
        "Dialogue enhance Off".into()
    } else {
        format!("Dialogue enhance {}%", amount_milli.clamp(0, 1_000) / 10)
    }
}

/// Vectorscope R–B quadrant occupancy counts (simplified color scope).
pub fn vectorscope_quadrant_counts(pixels: &[u32], sample_stride: usize) -> [u32; 4] {
    let mut counts = [0u32; 4];
    let stride = sample_stride.max(1);
    for p in pixels.iter().step_by(stride) {
        let r = ((*p >> 16) & 0xff) as i32;
        let b = (*p & 0xff) as i32;
        let cr = r - 128;
        let cb = b - 128;
        let idx = match (cr >= 0, cb >= 0) {
            (true, true) => 0,
            (false, true) => 1,
            (false, false) => 2,
            (true, false) => 3,
        };
        counts[idx] += 1;
    }
    counts
}

pub fn format_vectorscope_osd(enabled: bool) -> &'static str {
    if enabled {
        "Vectorscope On"
    } else {
        "Vectorscope Off"
    }
}

/// Touch / trackpad swipe → yaw delta for 360 look-around.
pub fn yaw_from_swipe_px(dx_px: i32, px_per_degree: i32) -> i32 {
    let ppd = px_per_degree.max(1);
    clamp_yaw_milli((dx_px.saturating_mul(1_000)) / ppd)
}

pub fn pitch_from_swipe_px(dy_px: i32, px_per_degree: i32) -> i32 {
    let ppd = px_per_degree.max(1);
    clamp_pitch_milli((-dy_px.saturating_mul(1_000)) / ppd)
}

/// Integrate device gyro rates (milli-deg/s) over dt_ms into look deltas.
pub fn gyro_look_delta_milli(yaw_rate_milli: i32, pitch_rate_milli: i32, dt_ms: i32) -> (i32, i32) {
    let dt = dt_ms.max(0);
    let yaw = (i64::from(yaw_rate_milli) * i64::from(dt) / 1_000) as i32;
    let pitch = (i64::from(pitch_rate_milli) * i64::from(dt) / 1_000) as i32;
    (yaw, pitch)
}

pub fn format_gyro_osd(enabled: bool) -> &'static str {
    if enabled {
        "Gyro look On"
    } else {
        "Gyro look Off"
    }
}

/// VR comfort vignette strength by eccentricity (0 at center → strength at edges).
pub fn vr_vignette_gain_milli(u_milli: i32, v_milli: i32, strength_milli: i32) -> i32 {
    let strength = strength_milli.clamp(0, 1_000);
    if strength == 0 {
        return 1_000;
    }
    let u = (u_milli.clamp(0, 1_000) - 500) as f32 / 500.0;
    let v = (v_milli.clamp(0, 1_000) - 500) as f32 / 500.0;
    let r = (u * u + v * v).sqrt().min(1.0);
    let darken = (r * strength as f32 / 1_000.0).clamp(0.0, 1.0);
    ((1.0 - darken) * 1_000.0).round() as i32
}

pub fn apply_vr_vignette_pixel(red: u8, green: u8, blue: u8, gain_milli: i32) -> (u8, u8, u8) {
    let g = gain_milli.clamp(0, 1_000) as u32;
    (
        (u32::from(red) * g / 1_000) as u8,
        (u32::from(green) * g / 1_000) as u8,
        (u32::from(blue) * g / 1_000) as u8,
    )
}

pub fn format_vr_vignette_osd(strength_milli: i32) -> String {
    if strength_milli <= 0 {
        "VR vignette Off".into()
    } else {
        format!("VR vignette {}%", strength_milli.clamp(0, 1_000) / 10)
    }
}

/// Equi-Angular Cubemap face index + local UV from direction (YouTube EAC).
pub fn eac_face_uv_from_dir(x: f32, y: f32, z: f32) -> (u8, f32, f32) {
    let ax = x.abs();
    let ay = y.abs();
    let az = z.abs();
    let (face, uc, vc, maxc) = if ax >= ay && ax >= az {
        if x > 0.0 {
            (0u8, -z, y, ax)
        } else {
            (1, z, y, ax)
        }
    } else if ay >= ax && ay >= az {
        if y > 0.0 {
            (2, x, -z, ay)
        } else {
            (3, x, z, ay)
        }
    } else if z > 0.0 {
        (4, x, y, az)
    } else {
        (5, -x, y, az)
    };
    let maxc = maxc.max(1e-6);
    // Equi-angular: atan mapping vs perspective
    let u = (uc.atan2(maxc) / std::f32::consts::FRAC_PI_4 + 1.0) * 0.5;
    let v = (vc.atan2(maxc) / std::f32::consts::FRAC_PI_4 + 1.0) * 0.5;
    (face, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

pub fn format_eac_face_osd(face: u8) -> String {
    let name = match face {
        0 => "+X",
        1 => "-X",
        2 => "+Y",
        3 => "-Y",
        4 => "+Z",
        _ => "-Z",
    };
    format!("EAC face {name}")
}

/// Lift HDR black level (pedestal) in milli (0 = crush blacks, 1000 = +full lift).
pub fn apply_hdr_black_lift_channel(value: u8, lift_milli: i32) -> u8 {
    let lift = lift_milli.clamp(0, 1_000);
    if lift == 0 {
        return value;
    }
    let floor = (255 * lift / 1_000) as u8;
    value.max(floor)
}

pub fn format_hdr_black_lift_osd(lift_milli: i32) -> String {
    if lift_milli <= 0 {
        "HDR black lift Off".into()
    } else {
        format!("HDR black lift {}%", lift_milli.clamp(0, 1_000) / 10)
    }
}

/// Unsharp / sharpen kernel weight (center vs neighbors).
pub fn apply_unsharp_pixel(
    width: u32,
    height: u32,
    pixels: &[u32],
    x: u32,
    y: u32,
    amount_milli: i32,
) -> u32 {
    let amount = amount_milli.clamp(-1_000, 1_000);
    if amount == 0 || width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        return pixels.get((y * width + x) as usize).copied().unwrap_or(0);
    }
    let src = pixels[(y * width + x) as usize];
    let blur = apply_box_denoise_pixel(width, height, pixels, x, y, 1_000);
    let sharpen = |s: u32, b: u32| -> u32 {
        let s = s as i32;
        let b = b as i32;
        let d = s - b;
        (s + d * amount / 1_000).clamp(0, 255) as u32
    };
    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;
    let br = (blur >> 16) & 0xff;
    let bg = (blur >> 8) & 0xff;
    let bb = blur & 0xff;
    (sharpen(sr, br) << 16) | (sharpen(sg, bg) << 8) | sharpen(sb, bb)
}

pub fn format_unsharp_osd(amount_milli: i32) -> String {
    if amount_milli == 0 {
        "Unsharp Off".into()
    } else {
        format!("Unsharp {}%", amount_milli.clamp(-1_000, 1_000) / 10)
    }
}

/// Icecast / Shoutcast stream title OSD.
pub fn format_icecast_metadata_osd(artist: Option<&str>, title: Option<&str>) -> String {
    match (
        artist.map(str::trim).filter(|s| !s.is_empty()),
        title.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(a), Some(t)) => format!("{a} — {t}"),
        (None, Some(t)) => t.to_string(),
        (Some(a), None) => a.to_string(),
        (None, None) => "Stream metadata none".into(),
    }
}

/// Media-key action labels (MPRIS / Windows media keys parity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKeyAction {
    PlayPause,
    Next,
    Previous,
    Stop,
    SeekForward,
    SeekBack,
}

pub fn media_key_label(action: MediaKeyAction) -> &'static str {
    match action {
        MediaKeyAction::PlayPause => "Play/Pause",
        MediaKeyAction::Next => "Next",
        MediaKeyAction::Previous => "Previous",
        MediaKeyAction::Stop => "Stop",
        MediaKeyAction::SeekForward => "Seek +",
        MediaKeyAction::SeekBack => "Seek −",
    }
}

pub fn format_media_key_osd(action: MediaKeyAction) -> String {
    format!("Media key {}", media_key_label(action))
}

/// `#EXTINF` title from M3U line (podcast / radio playlists).
pub fn parse_m3u_extinf_title(line: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix("#EXTINF:")?;
    let title = rest.split_once(',')?.1.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// Thumbnailer contact-sheet cell rect.
pub fn thumbnail_grid_rect(
    cols: u32,
    rows: u32,
    index: usize,
    canvas_w: u32,
    canvas_h: u32,
) -> (u32, u32, u32, u32) {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let cell_w = canvas_w / cols;
    let cell_h = canvas_h / rows;
    let i = index as u32;
    let col = i % cols;
    let row = (i / cols) % rows;
    (col * cell_w, row * cell_h, cell_w.max(1), cell_h.max(1))
}

pub fn format_thumbnail_grid_osd(cols: u32, rows: u32) -> String {
    format!("Thumbs {cols}×{rows}")
}

/// Clipboard snapshot mode OSD (copy frame without writing a file).
pub fn format_clipboard_snapshot_osd(enabled: bool) -> &'static str {
    if enabled {
        "Snapshot → clipboard"
    } else {
        "Snapshot → file"
    }
}

/// Live-edge lag / timeshift delay for DVR and HLS live windows.
pub fn timeshift_lag_us(live_edge_us: i64, playhead_us: i64) -> i64 {
    live_edge_us.saturating_sub(playhead_us).max(0)
}

pub fn format_live_edge_osd(lag_us: i64) -> String {
    if lag_us <= 0 {
        "Live".into()
    } else {
        format!("Timeshift −{}", format_play_clock(lag_us))
    }
}

/// Instant replay seek target (jump back by window, clamped to 0).
pub fn instant_replay_us(playhead_us: i64, window_us: i64) -> i64 {
    playhead_us.saturating_sub(window_us.max(0)).max(0)
}

pub fn format_instant_replay_osd(window_us: i64) -> String {
    format!("Replay −{}", format_play_clock(window_us.max(0)))
}

/// Stereo phase correlation (−1000..+1000; +1000 = identical L/R).
pub fn phase_correlation_milli(left: &[f32], right: &[f32]) -> i32 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0;
    }
    let mut dot = 0.0f64;
    let mut el = 0.0f64;
    let mut er = 0.0f64;
    for i in 0..n {
        let l = f64::from(left[i]);
        let r = f64::from(right[i]);
        dot += l * r;
        el += l * l;
        er += r * r;
    }
    let denom = (el * er).sqrt();
    if denom < 1e-12 {
        return 0;
    }
    ((dot / denom) * 1_000.0).round().clamp(-1_000.0, 1_000.0) as i32
}

pub fn format_phase_correlation_osd(corr_milli: i32) -> String {
    format!(
        "Phase {:.2}",
        corr_milli.clamp(-1_000, 1_000) as f32 / 1_000.0
    )
}

/// True-peak estimate from interleaved PCM (milli, 1000 = 0 dBFS).
pub fn true_peak_milli(samples: &[f32]) -> u32 {
    let mut peak = 0.0f32;
    for &s in samples {
        peak = peak.max(s.abs());
    }
    (peak * 1_000.0).round().clamp(0.0, 4_000.0) as u32
}

pub fn format_true_peak_osd(peak_milli: u32) -> String {
    if peak_milli == 0 {
        "True peak −∞".into()
    } else {
        let db = 20.0 * (peak_milli as f32 / 1_000.0).log10();
        format!("True peak {db:.1} dBTP")
    }
}

/// Dolby Atmos bed/object presence OSD (oracle; full Atmos decode OOS).
pub fn format_atmos_layout_osd(bed_channels: u32, objects: u32) -> String {
    if bed_channels == 0 && objects == 0 {
        "Atmos Off".into()
    } else {
        format!("Atmos bed {bed_channels} + {objects} objects")
    }
}

/// Content / parental rating label.
pub fn format_content_rating_osd(rating: Option<&str>) -> String {
    match rating.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => format!("Rating {r}"),
        None => "Rating none".into(),
    }
}

/// ASS/SSA forced margin override (VLC freetype).
pub fn ass_override_margin_px(base_px: i32, override_px: Option<i32>) -> i32 {
    override_px.unwrap_or(base_px).clamp(0, 4_000)
}

pub fn format_ass_override_osd(override_px: Option<i32>) -> String {
    match override_px {
        Some(px) => format!("ASS margin {px}px"),
        None => "ASS margin default".into(),
    }
}

/// Estimate stream bitrate from bytes over a window.
pub fn network_bandwidth_bps(bytes: u64, window_ms: u64) -> u64 {
    if window_ms == 0 {
        return 0;
    }
    bytes.saturating_mul(8_000) / window_ms
}

pub fn format_network_bandwidth_osd(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("Net {:.1} Mbps", bps as f64 / 1_000_000.0)
    } else {
        format!("Net {} kbps", bps / 1_000)
    }
}

/// Multi-room / watch-party clock offset compensation.
pub fn multi_room_sync_target_us(local_us: i64, peer_us: i64, offset_us: i64) -> i64 {
    let peer = peer_us.saturating_add(offset_us);
    // Nudge local halfway toward peer for gentle lock.
    local_us + (peer - local_us) / 2
}

pub fn format_watch_party_osd(enabled: bool, offset_us: i64) -> String {
    if !enabled {
        "Watch party Off".into()
    } else {
        let sign = if offset_us < 0 { "-" } else { "+" };
        format!(
            "Watch party sync {sign}{}",
            format_play_clock(offset_us.abs())
        )
    }
}

/// HDR vs SDR display ratio hint (content peak / display peak).
pub fn hdr_sdr_ratio_milli(content_nits: u32, display_nits: u32) -> u32 {
    let c = clamp_hdr_nits(content_nits).max(1);
    let d = clamp_hdr_nits(display_nits).max(1);
    ((c as u64 * 1_000) / d as u64).min(100_000) as u32
}

pub fn format_hdr_sdr_ratio_osd(ratio_milli: u32) -> String {
    format!("HDR/SDR ×{:.2}", ratio_milli as f32 / 1_000.0)
}

/// Auto-horizon from accelerometer pitch (milli-g on device Y).
pub fn accelerometer_horizon_pitch_milli(accel_y_milli_g: i32) -> i32 {
    let y = (accel_y_milli_g.clamp(-1_000, 1_000) as f32) / 1_000.0;
    let pitch_deg = y.clamp(-1.0, 1.0).asin().to_degrees();
    clamp_pitch_milli((pitch_deg * 1_000.0).round() as i32)
}

pub fn format_auto_horizon_osd(enabled: bool) -> &'static str {
    if enabled {
        "Auto horizon On"
    } else {
        "Auto horizon Off"
    }
}

/// Lateral chromatic aberration UV offsets for Cardboard / VR lenses (milli).
pub fn chromatic_aberration_uv_milli(
    u_milli: i32,
    v_milli: i32,
    amount_milli: i32,
) -> ((i32, i32), (i32, i32), (i32, i32)) {
    let amount = amount_milli.clamp(0, 1_000) as f32 / 1_000.0;
    let u = (u_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let v = (v_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let r = (u * u + v * v).sqrt();
    let shift = amount * r * 12.0; // up to ~12 milli-UV units at edge
    let to_milli = |x: f32, y: f32| -> (i32, i32) {
        (
            (((x + 1.0) * 500.0).round() as i32).clamp(0, 1_000),
            (((y + 1.0) * 500.0).round() as i32).clamp(0, 1_000),
        )
    };
    let ru = to_milli(u + shift * u.max(1e-3).signum(), v);
    let gu = to_milli(u, v);
    let bu = to_milli(u - shift * u.max(1e-3).signum(), v);
    (ru, gu, bu)
}

pub fn format_chromatic_aberration_osd(amount_milli: i32) -> String {
    if amount_milli <= 0 {
        "Chromatic aberr. Off".into()
    } else {
        format!("Chromatic aberr. {}%", amount_milli.clamp(0, 1_000) / 10)
    }
}

/// Ambisonic channel ordering / normalization (libambiX / Google Spatial Media).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AmbisonicChannelOrder {
    #[default]
    AcnSn3d,
    AcnN3d,
    Fuma,
}

pub fn cycle_ambisonic_order(order: AmbisonicChannelOrder) -> AmbisonicChannelOrder {
    match order {
        AmbisonicChannelOrder::AcnSn3d => AmbisonicChannelOrder::AcnN3d,
        AmbisonicChannelOrder::AcnN3d => AmbisonicChannelOrder::Fuma,
        AmbisonicChannelOrder::Fuma => AmbisonicChannelOrder::AcnSn3d,
    }
}

pub fn ambisonic_order_label(order: AmbisonicChannelOrder) -> &'static str {
    match order {
        AmbisonicChannelOrder::AcnSn3d => "ACN/SN3D",
        AmbisonicChannelOrder::AcnN3d => "ACN/N3D",
        AmbisonicChannelOrder::Fuma => "FuMa",
    }
}

pub fn format_ambisonic_order_osd(order: AmbisonicChannelOrder) -> String {
    format!("Ambisonic {}", ambisonic_order_label(order))
}

/// Soft-knee limiter (playback safety / broadcast players).
pub fn apply_soft_limiter_sample(sample: f32, threshold_milli: i32) -> f32 {
    let thr = (threshold_milli.clamp(100, 1_000) as f32) / 1_000.0;
    let x = sample;
    let ax = x.abs();
    if ax <= thr {
        return x;
    }
    let over = ax - thr;
    let limited = thr + over / (1.0 + over / (1.0 - thr).max(1e-3));
    soft_clip_sample(limited.copysign(x))
}

pub fn format_soft_limiter_osd(threshold_milli: i32) -> String {
    format!(
        "Limiter −{:.1} dBFS",
        -20.0 * (threshold_milli.clamp(100, 1_000) as f32 / 1_000.0).log10()
    )
}

/// Simple feedforward echo / delay (VLC audio filter style).
pub fn apply_echo_sample(sample: f32, delayed: f32, feedback_milli: i32) -> f32 {
    let fb = feedback_milli.clamp(0, 900) as f32 / 1_000.0;
    soft_clip_sample(sample + delayed * fb)
}

pub fn format_echo_osd(feedback_milli: i32) -> String {
    if feedback_milli <= 0 {
        "Echo Off".into()
    } else {
        format!("Echo {}%", feedback_milli.clamp(0, 900) / 10)
    }
}

/// One-pole low-pass (karaoke / voice / night-mode companion).
pub fn apply_lowpass_1pole(sample: f32, state: &mut f32, coeff_milli: i32) -> f32 {
    let a = (coeff_milli.clamp(1, 999) as f32) / 1_000.0;
    *state = *state + a * (sample - *state);
    *state
}

/// One-pole high-pass via DC blocker style.
pub fn apply_highpass_1pole(
    sample: f32,
    state: &mut f32,
    prev_in: &mut f32,
    coeff_milli: i32,
) -> f32 {
    let a = (coeff_milli.clamp(1, 999) as f32) / 1_000.0;
    let y = a * (*state + sample - *prev_in);
    *prev_in = sample;
    *state = y;
    y
}

pub fn format_tone_filter_osd(kind: &str, coeff_milli: i32) -> String {
    format!("{kind} {}%", coeff_milli.clamp(0, 1_000) / 10)
}

/// BS.1770-ish short-term loudness from a window of peak millis (oracle).
pub fn short_term_lufs_from_peaks(peaks_milli: &[u32]) -> i32 {
    if peaks_milli.is_empty() {
        return -700;
    }
    let mut sum = 0.0f64;
    for &p in peaks_milli {
        let linear = (p.max(1) as f64) / 1_000.0;
        sum += linear * linear;
    }
    let mean = sum / peaks_milli.len() as f64;
    let lufs = 10.0 * mean.log10() - 0.691; // rough K-weight offset
    (lufs * 10.0).round().clamp(-700.0, 0.0) as i32
}

/// Loudness range (LRA) from short-term LUFS samples (×10).
pub fn loudness_range_l_milli(short_term_lufs_x10: &[i32]) -> u32 {
    if short_term_lufs_x10.len() < 2 {
        return 0;
    }
    let mut sorted = short_term_lufs_x10.to_vec();
    sorted.sort_unstable();
    let lo = sorted[sorted.len() / 10];
    let hi = sorted[sorted.len().saturating_mul(9) / 10];
    hi.saturating_sub(lo).max(0) as u32
}

pub fn format_loudness_range_osd(lra_x10: u32) -> String {
    format!("LRA {:.1} LU", lra_x10 as f32 / 10.0)
}

/// Dubois anaglyph matrix (better than simple R/C for 3D players).
pub fn anaglyph_dubois(left: u32, right: u32) -> u32 {
    let lr = ((left >> 16) & 0xff) as f32;
    let lg = ((left >> 8) & 0xff) as f32;
    let lb = (left & 0xff) as f32;
    let rr = ((right >> 16) & 0xff) as f32;
    let rg = ((right >> 8) & 0xff) as f32;
    let rb = (right & 0xff) as f32;
    let r = (0.456 * lr + 0.500 * lg + 0.176 * lb - 0.043 * rr - 0.088 * rg - 0.002 * rb)
        .clamp(0.0, 255.0);
    let g = (-0.040 * lr - 0.038 * lg - 0.016 * lb + 0.378 * rr + 0.734 * rg - 0.018 * rb)
        .clamp(0.0, 255.0);
    let b = (-0.015 * lr - 0.021 * lg - 0.005 * lb - 0.072 * rr - 0.113 * rg + 1.226 * rb)
        .clamp(0.0, 255.0);
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

pub fn format_anaglyph_dubois_osd(enabled: bool) -> &'static str {
    if enabled {
        "Anaglyph Dubois"
    } else {
        "Anaglyph RC"
    }
}

/// Delogo / watermark cover rectangle fill (mean of border).
pub fn apply_delogo_rect(
    width: u32,
    height: u32,
    pixels: &mut [u32],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) {
    if width == 0 || height == 0 || pixels.len() < (width * height) as usize {
        return;
    }
    let x1 = x.min(width.saturating_sub(1));
    let y1 = y.min(height.saturating_sub(1));
    let x2 = (x + w.max(1)).min(width);
    let y2 = (y + h.max(1)).min(height);
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    // Sample average of outer ring.
    let mut sum = 0u64;
    let mut n = 0u64;
    for yy in y1..y2 {
        for xx in x1..x2 {
            if xx == x1 || yy == y1 || xx + 1 == x2 || yy + 1 == y2 {
                sum += u64::from(pixels[(yy * width + xx) as usize] & 0x00ff_ffff);
                n += 1;
            }
        }
    }
    let fill = if n > 0 { (sum / n) as u32 } else { 0 };
    for yy in y1..y2 {
        for xx in x1..x2 {
            pixels[(yy * width + xx) as usize] = fill;
        }
    }
}

pub fn format_delogo_osd(x: u32, y: u32, w: u32, h: u32) -> String {
    format!("Delogo {w}×{h} @{x},{y}")
}

/// Prefer OpenSubtitles-style sidecar beside media (`name.en.srt`).
pub fn prefer_opensubtitles_path(media: &Path, lang: &str) -> Option<String> {
    let stem = media.file_stem()?.to_str()?;
    let parent = media.parent()?;
    let lang = lang.trim();
    if lang.is_empty() {
        return None;
    }
    let candidate = parent.join(format!("{stem}.{lang}.srt"));
    if candidate.is_file() {
        Some(candidate.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// Smart playlist: keep paths whose extension is in `exts` (lowercase, no dot).
pub fn filter_playlist_by_extension(paths: &[String], exts: &[&str]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| {
            Path::new(p)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    let e = e.to_ascii_lowercase();
                    exts.iter().any(|want| e == *want)
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

pub fn format_smart_playlist_osd(kept: usize, total: usize) -> String {
    format!("Smart playlist {kept}/{total}")
}

/// Onset-based BPM estimate (oracle; uses peak gaps in ms).
pub fn detect_bpm_from_onset_gaps_ms(gaps_ms: &[u32]) -> u32 {
    if gaps_ms.is_empty() {
        return 0;
    }
    let mut sum = 0u64;
    for &g in gaps_ms {
        sum += u64::from(g.max(1));
    }
    let mean = sum / gaps_ms.len() as u64;
    if mean == 0 {
        return 0;
    }
    ((60_000 / mean) as u32).clamp(40, 240)
}

pub fn format_bpm_osd(bpm: u32) -> String {
    if bpm == 0 {
        "BPM —".into()
    } else {
        format!("BPM {bpm}")
    }
}

/// Haas / precedence delay for stereo widening (samples).
pub fn haas_delay_samples(sample_rate: u32, delay_ms_milli: i32) -> usize {
    let ms = delay_ms_milli.clamp(0, 40_000) as f32 / 1_000.0;
    ((sample_rate as f32) * ms / 1_000.0).round().max(0.0) as usize
}

pub fn format_haas_osd(delay_ms_milli: i32) -> String {
    format!(
        "Haas {:.1} ms",
        delay_ms_milli.clamp(0, 40_000) as f32 / 1_000.0
    )
}

/// Timestretch duration via atempo-style rate (VLC / mpv rubberband companion).
pub fn atempo_duration_us(src_us: i64, tempo_milli: i32) -> i64 {
    let tempo = tempo_milli.clamp(250, 4_000) as i64;
    src_us.saturating_mul(1_000) / tempo
}

pub fn format_atempo_osd(tempo_milli: i32) -> String {
    format!(
        "Atempo {:.2}×",
        tempo_milli.clamp(250, 4_000) as f32 / 1_000.0
    )
}

/// Lightweight chorus (modulated delay mix).
pub fn apply_chorus_sample(sample: f32, delayed: f32, depth_milli: i32) -> f32 {
    let depth = depth_milli.clamp(0, 1_000) as f32 / 1_000.0;
    soft_clip_sample(sample * (1.0 - 0.5 * depth) + delayed * 0.5 * depth)
}

pub fn format_chorus_osd(depth_milli: i32) -> String {
    if depth_milli <= 0 {
        "Chorus Off".into()
    } else {
        format!("Chorus {}%", depth_milli.clamp(0, 1_000) / 10)
    }
}

/// Comb-filter reverb tap mix.
pub fn apply_reverb_sample(sample: f32, tap1: f32, tap2: f32, wet_milli: i32) -> f32 {
    let wet = wet_milli.clamp(0, 1_000) as f32 / 1_000.0;
    let room = 0.5 * tap1 + 0.35 * tap2;
    soft_clip_sample(sample * (1.0 - wet) + room * wet)
}

pub fn format_reverb_osd(wet_milli: i32) -> String {
    if wet_milli <= 0 {
        "Reverb Off".into()
    } else {
        format!("Reverb {}%", wet_milli.clamp(0, 1_000) / 10)
    }
}

/// ASS/SSA forced style override string (font + size).
pub fn format_ass_force_style(font: &str, size_px: u32, primary_color: &str) -> String {
    let font = font.trim();
    let font = if font.is_empty() { "Arial" } else { font };
    let color = primary_color.trim();
    let color = if color.is_empty() {
        "&H00FFFFFF"
    } else {
        color
    };
    format!("FontName={font},FontSize={size_px},PrimaryColour={color}")
}

pub fn format_ass_force_style_osd(enabled: bool) -> &'static str {
    if enabled {
        "ASS force style On"
    } else {
        "ASS force style Off"
    }
}

/// BT.2446-ish HDR→SDR spline tonemap channel (broadcast).
pub fn bt2446_tonemap_channel(value: f32, peak: f32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    let p = peak.clamp(1.0, 10_000.0);
    let y = v * p;
    // Soft shoulder toward SDR 100 nits.
    let mapped = if y <= 100.0 {
        y / 100.0
    } else {
        let t = ((y - 100.0) / (p - 100.0).max(1.0)).clamp(0.0, 1.0);
        0.7 + 0.3 * (1.0 - (1.0 - t).powi(2))
    };
    mapped.clamp(0.0, 1.0)
}

pub fn apply_bt2446_tonemap_pixel(red: u8, green: u8, blue: u8, peak_nits: u32) -> (u8, u8, u8) {
    let peak = clamp_hdr_nits(peak_nits).max(100) as f32;
    let map = |c: u8| -> u8 {
        let v = bt2446_tonemap_channel(f32::from(c) / 255.0, peak);
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (map(red), map(green), map(blue))
}

pub fn format_bt2446_osd(enabled: bool) -> &'static str {
    if enabled {
        "BT.2446 tonemap"
    } else {
        "BT.2446 Off"
    }
}

/// Interactive 360 hotspot (yaw/pitch milli → hit test radius).
#[derive(Clone, Debug, PartialEq)]
pub struct SphericalHotspot {
    pub yaw_deg_milli: i32,
    pub pitch_deg_milli: i32,
    pub radius_deg_milli: i32,
    pub label: String,
}

pub fn spherical_hotspot_hit(
    hotspot: &SphericalHotspot,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
) -> bool {
    let dyaw = (clamp_yaw_milli(yaw_deg_milli) - clamp_yaw_milli(hotspot.yaw_deg_milli)).abs();
    let dyaw = dyaw.min(360_000 - dyaw);
    let dpitch =
        (clamp_pitch_milli(pitch_deg_milli) - clamp_pitch_milli(hotspot.pitch_deg_milli)).abs();
    let r = hotspot.radius_deg_milli.max(1);
    dyaw <= r && dpitch <= r
}

pub fn format_spherical_hotspot_osd(hotspot: &SphericalHotspot, hit: bool) -> String {
    if hit {
        format!("Hotspot {}", hotspot.label)
    } else {
        format!("Hotspot miss {}", hotspot.label)
    }
}

/// Parse WebVTT REGION settings line (`id:foo width:50%`).
pub fn parse_webvtt_region_id(line: &str) -> Option<String> {
    let line = line.trim();
    let rest = line
        .strip_prefix("REGION")
        .or_else(|| line.strip_prefix("Region"))?
        .trim();
    for part in rest.split_whitespace() {
        if let Some(id) = part.strip_prefix("id:") {
            let id = id.trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Thumbnail preview cache key (path + media time bucket).
pub fn thumbnail_cache_key(path: &str, media_us: i64, bucket_us: i64) -> String {
    let bucket = bucket_us.max(1);
    let slot = media_us.max(0) / bucket;
    format!("{path}@{slot}")
}

/// Integrated loudness estimate from short-term windows (×10 LUFS).
pub fn integrated_lufs_from_short_term(short_term_x10: &[i32]) -> i32 {
    if short_term_x10.is_empty() {
        return -700;
    }
    // Absolute gate ≈ −70 LUFS; relative gate ≈ −10 LU below ungated mean.
    let ungated: Vec<i32> = short_term_x10
        .iter()
        .copied()
        .filter(|&v| v > -700)
        .collect();
    if ungated.is_empty() {
        return -700;
    }
    let mean = ungated.iter().map(|&v| v as i64).sum::<i64>() / ungated.len() as i64;
    let gate = mean - 100;
    let gated: Vec<i32> = ungated
        .into_iter()
        .filter(|&v| (v as i64) >= gate)
        .collect();
    if gated.is_empty() {
        return mean as i32;
    }
    (gated.iter().map(|&v| v as i64).sum::<i64>() / gated.len() as i64) as i32
}

pub fn format_integrated_lufs_osd(lufs_x10: i32) -> String {
    format!("Integrated {:+.1} LUFS", lufs_x10 as f32 / 10.0)
}

/// Brown–Conrady radial distortion for Cardboard lens calibration (k1/k2 milli).
pub fn brown_conrady_uv_milli(
    u_milli: i32,
    v_milli: i32,
    k1_milli: i32,
    k2_milli: i32,
) -> (i32, i32) {
    let u = (u_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let v = (v_milli.clamp(0, 1_000) as f32 / 1_000.0) * 2.0 - 1.0;
    let k1 = k1_milli.clamp(-2_000, 2_000) as f32 / 1_000.0;
    let k2 = k2_milli.clamp(-2_000, 2_000) as f32 / 1_000.0;
    let r2 = u * u + v * v;
    let scale = 1.0 + k1 * r2 + k2 * r2 * r2;
    (
        (((u * scale + 1.0) * 500.0).round() as i32).clamp(0, 1_000),
        (((v * scale + 1.0) * 500.0).round() as i32).clamp(0, 1_000),
    )
}

pub fn format_lens_calibration_osd(k1_milli: i32, k2_milli: i32) -> String {
    format!(
        "Lens k1={:.2} k2={:.2}",
        k1_milli as f32 / 1_000.0,
        k2_milli as f32 / 1_000.0
    )
}

/// Approximate ICtCp intensity from RGB (HDR metering oracle).
pub fn ictcp_intensity_milli(red: u8, green: u8, blue: u8) -> u32 {
    let y = (54 * u32::from(red) + 183 * u32::from(green) + 19 * u32::from(blue)) / 256;
    // Lightly compress highlights toward PQ-ish code value.
    let t = y as f32 / 255.0;
    let i = (t.powf(0.45) * 1_000.0).round().clamp(0.0, 1_000.0) as u32;
    i
}

pub fn format_ictcp_osd(intensity_milli: u32) -> String {
    format!("ICtCp I {}%", intensity_milli.min(1_000) / 10)
}

/// ABR representation pick by available bandwidth (HLS/DASH players).
pub fn prefer_abr_rendition_index(bandwidths_bps: &[u32], available_bps: u32) -> usize {
    if bandwidths_bps.is_empty() {
        return 0;
    }
    let mut best: Option<usize> = None;
    for (i, &bw) in bandwidths_bps.iter().enumerate() {
        if bw <= available_bps {
            best = Some(match best {
                Some(j) if bandwidths_bps[j] >= bw => j,
                _ => i,
            });
        }
    }
    if let Some(i) = best {
        return i;
    }
    bandwidths_bps
        .iter()
        .enumerate()
        .min_by_key(|(_, bw)| *bw)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub fn format_abr_osd(index: usize, bandwidth_bps: u32) -> String {
    format!("ABR#{index} {} kbps", bandwidth_bps / 1_000)
}

/// YouTube/BIF-style storyboard tile index from media time.
pub fn storyboard_tile_index(media_us: i64, interval_us: i64, tile_count: usize) -> usize {
    if tile_count == 0 || interval_us <= 0 {
        return 0;
    }
    let idx = (media_us.max(0) / interval_us) as usize;
    idx.min(tile_count - 1)
}

pub fn format_storyboard_osd(index: usize, tile_count: usize) -> String {
    format!("Storyboard {}/{}", index + 1, tile_count.max(1))
}

/// Continue-watching progress (0..=1000 milli).
pub fn watch_progress_milli(position_us: i64, duration_us: i64) -> u32 {
    if duration_us <= 0 {
        return 0;
    }
    ((position_us.max(0) as i128 * 1_000) / duration_us as i128).clamp(0, 1_000) as u32
}

pub fn format_continue_watching_osd(progress_milli: u32) -> String {
    format!("Continue {}%", progress_milli.min(1_000) / 10)
}

/// Up-next / binge autoplay countdown.
pub fn up_next_should_start(remaining_us: i64, countdown_us: i64) -> bool {
    remaining_us <= countdown_us.max(0) && remaining_us >= 0
}

pub fn format_up_next_osd(title: &str, remaining_us: i64) -> String {
    format!(
        "Up next: {title} in {}",
        format_play_clock(remaining_us.max(0))
    )
}

/// Scrobble / Last.fm-style payload line.
pub fn format_scrobble_line(artist: &str, title: &str, duration_us: i64) -> String {
    format!(
        "scrobble\t{}\t{}\t{}",
        artist.trim(),
        title.trim(),
        duration_us.max(0) / 1_000_000
    )
}

/// Gaze dwell: hotspot activates after dwell_ms at hit.
pub fn gaze_dwell_triggered(hit: bool, dwell_ms: u64, required_ms: u64) -> bool {
    hit && dwell_ms >= required_ms.max(1)
}

pub fn format_gaze_dwell_osd(triggered: bool) -> &'static str {
    if triggered {
        "Gaze select"
    } else {
        "Gaze idle"
    }
}

/// VR controller ray hit test against hotspot angular radius.
pub fn controller_ray_hit(
    ray_yaw_milli: i32,
    ray_pitch_milli: i32,
    hotspot: &SphericalHotspot,
) -> bool {
    spherical_hotspot_hit(hotspot, ray_yaw_milli, ray_pitch_milli)
}

/// Parse TTML/DFXP clock (`HH:MM:SS.mmm` or `SS.mmm`).
pub fn parse_ttml_clock(spec: &str) -> Option<i64> {
    let spec = spec.trim();
    if let Some(us) = parse_play_clock(spec) {
        return Some(us);
    }
    // TTML frames form HH:MM:SS:FF — treat FF as centiseconds if 2 digits.
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() == 4 {
        let h: i64 = parts[0].parse().ok()?;
        let m: i64 = parts[1].parse().ok()?;
        let s: i64 = parts[2].parse().ok()?;
        let f: i64 = parts[3].parse().ok()?;
        return Some((((h * 60 + m) * 60 + s) * 1_000 + f * 10) * 1_000);
    }
    None
}

/// Live DVR window clamp: playhead must stay within [edge-window, edge].
pub fn clamp_dvr_playhead_us(playhead_us: i64, live_edge_us: i64, window_us: i64) -> i64 {
    let edge = live_edge_us.max(0);
    let start = edge.saturating_sub(window_us.max(0));
    playhead_us.clamp(start, edge)
}

pub fn format_dvr_window_osd(window_us: i64) -> String {
    format!("DVR window {}", format_play_clock(window_us.max(0)))
}

/// PQ OETF (inverse EOTF) for encoding display-linear → PQ code value.
pub fn pq_oetf(luminance: f32) -> f32 {
    let y = (luminance.max(0.0) / 100.0).clamp(0.0, 1.0);
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let ym = y.powf(m1);
    ((c1 + c2 * ym) / (1.0 + c3 * ym)).powf(m2).clamp(0.0, 1.0)
}

/// HLG OETF (scene-light → HLG signal).
pub fn hlg_oetf(scene: f32) -> f32 {
    let e = scene.clamp(0.0, 1.0);
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    if e <= 1.0 / 12.0 {
        (3.0 * e).sqrt().clamp(0.0, 1.0)
    } else {
        (a * (12.0 * e - b).ln() + c).clamp(0.0, 1.0)
    }
}

/// MaxRGB tonemap: scale by max channel (HDR games / mpv alternative).
pub fn maxrgb_tonemap_pixel(red: u8, green: u8, blue: u8) -> (u8, u8, u8) {
    let r = f32::from(red) / 255.0;
    let g = f32::from(green) / 255.0;
    let b = f32::from(blue) / 255.0;
    let m = r.max(g).max(b).max(1e-6);
    let scale = (1.0 / (1.0 + m)).clamp(0.0, 1.0);
    (
        (r * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * scale * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

pub fn format_maxrgb_osd(enabled: bool) -> &'static str {
    if enabled {
        "MaxRGB tonemap"
    } else {
        "MaxRGB Off"
    }
}

/// Mercator projection from equirect (cartographic / planet viewers).
pub fn project_mercator_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + nx * half * aspect;
            let lat = (pitch + (ny * half).sinh().atan()).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Dual fisheye top-bottom → equirect (Insta360 / GoPro TB layout).
pub fn dual_fisheye_tb_to_equirect(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    if src_w == 0 || src_h < 2 || src.is_empty() {
        return out;
    }
    let eye_h = src_h / 2;
    for y in 0..out_h {
        let lat = (0.5 - (y as f32 + 0.5) / out_h as f32) * std::f32::consts::PI;
        for x in 0..out_w {
            let lon = ((x as f32 + 0.5) / out_w as f32 * 2.0 - 1.0) * std::f32::consts::PI;
            let use_bottom = lon >= 0.0;
            let eye_lon = if use_bottom {
                lon
            } else {
                lon + std::f32::consts::PI
            };
            let r = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0) * 0.5;
            let fx = 0.5 + r * eye_lon.cos();
            let fy = 0.5 + r * eye_lon.sin();
            let ox = (fx.clamp(0.0, 1.0) * (src_w.saturating_sub(1) as f32)).round() as u32;
            let oy = ((fy.clamp(0.0, 1.0) * (eye_h.saturating_sub(1) as f32)).round() as u32)
                + if use_bottom { eye_h } else { 0 };
            let idx =
                (oy.min(src_h - 1) as usize) * (src_w as usize) + (ox.min(src_w - 1) as usize);
            out[(y * out_w + x) as usize] = src.get(idx).copied().unwrap_or(0);
        }
    }
    out
}

/// Target live latency for low-latency HLS/DASH (ms).
pub fn clamp_live_latency_ms(ms: u32) -> u32 {
    ms.clamp(100, 60_000)
}

pub fn format_live_latency_osd(ms: u32) -> String {
    format!("Live latency {} ms", clamp_live_latency_ms(ms))
}

/// Buffer target fill ratio (0..=1000) for ABR decisions.
pub fn buffer_health_ratio_milli(buffered_us: i64, target_us: i64) -> u32 {
    if target_us <= 0 {
        return 1_000;
    }
    ((buffered_us.max(0) as i128 * 1_000) / target_us as i128).clamp(0, 2_000) as u32
}

/// EPG / program-guide row.
pub fn format_epg_program_osd(title: &str, start_us: i64, end_us: i64) -> String {
    format!(
        "EPG {} {}–{}",
        title.trim(),
        format_play_clock(start_us.max(0)),
        format_play_clock(end_us.max(0))
    )
}

/// CEA-708 service channel label.
pub fn format_cea708_service_osd(service: u32) -> String {
    if service == 0 {
        "CEA-708 Off".into()
    } else {
        format!("CEA-708 service {service}")
    }
}

/// Display paper-white / reference white nits (HDR UI / subtitles).
pub const PAPER_WHITE_DEFAULT_NITS: u32 = 203;

pub fn clamp_paper_white_nits(nits: u32) -> u32 {
    nits.clamp(80, 400)
}

pub fn format_paper_white_osd(nits: u32) -> String {
    format!("Paper white {} nits", clamp_paper_white_nits(nits))
}

/// Scale SDR UI overlay toward HDR paper-white.
pub fn scale_sdr_overlay_to_paper_white(value: u8, paper_white_nits: u32) -> u8 {
    let pw = clamp_paper_white_nits(paper_white_nits) as f32;
    let scale = (pw / 100.0).clamp(0.5, 4.0);
    (f32::from(value) * scale).round().clamp(0.0, 255.0) as u8
}

/// ST.2094 / HDR10+ L1 frame peak & average (oracle metadata).
pub fn format_st2094_l1_osd(peak_nits: u32, avg_nits: u32) -> String {
    format!(
        "ST.2094 L1 peak {} avg {}",
        clamp_hdr_maxcll(peak_nits),
        clamp_hdr_maxcll(avg_nits)
    )
}

/// Half vs full side-by-side packing for 3D/360 stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StereoPacking {
    #[default]
    HalfSbs,
    FullSbs,
    HalfOu,
    FullOu,
}

pub fn cycle_stereo_packing(mode: StereoPacking) -> StereoPacking {
    match mode {
        StereoPacking::HalfSbs => StereoPacking::FullSbs,
        StereoPacking::FullSbs => StereoPacking::HalfOu,
        StereoPacking::HalfOu => StereoPacking::FullOu,
        StereoPacking::FullOu => StereoPacking::HalfSbs,
    }
}

pub fn stereo_packing_label(mode: StereoPacking) -> &'static str {
    match mode {
        StereoPacking::HalfSbs => "Half-SBS",
        StereoPacking::FullSbs => "Full-SBS",
        StereoPacking::HalfOu => "Half-OU",
        StereoPacking::FullOu => "Full-OU",
    }
}

pub fn format_stereo_packing_osd(mode: StereoPacking) -> String {
    format!("Stereo {}", stereo_packing_label(mode))
}

/// Row-interleaved 3D field select (even/odd rows).
pub fn row_interleaved_eye_pixel(y: u32, left_eye: bool) -> bool {
    if left_eye { y % 2 == 0 } else { y % 2 == 1 }
}

pub fn format_row_interleaved_osd(enabled: bool) -> &'static str {
    if enabled {
        "Row-interleaved 3D"
    } else {
        "Row-interleaved Off"
    }
}

/// AR / passthrough compositor opacity (0 = full VR, 1000 = full camera).
pub fn passthrough_blend_milli(vr_opacity_milli: i32) -> i32 {
    (1_000 - vr_opacity_milli.clamp(0, 1_000)).clamp(0, 1_000)
}

pub fn format_passthrough_osd(blend_milli: i32) -> String {
    format!("Passthrough {}%", blend_milli.clamp(0, 1_000) / 10)
}

/// Skip-recap / skip-intro marker pair.
pub fn skip_segment_target_us(position_us: i64, start_us: i64, end_us: i64) -> Option<i64> {
    if end_us <= start_us {
        return None;
    }
    if position_us >= start_us && position_us < end_us {
        Some(end_us)
    } else {
        None
    }
}

pub fn format_skip_segment_osd(kind: &str) -> String {
    format!("Skip {kind}")
}

/// Binge / autoplay-next mode OSD.
pub fn format_binge_mode_osd(enabled: bool) -> &'static str {
    if enabled {
        "Binge mode On"
    } else {
        "Binge mode Off"
    }
}

/// Orthographic spherical projection (planetarium / scientific viewers).
pub fn project_orthographic_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let r2 = nx * nx + ny * ny;
            if r2 > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let nz = (1.0 - r2).sqrt();
            let y1 = ny * cos_p - nz * sin_p;
            let z1 = ny * sin_p + nz * cos_p;
            let x2 = nx * cos_y + z1 * sin_y;
            let z2 = -nx * sin_y + z1 * cos_y;
            let lon = z2.atan2(x2);
            let lat = y1.asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Diffuse white / graphics white nits for HDR UI (ITU-R BT.2408).
pub fn clamp_diffuse_white_nits(nits: u32) -> u32 {
    nits.clamp(100, 300)
}

pub fn format_diffuse_white_osd(nits: u32) -> String {
    format!("Diffuse white {} nits", clamp_diffuse_white_nits(nits))
}

/// Map content luminance to display using paper-white reference.
pub fn map_nits_via_paper_white(
    content_nits: u32,
    paper_white_nits: u32,
    display_peak_nits: u32,
) -> u32 {
    let pw = clamp_paper_white_nits(paper_white_nits).max(1);
    let peak = clamp_hdr_nits(display_peak_nits).max(1);
    let scaled = (content_nits as u64 * peak as u64) / pw as u64;
    scaled.min(u64::from(peak)) as u32
}

/// Dolby Vision profile/level OSD (oracle; bitstream OOS).
pub fn format_dolby_vision_profile_level_osd(profile: u32, level: u32) -> String {
    format!("Dolby Vision {profile}.{level}")
}

/// Checkerboard 3D sample: choose L/R by (x+y) parity.
pub fn checkerboard_eye_is_left(x: u32, y: u32) -> bool {
    (x + y) % 2 == 0
}

pub fn format_checkerboard_3d_osd(enabled: bool) -> &'static str {
    if enabled {
        "Checkerboard 3D"
    } else {
        "Checkerboard Off"
    }
}

/// Column-interleaved 3D eye select.
pub fn column_interleaved_eye_is_left(x: u32) -> bool {
    x % 2 == 0
}

/// Wiggle stereoscopy amplitude (milli-deg yaw oscillation).
pub fn wiggle_yaw_offset_milli(phase_milli: i32, amplitude_milli: i32) -> i32 {
    let phase = (phase_milli.rem_euclid(1_000) as f32 / 1_000.0) * std::f32::consts::TAU;
    let amp = amplitude_milli.clamp(0, 30_000) as f32;
    (phase.sin() * amp).round() as i32
}

pub fn format_wiggle_3d_osd(amplitude_milli: i32) -> String {
    if amplitude_milli <= 0 {
        "Wiggle 3D Off".into()
    } else {
        format!(
            "Wiggle ±{:.1}°",
            amplitude_milli.clamp(0, 30_000) as f32 / 1_000.0
        )
    }
}

/// Quest / Pico style guardian boundary distance (mm).
pub fn format_guardian_osd(distance_mm: u32) -> String {
    if distance_mm == 0 {
        "Guardian clear".into()
    } else {
        format!("Guardian {distance_mm} mm")
    }
}

/// Gnomonic (gnomonic/rectilinear) projection from equirect.
pub fn project_gnomonic_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    // Same ray math as equirect viewport — gnomonic is the rectilinear camera model.
    project_equirect_view_ex(
        src_w,
        src_h,
        src,
        out_w,
        out_h,
        yaw_deg_milli,
        pitch_deg_milli,
        roll_deg_milli,
        fov_deg_milli,
    )
}

/// Sinusoidal equal-area map projection from equirect.
pub fn project_sinusoidal_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        let lat = (pitch + ny * half).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        let cos_lat = lat.cos().max(1e-3);
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + (nx * half * aspect) / cos_lat;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Vertical cubemap cross layout face UV rect.
pub fn cubemap_cross_face_rect(canvas_w: u32, canvas_h: u32, face: u8) -> (u32, u32, u32, u32) {
    //     [+Y]
    // [-X][+Z][+X][-Z]
    //     [-Y]
    let cell = (canvas_w.min(canvas_h) / 4).max(1);
    let (col, row) = match face % 6 {
        0 => (2u32, 1), // +X
        1 => (0, 1),    // -X
        2 => (1, 0),    // +Y
        3 => (1, 2),    // -Y
        4 => (1, 1),    // +Z
        _ => (3, 1),    // -Z
    };
    (col * cell, row * cell, cell, cell)
}

pub fn format_cubemap_cross_osd(face: u8) -> String {
    format!("Cube cross face {face}")
}

/// HLG system gamma (OOTF) for display light (BT.2100).
pub fn clamp_hlg_system_gamma_milli(value: i32) -> i32 {
    value.clamp(1_000, 2_000)
}

pub fn hlg_system_gamma_default_milli(peak_nits: u32) -> i32 {
    // BT.2100: γ = 1.2 + 0.42*log10(Lw/1000)
    let lw = clamp_hdr_nits(peak_nits).max(1) as f32;
    let g = 1.2 + 0.42 * (lw / 1_000.0).log10();
    clamp_hlg_system_gamma_milli((g * 1_000.0).round() as i32)
}

pub fn format_hlg_system_gamma_osd(gamma_milli: i32) -> String {
    format!(
        "HLG system γ{:.2}",
        clamp_hlg_system_gamma_milli(gamma_milli) as f32 / 1_000.0
    )
}

/// BT.2020 → approximate BT.709 matrix (gamut map oracle).
pub fn bt2020_to_bt709_rgb(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    // Coarse primary remap; not ICC-accurate.
    let r = 1.6605 * red - 0.5876 * green - 0.0728 * blue;
    let g = -0.1246 * red + 1.1329 * green - 0.0083 * blue;
    let b = -0.0182 * red - 0.1006 * green + 1.1187 * blue;
    (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0))
}

pub fn apply_bt2020_to_bt709_pixel(red: u8, green: u8, blue: u8) -> (u8, u8, u8) {
    let (r, g, b) = bt2020_to_bt709_rgb(
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
    );
    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

pub fn format_gamut_map_osd(enabled: bool) -> &'static str {
    if enabled {
        "Gamut BT.2020→709"
    } else {
        "Gamut map Off"
    }
}

/// Display white-point presets (CIE xy ×10000).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayWhitePoint {
    #[default]
    D65,
    Dci,
    D50,
}

pub fn white_point_xy_milli(wp: DisplayWhitePoint) -> (u32, u32) {
    match wp {
        DisplayWhitePoint::D65 => (3_127, 3_290),
        DisplayWhitePoint::Dci => (3_140, 3_510),
        DisplayWhitePoint::D50 => (3_457, 3_586),
    }
}

pub fn cycle_display_white_point(wp: DisplayWhitePoint) -> DisplayWhitePoint {
    match wp {
        DisplayWhitePoint::D65 => DisplayWhitePoint::Dci,
        DisplayWhitePoint::Dci => DisplayWhitePoint::D50,
        DisplayWhitePoint::D50 => DisplayWhitePoint::D65,
    }
}

pub fn format_white_point_osd(wp: DisplayWhitePoint) -> String {
    let (x, y) = white_point_xy_milli(wp);
    let name = match wp {
        DisplayWhitePoint::D65 => "D65",
        DisplayWhitePoint::Dci => "DCI",
        DisplayWhitePoint::D50 => "D50",
    };
    format!(
        "{name} x={:.4} y={:.4}",
        x as f32 / 10_000.0,
        y as f32 / 10_000.0
    )
}

/// Parse EDID-like peak luminance token (`MaxLuminance=600`).
pub fn parse_edid_max_luminance(spec: &str) -> Option<u32> {
    let spec = spec.trim();
    let value = spec
        .strip_prefix("MaxLuminance=")
        .or_else(|| spec.strip_prefix("max_luminance="))
        .or_else(|| spec.strip_prefix("peak="))?;
    value.parse().ok().map(clamp_hdr_nits)
}

pub fn format_edid_peak_osd(nits: u32) -> String {
    format!("EDID peak {} nits", clamp_hdr_nits(nits))
}

/// Trilinear-ish 1D LUT sample (CMS / calibration).
pub fn sample_1d_lut_u8(lut: &[u8], value: u8) -> u8 {
    if lut.is_empty() {
        return value;
    }
    if lut.len() == 1 {
        return lut[0];
    }
    let pos = (u32::from(value) * (lut.len() as u32 - 1)) / 255;
    let idx = pos as usize;
    let next = (idx + 1).min(lut.len() - 1);
    let frac = (u32::from(value) * (lut.len() as u32 - 1)) % 255;
    let a = u32::from(lut[idx]);
    let b = u32::from(lut[next]);
    ((a * (255 - frac) + b * frac) / 255) as u8
}

pub fn format_cms_lut_osd(enabled: bool, size: usize) -> String {
    if !enabled {
        "CMS LUT Off".into()
    } else {
        format!("CMS 1D LUT {size}")
    }
}

/// Miller cylindrical projection from equirect.
pub fn project_miller_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        // Inverse Miller: lat = 1.25*atan(sinh(0.8*y))
        let y = pitch + ny * half;
        let lat = (1.25 * (0.8 * y).sinh().atan()).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + nx * half * aspect;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Azimuthal equidistant projection (radar / planetarium style).
pub fn project_azimuthal_equidistant_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let rho = (nx * nx + ny * ny).sqrt();
            if rho > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let c = rho * std::f32::consts::PI; // angular distance
            let phi = ny.atan2(nx);
            let lat = (c.cos() * pitch.sin() + c.sin() * pitch.cos() * phi.cos())
                .asin()
                .clamp(
                    -std::f32::consts::FRAC_PI_2 + 0.01,
                    std::f32::consts::FRAC_PI_2 - 0.01,
                );
            let lon =
                yaw + (phi.sin() * c.sin() * pitch.cos()).atan2(c.cos() - pitch.sin() * lat.sin());
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// HDR brightness boost relative to paper-white (UI / subtitles).
pub fn hdr_brightness_boost_milli(paper_white_nits: u32, boost_percent: i32) -> u32 {
    let pw = clamp_paper_white_nits(paper_white_nits);
    let boost = boost_percent.clamp(0, 400) as u32;
    clamp_hdr_nits(pw.saturating_mul(100 + boost) / 100)
}

pub fn format_hdr_brightness_osd(boost_percent: i32) -> String {
    format!("HDR brightness +{}%", boost_percent.clamp(0, 400))
}

/// Force HDR path even when stream is SDR (debug / calibration).
pub fn format_force_hdr_osd(enabled: bool) -> &'static str {
    if enabled {
        "Force HDR On"
    } else {
        "Force HDR Off"
    }
}

/// Scene-referred vs display-referred light model OSD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HdrLightModel {
    #[default]
    Display,
    Scene,
}

pub fn cycle_hdr_light_model(mode: HdrLightModel) -> HdrLightModel {
    match mode {
        HdrLightModel::Display => HdrLightModel::Scene,
        HdrLightModel::Scene => HdrLightModel::Display,
    }
}

pub fn format_hdr_light_model_osd(mode: HdrLightModel) -> &'static str {
    match mode {
        HdrLightModel::Display => "HDR display-light",
        HdrLightModel::Scene => "HDR scene-light",
    }
}

/// Parse `.cube` 1D LUT SIZE line (`LUT_1D_SIZE 256`).
pub fn parse_cube_lut_1d_size(line: &str) -> Option<usize> {
    let rest = line.trim().strip_prefix("LUT_1D_SIZE")?.trim();
    rest.parse().ok().filter(|&n| n >= 2 && n <= 65_536)
}

pub fn format_cube_lut_osd(size: Option<usize>) -> String {
    match size {
        Some(n) => format!("CUBE 1D {n}"),
        None => "CUBE Off".into(),
    }
}

/// Prefer container metadata title; otherwise the file stem / URL leaf.
pub fn media_display_title(path: &Path, metadata_title: Option<&str>) -> String {
    if let Some(title) = metadata_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
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
    pub stereo3d: PlayStereo3D,
    pub spherical: bool,
    pub yaw_deg_milli: i32,
    pub pitch_deg_milli: i32,
    pub roll_deg_milli: i32,
    pub fov_deg_milli: i32,
    pub hdr_tonemap: HdrTonemap,
    /// Stream `color_trc` used when expanding PQ/HLG before display tonemap.
    pub color_trc: u32,
    /// Post-process color effect (invert/sepia/grayscale).
    pub display_effect: DisplayEffect,
    /// Display peak luminance in nits for HDR output scaling.
    pub hdr_nits: u32,
    /// Spatial post filter (blur/sharpen/grain).
    pub post_fx: VideoPostFx,
    /// 360° source projection when `spherical` is enabled.
    pub spherical_projection: SphericalProjection,
    /// HDR10 MaxCLL metadata (nits).
    pub hdr_maxcll: u32,
    /// HDR10 MaxFALL metadata (nits).
    pub hdr_maxfall: u32,
    /// Stream color primaries (BT.709 / BT.2020).
    pub color_primaries: u32,
    /// Blend HDR tonemap toward identity (0..=1000).
    pub tonemap_strength_milli: i32,
    /// Desaturate HDR highlights after tonemap (0..=1000).
    pub hdr_highlight_desat_milli: i32,
    /// Lift crushed HDR blacks (0..=1000).
    pub hdr_black_lift_milli: i32,
    /// White-balance color temperature (Kelvin).
    pub color_temp_kelvin: i32,
    /// HLG OOTF display gamma ×1000; `0` skips OOTF.
    pub hlg_ootf_gamma_milli: i32,
    /// Use BT.2446 HDR→SDR tonemap instead of Hable/Reinhard when set.
    pub bt2446_tonemap: bool,
    /// Map BT.2020 content toward BT.709 display.
    pub gamut_map_bt709: bool,
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
            stereo3d: PlayStereo3D::Off,
            spherical: false,
            yaw_deg_milli: 0,
            pitch_deg_milli: 0,
            roll_deg_milli: 0,
            fov_deg_milli: FOV_DEFAULT_MILLI,
            hdr_tonemap: HdrTonemap::Off,
            color_trc: 0,
            display_effect: DisplayEffect::Off,
            hdr_nits: HDR_NITS_DEFAULT,
            post_fx: VideoPostFx::Off,
            spherical_projection: SphericalProjection::Equirect,
            hdr_maxcll: 0,
            hdr_maxfall: 0,
            color_primaries: 0,
            tonemap_strength_milli: TONEMAP_STRENGTH_DEFAULT_MILLI,
            hdr_highlight_desat_milli: 0,
            hdr_black_lift_milli: 0,
            color_temp_kelvin: COLOR_TEMP_DAYLIGHT_K,
            hlg_ootf_gamma_milli: 0,
            bt2446_tonemap: false,
            gamut_map_bt709: false,
        }
    }
}

/// VLC-compatible 360° FOV limits (degrees ×1000).
pub const FOV_MIN_MILLI: i32 = 20_000;
pub const FOV_MAX_MILLI: i32 = 150_000;
pub const FOV_DEFAULT_MILLI: i32 = 80_000;
pub const FOV_STEP_MILLI: i32 = 5_000;
pub const YAW_STEP_MILLI: i32 = 5_000;
pub const PITCH_STEP_MILLI: i32 = 5_000;
pub const PITCH_MIN_MILLI: i32 = -89_000;
pub const PITCH_MAX_MILLI: i32 = 89_000;
pub const ROLL_STEP_MILLI: i32 = 5_000;

pub fn clamp_yaw_milli(value: i32) -> i32 {
    let mut yaw = value % 360_000;
    if yaw < 0 {
        yaw += 360_000;
    }
    yaw
}

pub fn clamp_pitch_milli(value: i32) -> i32 {
    value.clamp(PITCH_MIN_MILLI, PITCH_MAX_MILLI)
}

pub fn clamp_fov_milli(value: i32) -> i32 {
    value.clamp(FOV_MIN_MILLI, FOV_MAX_MILLI)
}

pub fn clamp_roll_milli(value: i32) -> i32 {
    let mut roll = value % 360_000;
    if roll < 0 {
        roll += 360_000;
    }
    roll
}

pub fn yaw_step_milli(current: i32, delta: i32) -> i32 {
    clamp_yaw_milli(current + delta)
}

pub fn pitch_step_milli(current: i32, delta: i32) -> i32 {
    clamp_pitch_milli(current + delta)
}

pub fn fov_step_milli(current: i32, delta: i32) -> i32 {
    clamp_fov_milli(current + delta)
}

pub fn roll_step_milli(current: i32, delta: i32) -> i32 {
    clamp_roll_milli(current + delta)
}

pub fn format_spherical_osd(
    enabled: bool,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> String {
    format_spherical_osd_ex(enabled, yaw_deg_milli, pitch_deg_milli, 0, fov_deg_milli)
}

pub fn format_spherical_osd_ex(
    enabled: bool,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> String {
    if !enabled {
        return "360° off".into();
    }
    format!(
        "360° yaw {:.0}° pitch {:.0}° roll {:.0}° fov {:.0}°",
        clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0,
        clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0,
        clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0,
        clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0
    )
}

/// Sample equirectangular source at normalized lon/lat (radians).
pub fn sample_equirect_pixel(pixels: &[u32], width: u32, height: u32, lon: f32, lat: f32) -> u32 {
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let u = ((lon / std::f32::consts::PI + 1.0) * 0.5).rem_euclid(1.0);
    let v = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0);
    let x = ((u * w).floor() as usize).min(width.saturating_sub(1) as usize);
    let y = ((v * h).floor() as usize).min(height.saturating_sub(1) as usize);
    let idx = y.saturating_mul(width as usize).saturating_add(x);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Project a rectilinear viewport from an equirectangular 360° frame (VLC-style).
pub fn project_equirect_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    project_equirect_view_ex(
        src_w,
        src_h,
        src,
        out_w,
        out_h,
        yaw_deg_milli,
        pitch_deg_milli,
        0,
        fov_deg_milli,
    )
}

/// Like [`project_equirect_view`] with viewpoint roll (degrees ×1000).
pub fn project_equirect_view_ex(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            // Camera looks +Z; rotate pitch then yaw.
            let x1 = nx;
            let y1 = ny * cos_p - 1.0 * sin_p;
            let z1 = ny * sin_p + 1.0 * cos_p;
            let x2 = x1 * cos_y + z1 * sin_y;
            let y2 = y1;
            let z2 = -x1 * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
            let dx = x2 / len;
            let dy = y2 / len;
            let dz = z2 / len;
            let lon = dx.atan2(dz);
            let lat = dy.clamp(-1.0, 1.0).asin();
            out[oy as usize * out_w as usize + ox as usize] =
                sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Display HDR→SDR tonemap operators for play (VLC-style preference names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HdrTonemap {
    #[default]
    Off,
    Clip,
    Reinhard,
    Hable,
    /// Möbius tonemap (mpv `mobius`).
    Mobius,
    /// ACES filmic curve (approximate).
    Aces,
    /// MaxRGB channel-relative tonemap.
    MaxRgb,
}

/// FFmpeg `AVCOL_TRC_SMPTE2084` (PQ).
pub const COLOR_TRC_SMPTE2084: u32 = 16;
/// FFmpeg `AVCOL_TRC_ARIB_STD_B67` (HLG).
pub const COLOR_TRC_HLG: u32 = 18;

pub fn is_hdr_transfer(color_trc: u32) -> bool {
    color_trc == COLOR_TRC_SMPTE2084 || color_trc == COLOR_TRC_HLG
}

pub fn cycle_hdr_tonemap(mode: HdrTonemap) -> HdrTonemap {
    match mode {
        HdrTonemap::Off => HdrTonemap::Clip,
        HdrTonemap::Clip => HdrTonemap::Reinhard,
        HdrTonemap::Reinhard => HdrTonemap::Hable,
        HdrTonemap::Hable => HdrTonemap::Mobius,
        HdrTonemap::Mobius => HdrTonemap::Aces,
        HdrTonemap::Aces => HdrTonemap::MaxRgb,
        HdrTonemap::MaxRgb => HdrTonemap::Off,
    }
}

pub fn hdr_tonemap_label(mode: HdrTonemap) -> &'static str {
    match mode {
        HdrTonemap::Off => "off",
        HdrTonemap::Clip => "clip",
        HdrTonemap::Reinhard => "reinhard",
        HdrTonemap::Hable => "hable",
        HdrTonemap::Mobius => "mobius",
        HdrTonemap::Aces => "aces",
        HdrTonemap::MaxRgb => "maxrgb",
    }
}

pub fn format_hdr_tonemap_osd(mode: HdrTonemap) -> String {
    format!("HDR tonemap {}", hdr_tonemap_label(mode))
}

fn hable_tonemap(x: f32) -> f32 {
    let a = 0.15;
    let b = 0.50;
    let c = 0.10;
    let d = 0.20;
    let e = 0.02;
    let f = 0.30;
    ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f
}

/// Möbius shoulder from FFmpeg/mpv: 1:1 below the joint `j`, rational above it,
/// reaching exactly 1.0 at the content peak. `PEAK` is this file's own highlight
/// ceiling — the one [`hable_tonemap`] is normalised by — so the modes agree on
/// where the top of the range sits even though only some of them know it.
fn mobius_tonemap(x: f32, j: f32) -> f32 {
    const PEAK: f32 = 11.2;
    let j = j.clamp(0.0, 0.999);
    let x = x.max(0.0);
    if x <= j {
        return x;
    }
    let a = -j * j * (PEAK - 1.0) / (j * j - 2.0 * j + PEAK);
    let b = (j * j - 2.0 * j * PEAK + PEAK) / (PEAK - 1.0);
    let scale = (b * b + 2.0 * b * j + j * j) / (b - a);
    (scale * (x + a) / (x + b)).clamp(0.0, 1.0)
}

fn aces_tonemap(x: f32) -> f32 {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let x = x.max(0.0);
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

/// Approximate PQ EOTF (SMPTE ST 2084) from normalized [0,1] code value to relative luminance.
pub fn pq_eotf(value: f32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let vm = v.powf(1.0 / m2);
    let num = (vm - c1).max(0.0);
    let den = (c2 - c3 * vm).max(1e-6);
    (num / den).powf(1.0 / m1) * 100.0 // scale toward displayable range
}

/// Approximate HLG inverse OETF to relative scene-light.
pub fn hlg_eotf(value: f32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    if v <= 0.5 {
        (v * v) / 3.0 * 12.0
    } else {
        ((v - c) / a).exp().mul_add(1.0, b) / 12.0 * 12.0
    }
}

/// Expand an 8-bit channel using stream transfer before display tonemap.
pub fn expand_hdr_channel(value: f32, color_trc: u32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    if color_trc == COLOR_TRC_SMPTE2084 {
        pq_eotf(v)
    } else if color_trc == COLOR_TRC_HLG {
        hlg_eotf(v)
    } else {
        v * 2.5
    }
}

/// Map one linear channel through a display tonemap curve.
pub fn tonemap_channel(value: f32, mode: HdrTonemap) -> f32 {
    let x = value.max(0.0);
    match mode {
        HdrTonemap::Off => x.clamp(0.0, 1.0),
        HdrTonemap::Clip => x.clamp(0.0, 1.0),
        HdrTonemap::Reinhard => (x / (1.0 + x)).clamp(0.0, 1.0),
        HdrTonemap::Hable => {
            let white = hable_tonemap(11.2).max(1e-6);
            (hable_tonemap(x) / white).clamp(0.0, 1.0)
        }
        HdrTonemap::Mobius => mobius_tonemap(x, 0.3),
        HdrTonemap::Aces => aces_tonemap(x),
        HdrTonemap::MaxRgb => (x / (1.0 + x)).clamp(0.0, 1.0),
    }
}

pub fn apply_hdr_tonemap_pixel(
    red: u8,
    green: u8,
    blue: u8,
    mode: HdrTonemap,
    color_trc: u32,
) -> (u8, u8, u8) {
    if matches!(mode, HdrTonemap::Off | HdrTonemap::Clip) {
        return (red, green, blue);
    }
    if matches!(mode, HdrTonemap::MaxRgb) {
        let r = expand_hdr_channel(f32::from(red) / 255.0, color_trc);
        let g = expand_hdr_channel(f32::from(green) / 255.0, color_trc);
        let b = expand_hdr_channel(f32::from(blue) / 255.0, color_trc);
        let m = r.max(g).max(b).max(1e-6);
        let scale = 1.0 / (1.0 + m);
        return (
            (r * scale * 255.0).round().clamp(0.0, 255.0) as u8,
            (g * scale * 255.0).round().clamp(0.0, 255.0) as u8,
            (b * scale * 255.0).round().clamp(0.0, 255.0) as u8,
        );
    }
    let r = tonemap_channel(expand_hdr_channel(f32::from(red) / 255.0, color_trc), mode);
    let g = tonemap_channel(
        expand_hdr_channel(f32::from(green) / 255.0, color_trc),
        mode,
    );
    let b = tonemap_channel(expand_hdr_channel(f32::from(blue) / 255.0, color_trc), mode);
    (
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Default tonemap when stream signals PQ/HLG.
pub fn auto_hdr_tonemap(color_trc: u32) -> HdrTonemap {
    if is_hdr_transfer(color_trc) {
        HdrTonemap::Hable
    } else {
        HdrTonemap::Off
    }
}

/// Apply deinterlace, optional 360° projection, bitmap overlay, color adjust, HDR tonemap, and rotate.
/// Returns `(width, height, packed 0x00RRGGBB pixels)`.
pub fn render_play_pixels(
    width: u32,
    height: u32,
    pixels: &[u32],
    opts: &PlayRenderOptions,
    bitmap: Option<&BitmapSubtitle>,
) -> (u32, u32, Vec<u32>) {
    let mut width = width;
    let mut height = height;
    let mut source = pixels.to_vec();
    let needed = (width as usize).saturating_mul(height as usize);
    if source.len() < needed {
        source.resize(needed, 0);
    }
    apply_deinterlace_rgb(&mut source, width, height, opts.deinterlace);
    if !matches!(opts.stereo3d, PlayStereo3D::Off) {
        let (sw, sh, packed) = apply_play_stereo3d(width, height, &source, opts.stereo3d);
        width = sw;
        height = sh;
        source = packed;
    }
    if opts.spherical {
        source = project_spherical_view(
            width,
            height,
            &source,
            width,
            height,
            opts.spherical_projection,
            opts.yaw_deg_milli,
            opts.pitch_deg_milli,
            opts.roll_deg_milli,
            opts.fov_deg_milli,
        );
    }
    if let Some(plane) = bitmap {
        blit_bitmap_subtitle(&mut source, width, height, plane);
    }
    let w = width as usize;
    let h = height as usize;
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
            let orig_r = red;
            let orig_g = green;
            let orig_b = blue;
            let (red, green, blue) =
                if opts.bt2446_tonemap && !matches!(opts.hdr_tonemap, HdrTonemap::Off) {
                    apply_bt2446_tonemap_pixel(red, green, blue, opts.hdr_nits)
                } else {
                    apply_hdr_tonemap_pixel(red, green, blue, opts.hdr_tonemap, opts.color_trc)
                };
            let (red, green, blue) = (
                blend_tonemap_channel(red, orig_r, opts.tonemap_strength_milli),
                blend_tonemap_channel(green, orig_g, opts.tonemap_strength_milli),
                blend_tonemap_channel(blue, orig_b, opts.tonemap_strength_milli),
            );
            let (red, green, blue) = if matches!(opts.hdr_tonemap, HdrTonemap::Off) {
                (red, green, blue)
            } else {
                (
                    scale_hdr_display_channel(red, opts.hdr_nits),
                    scale_hdr_display_channel(green, opts.hdr_nits),
                    scale_hdr_display_channel(blue, opts.hdr_nits),
                )
            };
            let (red, green, blue) =
                apply_hdr_highlight_desat_pixel(red, green, blue, opts.hdr_highlight_desat_milli);
            let (red, green, blue) = (
                apply_hdr_black_lift_channel(red, opts.hdr_black_lift_milli),
                apply_hdr_black_lift_channel(green, opts.hdr_black_lift_milli),
                apply_hdr_black_lift_channel(blue, opts.hdr_black_lift_milli),
            );
            let (red, green, blue) =
                apply_white_balance_pixel(red, green, blue, opts.color_temp_kelvin);
            let (red, green, blue) =
                if opts.hlg_ootf_gamma_milli > 0 && opts.color_trc == COLOR_TRC_HLG {
                    apply_hlg_ootf_pixel(red, green, blue, opts.hlg_ootf_gamma_milli)
                } else {
                    (red, green, blue)
                };
            let (red, green, blue) =
                if opts.gamut_map_bt709 && opts.color_primaries == COLOR_PRIMARIES_BT2020 {
                    apply_bt2020_to_bt709_pixel(red, green, blue)
                } else {
                    (red, green, blue)
                };
            let (red, green, blue) =
                apply_display_effect_pixel(red, green, blue, opts.display_effect);
            let (dx, dy) = rotate_pixel(x as u32, y as u32, width, height, opts.rotate);
            out[dy as usize * out_w as usize + dx as usize] =
                (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        }
    }
    apply_video_post_fx(&mut out, out_w, out_h, opts.post_fx);
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

/// Sleep-timer presets in minutes (`0` = off), VLC-style cycle.
pub const SLEEP_TIMER_STEPS_MIN: &[u32] = &[0, 15, 30, 45, 60, 90, 120];

pub fn cycle_sleep_timer_min(current_min: u32) -> u32 {
    let idx = SLEEP_TIMER_STEPS_MIN
        .iter()
        .position(|&minutes| minutes == current_min)
        .unwrap_or(0);
    SLEEP_TIMER_STEPS_MIN[(idx + 1) % SLEEP_TIMER_STEPS_MIN.len()]
}

pub fn sleep_deadline_secs(minutes: u32, now_secs: u64) -> Option<u64> {
    if minutes == 0 {
        None
    } else {
        Some(now_secs.saturating_add(u64::from(minutes) * 60))
    }
}

pub fn sleep_timer_fired(deadline_secs: Option<u64>, now_secs: u64) -> bool {
    deadline_secs
        .map(|deadline| now_secs >= deadline)
        .unwrap_or(false)
}

pub fn format_sleep_osd(minutes: u32) -> String {
    if minutes == 0 {
        "Sleep timer off".into()
    } else {
        format!("Sleep in {minutes} min")
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
    if paused { "Paused" } else { "Playing" }
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
    /// Enable equirectangular 360° view on open (`--spherical`).
    pub spherical: bool,
    /// 360° source projection (`--spherical-projection`).
    pub spherical_projection: SphericalProjection,
    /// Initial yaw in degrees ×1000 for 360° view.
    pub yaw_deg_milli: i32,
    /// Initial pitch in degrees ×1000 for 360° view.
    pub pitch_deg_milli: i32,
    /// Initial roll in degrees ×1000 for 360° view.
    pub roll_deg_milli: i32,
    /// Initial FOV in degrees ×1000 for 360° view.
    pub fov_deg_milli: i32,
    /// Display HDR tonemap mode (`--hdr-tonemap`).
    pub hdr_tonemap: HdrTonemap,
    /// Packed stereo3d display mode (`--play-stereo3d`).
    pub stereo3d: PlayStereo3D,
    /// Close the player when the playlist finishes (VLC `--play-and-exit`).
    pub quit_at_end: bool,
    /// Open paused (VLC `--start-paused`).
    pub start_paused: bool,
    /// Demux/network cache in milliseconds (VLC `--network-caching`).
    pub network_cache_ms: u32,
    /// Directory for snapshots (VLC `--snapshot-path`). `None` = beside media.
    pub snapshot_dir: Option<PathBuf>,
    /// HTTP reconnect attempt budget.
    pub http_reconnect: u32,
    /// Preferred audio language tag (e.g. `en`, `ru`).
    pub prefer_audio_lang: Option<String>,
    /// Preferred subtitle language tag.
    pub prefer_sub_lang: Option<String>,
    /// Fullscreen controls autohide timeout in ms.
    pub controls_autohide_ms: u64,
    /// Initial display color effect.
    pub display_effect: DisplayEffect,
    /// HDR display peak nits for tonemap output scaling.
    pub hdr_nits: u32,
    /// HDR10 MaxCLL metadata (nits).
    pub hdr_maxcll: u32,
    /// HDR10 MaxFALL metadata (nits).
    pub hdr_maxfall: u32,
    /// Stereo packing inside 360° source (`--spherical-stereo`).
    pub spherical_stereo: SphericalStereoLayout,
    /// Mastering display min luminance (milli-nits).
    pub hdr_mastering_min_milli: u32,
    /// Mastering display max luminance (nits).
    pub hdr_mastering_max_nits: u32,
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
            spherical: false,
            spherical_projection: SphericalProjection::Equirect,
            yaw_deg_milli: 0,
            pitch_deg_milli: 0,
            roll_deg_milli: 0,
            fov_deg_milli: FOV_DEFAULT_MILLI,
            hdr_tonemap: HdrTonemap::Off,
            stereo3d: PlayStereo3D::Off,
            quit_at_end: false,
            start_paused: false,
            network_cache_ms: NETWORK_CACHE_DEFAULT_MS,
            snapshot_dir: None,
            http_reconnect: HTTP_RECONNECT_DEFAULT,
            prefer_audio_lang: None,
            prefer_sub_lang: None,
            controls_autohide_ms: CONTROLS_AUTOHIDE_DEFAULT_MS,
            display_effect: DisplayEffect::Off,
            hdr_nits: HDR_NITS_DEFAULT,
            hdr_maxcll: 0,
            hdr_maxfall: 0,
            spherical_stereo: SphericalStereoLayout::Mono,
            hdr_mastering_min_milli: 0,
            hdr_mastering_max_nits: 0,
        }
    }
}

/// Parse `--hdr-tonemap` values: `off|clip|reinhard|hable|mobius|aces`.
pub fn parse_hdr_tonemap(spec: &str) -> Result<HdrTonemap> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" => Ok(HdrTonemap::Off),
        "clip" => Ok(HdrTonemap::Clip),
        "reinhard" => Ok(HdrTonemap::Reinhard),
        "hable" => Ok(HdrTonemap::Hable),
        "mobius" => Ok(HdrTonemap::Mobius),
        "aces" => Ok(HdrTonemap::Aces),
        "maxrgb" | "max-rgb" => Ok(HdrTonemap::MaxRgb),
        other => Err(format!(
            "unknown hdr tonemap `{other}` (off|clip|reinhard|hable|mobius|aces|maxrgb)"
        )
        .into()),
    }
}

/// Parse `--spherical-projection` values.
pub fn parse_spherical_projection(spec: &str) -> Result<SphericalProjection> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "equirect" | "equirectangular" | "360" => Ok(SphericalProjection::Equirect),
        "dual-fisheye" | "fisheye" | "dfisheye" => Ok(SphericalProjection::DualFisheye),
        "cubemap" | "cube" => Ok(SphericalProjection::Cubemap),
        "little-planet" | "planet" | "stereographic" => Ok(SphericalProjection::LittlePlanet),
        "eac" | "equi-angular" | "equiangular" => Ok(SphericalProjection::Eac),
        "panini" => Ok(SphericalProjection::Panini),
        "cylindrical" | "cylinder" => Ok(SphericalProjection::Cylindrical),
        "mercator" => Ok(SphericalProjection::Mercator),
        "dual-fisheye-tb" | "dfisheye-tb" | "fisheye-tb" => Ok(SphericalProjection::DualFisheyeTb),
        "octahedral" | "octa" => Ok(SphericalProjection::Octahedral),
        "equisolid" => Ok(SphericalProjection::Equisolid),
        "orthographic" | "ortho" => Ok(SphericalProjection::Orthographic),
        "gnomonic" | "rectilinear" => Ok(SphericalProjection::Gnomonic),
        "sinusoidal" | "sanson" => Ok(SphericalProjection::Sinusoidal),
        "miller" => Ok(SphericalProjection::Miller),
        "azimuthal" | "azimuthal-equidistant" | "aeqd" => {
            Ok(SphericalProjection::AzimuthalEquidistant)
        }
        other => Err(format!(
            "unknown spherical projection `{other}` (equirect|dual-fisheye|cubemap|little-planet|eac|panini|cylindrical|mercator|dual-fisheye-tb|octahedral|equisolid|orthographic|gnomonic|sinusoidal|miller|azimuthal)"
        )
        .into()),
    }
}

/// Parse `--play-stereo3d` values: `off|sbsl|abl|mono-left|mono-right`.
pub fn parse_play_stereo3d(spec: &str) -> Result<PlayStereo3D> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" => Ok(PlayStereo3D::Off),
        "sbsl" | "sbs" | "anaglyph" => Ok(PlayStereo3D::SbslAnaglyph),
        "abl" | "tab" => Ok(PlayStereo3D::AblAnaglyph),
        "mono-left" | "ml" | "left" => Ok(PlayStereo3D::MonoLeft),
        "mono-right" | "mr" | "right" => Ok(PlayStereo3D::MonoRight),
        other => Err(format!(
            "unknown play stereo3d `{other}` (off|sbsl|abl|mono-left|mono-right)"
        )
        .into()),
    }
}

/// Parse degrees for viewpoint CLI (`45`, `45.5`) into milli-degrees.
pub fn parse_degrees_milli(spec: &str) -> Result<i32> {
    let value: f32 = spec
        .parse()
        .map_err(|_| format!("invalid degrees `{spec}`"))?;
    if !value.is_finite() {
        return Err(format!("invalid degrees `{spec}`").into());
    }
    Ok((value * 1_000.0).round() as i32)
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
        if line.is_empty()
            || line.starts_with('#')
            || (line.starts_with('[') && line.ends_with(']'))
        {
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
        } else if line.contains('=') && !is_playback_url(line) && !line.contains(['/', '\\']) {
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
            return Err(format!("playlist has no entries: {}", path.display()));
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
    if on { "shuffle on" } else { "shuffle off" }
}

pub fn format_subtitle_scale_osd(scale_milli: i32) -> String {
    format!(
        "Subtitles {}%",
        clamp_subtitle_scale_milli(scale_milli) / 10
    )
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
    format_window_title_with_position(
        name,
        media_us,
        duration_us,
        paused,
        rate_milli,
        PositionDisplay::Elapsed,
    )
}

pub fn format_window_title_with_position(
    name: &str,
    media_us: i64,
    duration_us: i64,
    paused: bool,
    rate_milli: u32,
    position: PositionDisplay,
) -> String {
    let position = format_position_osd(media_us, duration_us, position);
    let paused = if paused { "  paused" } else { "" };
    let speed = if rate_milli != 1_000 {
        format!("  {}", format_rate(rate_milli))
    } else {
        String::new()
    };
    format!("{name}  {position}{paused}{speed}")
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
pub fn step_audio_skew(
    skew_frames: i64,
    media_frames: u32,
    queued_frames: u64,
) -> (i64, bool, u64) {
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

/// OSD for audio/subtitle track selection (`Audio 2/3`, `Subtitles off`).
pub fn format_track_osd(kind: &str, ordinal: i32, total: usize) -> String {
    if ordinal < 0 || total == 0 {
        format!("{kind} off")
    } else {
        format!(
            "{kind} {}/{}",
            (ordinal as usize).saturating_add(1),
            total.max(1)
        )
    }
}

/// Whether play should close after the playlist stops (VLC `--play-and-exit`).
pub fn should_quit_at_end(quit_at_end: bool, cont: PlaybackContinue) -> bool {
    quit_at_end && matches!(cont, PlaybackContinue::Stop)
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
#[cfg(feature = "player")]
pub fn audio_output_devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
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

/// Compact media-info OSD (title, geometry, duration, HDR/360 flags).
pub fn format_media_info_osd(
    title: &str,
    width: u32,
    height: u32,
    duration_us: i64,
    color_trc: u32,
    spherical: bool,
) -> String {
    let name = if title.is_empty() { "media" } else { title };
    let duration = if duration_us >= 0 {
        format_clock(duration_us)
    } else {
        "--:--".into()
    };
    let mut flags = Vec::new();
    if is_hdr_transfer(color_trc) {
        flags.push(if color_trc == COLOR_TRC_SMPTE2084 {
            "HDR PQ"
        } else {
            "HDR HLG"
        });
    }
    if spherical {
        flags.push("360°");
    }
    if flags.is_empty() {
        format!("{name}  {width}x{height}  {duration}")
    } else {
        format!("{name}  {width}x{height}  {duration}  {}", flags.join(" "))
    }
}

/// VLC/mpv-style random jump within the known duration (exclusive of the end).
pub fn random_seek_us(duration_us: i64, seed: u64) -> Option<i64> {
    if duration_us <= 0 {
        return None;
    }
    let mut state = seed | 1;
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    Some(((state as u128 * duration_us as u128) >> 64) as i64)
}

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
pub fn center_crop(
    source_w: u32,
    source_h: u32,
    ratio_w: u32,
    ratio_h: u32,
) -> (u32, u32, u32, u32) {
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

/// Cycle VLC-style integer zoom factors: 1:4 → 1:2 → 1:1 → 2:1 → 1:4.
pub fn cycle_integer_zoom(current_milli: u32) -> u32 {
    match current_milli.clamp(ZOOM_MIN_MILLI, ZOOM_MAX_MILLI) {
        0..=374 => 500,
        375..=749 => 1_000,
        750..=1_499 => 2_000,
        _ => 250,
    }
}

pub fn format_integer_zoom_osd(zoom_milli: u32) -> String {
    format!("Zoom {}", zoom_label(zoom_milli))
}

/// True when frame aspect is ~2:1 (common equirectangular 360° packaging).
pub fn detect_equirect_aspect(width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let ratio = width as f32 / height as f32;
    (ratio - 2.0).abs() <= 0.08
}

/// Fitted window size for "fit to video" / original-size views.
pub fn fit_window_to_video(video_w: u32, video_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let video_w = video_w.max(1);
    let video_h = video_h.max(1);
    let max_w = max_w.max(1);
    let max_h = max_h.max(1);
    if video_w <= max_w && video_h <= max_h {
        return (video_w, video_h);
    }
    let scale = (max_w as f32 / video_w as f32).min(max_h as f32 / video_h as f32);
    (
        ((video_w as f32) * scale).round().max(1.0) as u32,
        ((video_h as f32) * scale).round().max(1.0) as u32,
    )
}

pub fn format_fit_window_osd(width: u32, height: u32) -> String {
    format!("Window {width}x{height}")
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

pub fn clear_bookmarks(marks: &mut Vec<Bookmark>) {
    marks.clear();
}

/// Serialize bookmarks as `#EXTVLCOPT:start-time=`-style lines (seconds, fractional).
pub fn format_bookmarks_export(marks: &[Bookmark]) -> String {
    let mut out = String::from("#EXTM3U\n#EXTINF:-1,bookmarks\n");
    for mark in marks {
        let secs = mark.media_us.max(0) as f64 / 1_000_000.0;
        out.push_str(&format!("#EXTVLCOPT:start-time={secs:.3}\n"));
    }
    out
}

/// Parse `#EXTVLCOPT:start-time=` (seconds) bookmark lines into sorted unique marks.
pub fn parse_bookmarks_export(text: &str) -> Vec<Bookmark> {
    let mut marks = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("#EXTVLCOPT:start-time=")
            .or_else(|| line.strip_prefix("#EXTVLCOPT:start-time ="))
        else {
            continue;
        };
        let Ok(secs) = rest.trim().parse::<f64>() else {
            continue;
        };
        if !secs.is_finite() || secs < 0.0 {
            continue;
        }
        let media_us = (secs * 1_000_000.0).round() as i64;
        insert_bookmark(&mut marks, media_us);
    }
    marks
}

pub fn format_bookmark_osd(media_us: i64, count: usize, added: bool) -> String {
    if added {
        format!("bookmark {} ({count})", format_clock(media_us))
    } else {
        format!("bookmark {} already set", format_clock(media_us))
    }
}

pub fn format_playlist_osd(index: usize, total: usize) -> String {
    format!("{}/{}", index.saturating_add(1), total.max(1))
}

/// Serialize paths as a simple M3U playlist (one absolute/relative path per line).
pub fn format_playlist_m3u(paths: &[PathBuf]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for path in paths {
        if let Some(text) = path.to_str() {
            out.push_str(text);
            out.push('\n');
        }
    }
    out
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

/// Resolve snapshot output under an optional directory (VLC `--snapshot-path`).
pub fn snapshot_path_in_dir(dir: Option<&Path>, video: &Path, index: u32, ext: &str) -> PathBuf {
    let default = snapshot_path_with_ext(video, index, ext);
    match dir {
        Some(folder) if !folder.as_os_str().is_empty() => {
            let name = default
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("frame-fvid-{index}.{ext}")));
            folder.join(name)
        }
        _ => default,
    }
}

pub fn format_snapshot_dir_osd(dir: Option<&Path>) -> String {
    match dir {
        Some(path) if !path.as_os_str().is_empty() => {
            format!("Snapshots {}", path.display())
        }
        _ => "Snapshots beside media".into(),
    }
}

/// Network/stream demux cache in milliseconds (VLC `--network-caching`).
pub const NETWORK_CACHE_DEFAULT_MS: u32 = 1_000;
pub const NETWORK_CACHE_MIN_MS: u32 = 0;
pub const NETWORK_CACHE_MAX_MS: u32 = 60_000;

pub fn clamp_network_cache_ms(ms: u32) -> u32 {
    ms.clamp(NETWORK_CACHE_MIN_MS, NETWORK_CACHE_MAX_MS)
}

pub fn format_network_cache_osd(ms: u32) -> String {
    format!("Network cache {} ms", clamp_network_cache_ms(ms))
}

/// Convert network-caching milliseconds to a demux start delay in microseconds.
pub fn network_cache_delay_us(ms: u32) -> i64 {
    i64::from(clamp_network_cache_ms(ms)).saturating_mul(1_000)
}

/// VLC-style caching domains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CacheDomain {
    #[default]
    Network,
    File,
    Live,
    Disc,
}

pub fn cycle_cache_domain(domain: CacheDomain) -> CacheDomain {
    match domain {
        CacheDomain::Network => CacheDomain::File,
        CacheDomain::File => CacheDomain::Live,
        CacheDomain::Live => CacheDomain::Disc,
        CacheDomain::Disc => CacheDomain::Network,
    }
}

pub fn cache_domain_label(domain: CacheDomain) -> &'static str {
    match domain {
        CacheDomain::Network => "network",
        CacheDomain::File => "file",
        CacheDomain::Live => "live",
        CacheDomain::Disc => "disc",
    }
}

pub fn default_cache_ms(domain: CacheDomain) -> u32 {
    match domain {
        CacheDomain::Network => 1_000,
        CacheDomain::File => 300,
        CacheDomain::Live => 300,
        CacheDomain::Disc => 300,
    }
}

pub fn clamp_cache_ms(domain: CacheDomain, ms: u32) -> u32 {
    let _ = domain;
    clamp_network_cache_ms(ms)
}

pub fn format_cache_osd(domain: CacheDomain, ms: u32) -> String {
    format!(
        "{} cache {} ms",
        cache_domain_label(domain),
        clamp_cache_ms(domain, ms)
    )
}

/// Secondary (dual) subtitle delay — same step semantics as primary.
pub fn secondary_subtitle_delay_us(current_us: i64, steps: i32) -> i64 {
    subtitle_delay_us(current_us, steps)
}

pub fn format_secondary_subtitle_delay_osd(delay_us: i64) -> String {
    format_delay_osd("secondary subtitle", delay_us)
}

/// Snapshot filename with an optional prefix (VLC `--snapshot-prefix`).
pub fn snapshot_path_with_prefix(
    dir: Option<&Path>,
    video: &Path,
    prefix: &str,
    index: u32,
    ext: &str,
) -> PathBuf {
    let stem = video
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("frame");
    let prefix = prefix.trim();
    let name = if prefix.is_empty() {
        format!("{stem}-fvid-{index}.{ext}")
    } else {
        format!("{prefix}{stem}-{index}.{ext}")
    };
    match dir {
        Some(folder) if !folder.as_os_str().is_empty() => folder.join(name),
        _ => match video.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
            _ => PathBuf::from(name),
        },
    }
}

pub fn format_snapshot_prefix_osd(prefix: &str) -> String {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        "Snapshot prefix default".into()
    } else {
        format!("Snapshot prefix {prefix}")
    }
}

pub fn next_snapshot_index(current: u32) -> u32 {
    current.saturating_add(1)
}

/// Graphic EQ preamp (VLC equalizer preamp), milli-linear gain around unity.
pub const EQ_PREAMP_DEFAULT_MILLI: i32 = 0;
pub const EQ_PREAMP_MIN_MILLI: i32 = -2_000;
pub const EQ_PREAMP_MAX_MILLI: i32 = 2_000;
pub const EQ_PREAMP_STEP_MILLI: i32 = 100;

pub fn clamp_eq_preamp_milli(value: i32) -> i32 {
    value.clamp(EQ_PREAMP_MIN_MILLI, EQ_PREAMP_MAX_MILLI)
}

pub fn eq_preamp_step_milli(current: i32, delta: i32) -> i32 {
    clamp_eq_preamp_milli(current.saturating_add(delta))
}

pub fn apply_eq_preamp_sample(sample: f32, preamp_milli: i32) -> f32 {
    let gain = 1.0 + (clamp_eq_preamp_milli(preamp_milli) as f32 / 1_000.0);
    (sample * gain).clamp(-1.0, 1.0)
}

pub fn format_eq_preamp_osd(preamp_milli: i32) -> String {
    let db = clamp_eq_preamp_milli(preamp_milli) as f32 / 100.0;
    format!("EQ preamp {db:+.1} dB")
}

/// Headphone spatializer strength. 0 off, 1000 mild, 2000 strong.
pub const SPATIALIZER_DEFAULT_MILLI: i32 = 0;
pub const SPATIALIZER_MIN_MILLI: i32 = 0;
pub const SPATIALIZER_MAX_MILLI: i32 = 2_000;
pub const SPATIALIZER_STEP_MILLI: i32 = 100;

pub fn clamp_spatializer_milli(value: i32) -> i32 {
    value.clamp(SPATIALIZER_MIN_MILLI, SPATIALIZER_MAX_MILLI)
}

pub fn spatializer_step_milli(current: i32, delta: i32) -> i32 {
    clamp_spatializer_milli(current.saturating_add(delta))
}

pub fn apply_spatializer(frame: &mut [f32], strength_milli: i32) {
    if frame.len() < 2 || strength_milli <= 0 {
        return;
    }
    let strength = clamp_spatializer_milli(strength_milli) as f32 / 1_000.0;
    let left = frame[0];
    let right = frame[1];
    let mid = (left + right) * 0.5;
    let side = (left - right) * 0.5;
    let wide = side * (1.0 + strength);
    let narrow = mid * (1.0 - 0.25 * strength);
    frame[0] = (narrow + wide).clamp(-1.0, 1.0);
    frame[1] = (narrow - wide).clamp(-1.0, 1.0);
}

pub fn format_spatializer_osd(strength_milli: i32) -> String {
    if clamp_spatializer_milli(strength_milli) == 0 {
        "Spatializer Off".into()
    } else {
        format!(
            "Spatializer {}%",
            clamp_spatializer_milli(strength_milli) / 10
        )
    }
}

/// Playlist gapless handoff when remaining media time is under the threshold.
pub const GAPLESS_THRESHOLD_DEFAULT_US: i64 = 50_000;

pub fn gapless_should_prefetch(remaining_us: i64, threshold_us: i64) -> bool {
    remaining_us >= 0 && remaining_us <= threshold_us.max(0)
}

pub fn format_gapless_osd(enabled: bool) -> &'static str {
    if enabled { "Gapless On" } else { "Gapless Off" }
}

/// Crossfade between playlist items (VLC `--audio-desync` style fade window).
pub const CROSSFADE_DEFAULT_MS: u32 = 0;
pub const CROSSFADE_MIN_MS: u32 = 0;
pub const CROSSFADE_MAX_MS: u32 = 10_000;
pub const CROSSFADE_STEP_MS: u32 = 250;

pub fn clamp_crossfade_ms(ms: u32) -> u32 {
    ms.clamp(CROSSFADE_MIN_MS, CROSSFADE_MAX_MS)
}

pub fn crossfade_step_ms(current: u32, delta: i32) -> u32 {
    let next = (current as i64).saturating_add(i64::from(delta));
    clamp_crossfade_ms(next.clamp(0, i64::from(CROSSFADE_MAX_MS)) as u32)
}

/// Returns (outgoing, incoming) linear gains for a crossfade window.
pub fn crossfade_gain_pair(elapsed_ms: u32, duration_ms: u32) -> (f32, f32) {
    let duration = clamp_crossfade_ms(duration_ms);
    if duration == 0 {
        return (0.0, 1.0);
    }
    let t = (elapsed_ms.min(duration) as f32) / (duration as f32);
    (1.0 - t, t)
}

pub fn format_crossfade_osd(ms: u32) -> String {
    let ms = clamp_crossfade_ms(ms);
    if ms == 0 {
        "Crossfade Off".into()
    } else {
        format!("Crossfade {ms} ms")
    }
}

/// ReplayGain linear gain in milli-units (1000 = unity).
pub const REPLAYGAIN_UNITY_MILLI: i32 = 1_000;
pub const REPLAYGAIN_MIN_MILLI: i32 = 100;
pub const REPLAYGAIN_MAX_MILLI: i32 = 4_000;

pub fn clamp_replaygain_milli(value: i32) -> i32 {
    value.clamp(REPLAYGAIN_MIN_MILLI, REPLAYGAIN_MAX_MILLI)
}

/// Convert tagged ReplayGain dB×1000 into a linear milli-gain.
pub fn replaygain_milli_from_db_milli(db_milli: i32) -> i32 {
    let db = (db_milli as f32) / 1_000.0;
    let linear = 10f32.powf(db / 20.0);
    clamp_replaygain_milli((linear * 1_000.0).round() as i32)
}

pub fn apply_replaygain_sample(sample: f32, gain_milli: i32) -> f32 {
    let gain = clamp_replaygain_milli(gain_milli) as f32 / 1_000.0;
    (sample * gain).clamp(-1.0, 1.0)
}

pub fn format_replaygain_osd(gain_milli: i32) -> String {
    let gain = clamp_replaygain_milli(gain_milli);
    if gain == REPLAYGAIN_UNITY_MILLI {
        "ReplayGain Off".into()
    } else {
        format!("ReplayGain {}%", gain / 10)
    }
}

/// Demux/network buffer fill relative to the configured cache target.
pub fn buffer_health_pct(queued_ms: u32, target_ms: u32) -> u32 {
    let target = target_ms.max(1);
    ((u64::from(queued_ms) * 100) / u64::from(target)).min(200) as u32
}

pub fn format_buffer_health_osd(queued_ms: u32, target_ms: u32) -> String {
    format!(
        "Buffer {}% ({} / {} ms)",
        buffer_health_pct(queued_ms, target_ms),
        queued_ms,
        target_ms.max(1)
    )
}

/// Snap a seek target to the nearest earlier keyframe / RAP timestamp.
pub fn snap_seek_to_keyframe(target_us: i64, keyframes_us: &[i64]) -> i64 {
    if keyframes_us.is_empty() {
        return target_us.max(0);
    }
    let mut best = keyframes_us[0];
    for &kf in keyframes_us {
        if kf <= target_us {
            best = kf;
        } else {
            break;
        }
    }
    if target_us < keyframes_us[0] {
        keyframes_us[0]
    } else {
        best
    }
}

pub fn format_keyframe_seek_osd(us: i64) -> String {
    format!("Keyframe {}", format_play_clock(us))
}

/// VLC-style forced subtitle text colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleColor {
    #[default]
    White,
    Yellow,
    Cyan,
    Green,
    Magenta,
}

pub fn cycle_subtitle_color(color: SubtitleColor) -> SubtitleColor {
    match color {
        SubtitleColor::White => SubtitleColor::Yellow,
        SubtitleColor::Yellow => SubtitleColor::Cyan,
        SubtitleColor::Cyan => SubtitleColor::Green,
        SubtitleColor::Green => SubtitleColor::Magenta,
        SubtitleColor::Magenta => SubtitleColor::White,
    }
}

pub fn subtitle_color_label(color: SubtitleColor) -> &'static str {
    match color {
        SubtitleColor::White => "White",
        SubtitleColor::Yellow => "Yellow",
        SubtitleColor::Cyan => "Cyan",
        SubtitleColor::Green => "Green",
        SubtitleColor::Magenta => "Magenta",
    }
}

pub fn subtitle_color_rgba(color: SubtitleColor) -> [u8; 4] {
    match color {
        SubtitleColor::White => [255, 255, 255, 255],
        SubtitleColor::Yellow => [255, 255, 0, 255],
        SubtitleColor::Cyan => [0, 255, 255, 255],
        SubtitleColor::Green => [0, 255, 0, 255],
        SubtitleColor::Magenta => [255, 0, 255, 255],
    }
}

pub fn format_subtitle_color_osd(color: SubtitleColor) -> String {
    format!("Subtitle {}", subtitle_color_label(color))
}

/// Prefer a forced subtitle track when the demux marks tracks as forced.
pub fn prefer_forced_subtitle_index(forced: &[bool], current: usize) -> usize {
    if forced.is_empty() {
        return current;
    }
    if let Some(idx) = forced.iter().position(|flag| *flag) {
        return idx;
    }
    current.min(forced.len() - 1)
}

pub fn format_forced_subtitle_osd(index: usize) -> String {
    format!("Forced subtitle #{}", index + 1)
}

/// Rough momentary loudness from a linear peak (oracle for VLC-style meter).
pub fn momentary_lufs_from_peak_milli(peak_milli: u32) -> i32 {
    let peak = (peak_milli as f32 / 1_000.0).max(1e-6);
    let db = 20.0 * peak.log10();
    // K-weighting stub: map FS peak dB toward LUFS-ish scale.
    ((db - 0.691) * 10.0).round() as i32
}

pub fn format_loudness_osd(peak_milli: u32) -> String {
    format!(
        "Loudness {:+.1} LUFS",
        momentary_lufs_from_peak_milli(peak_milli) as f32 / 10.0
    )
}

/// Simple spectrum bar fill levels from interleaved PCM (visualization oracle).
pub fn spectrum_bar_fills(samples: &[f32], bars: usize) -> Vec<u8> {
    let bars = bars.max(1).min(64);
    let mut out = vec![0u8; bars];
    if samples.is_empty() {
        return out;
    }
    let chunk = (samples.len() / bars).max(1);
    for (i, slot) in out.iter_mut().enumerate() {
        let start = i * chunk;
        let end = (start + chunk).min(samples.len());
        if start >= samples.len() {
            break;
        }
        let mut peak = 0.0f32;
        for sample in &samples[start..end] {
            peak = peak.max(sample.abs());
        }
        *slot = ((peak * 255.0).round() as u32).min(255) as u8;
    }
    out
}

pub fn format_spectrum_osd(fills: &[u8]) -> String {
    let lit = fills.iter().filter(|v| **v > 8).count();
    format!("Spectrum {lit}/{}", fills.len().max(1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlaylistSort {
    #[default]
    Path,
    Name,
    ReverseName,
}

pub fn cycle_playlist_sort(mode: PlaylistSort) -> PlaylistSort {
    match mode {
        PlaylistSort::Path => PlaylistSort::Name,
        PlaylistSort::Name => PlaylistSort::ReverseName,
        PlaylistSort::ReverseName => PlaylistSort::Path,
    }
}

pub fn playlist_sort_label(mode: PlaylistSort) -> &'static str {
    match mode {
        PlaylistSort::Path => "Path",
        PlaylistSort::Name => "Name",
        PlaylistSort::ReverseName => "Name↓",
    }
}

pub fn sort_playlist_paths(paths: &mut [PathBuf], mode: PlaylistSort) {
    match mode {
        PlaylistSort::Path => paths.sort(),
        PlaylistSort::Name => paths.sort_by(|a, b| {
            let an = a.file_name().unwrap_or_default();
            let bn = b.file_name().unwrap_or_default();
            an.cmp(bn).then_with(|| a.cmp(b))
        }),
        PlaylistSort::ReverseName => paths.sort_by(|a, b| {
            let an = a.file_name().unwrap_or_default();
            let bn = b.file_name().unwrap_or_default();
            bn.cmp(an).then_with(|| b.cmp(a))
        }),
    }
}

pub fn format_playlist_sort_osd(mode: PlaylistSort) -> String {
    format!("Playlist sort {}", playlist_sort_label(mode))
}

pub fn format_bookmark_label(media_us: i64, title: Option<&str>) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(title) => format!("{title} ({})", format_play_clock(media_us)),
        None => format_play_clock(media_us),
    }
}

pub const RECENT_PLAY_MAX: usize = 32;

pub fn push_recent_path(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|entry| entry != &path);
    recent.insert(0, path);
    if recent.len() > RECENT_PLAY_MAX {
        recent.truncate(RECENT_PLAY_MAX);
    }
}

pub fn format_recent_osd(recent: &[PathBuf]) -> String {
    match recent
        .first()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    {
        Some(name) => format!("Recent {} ({})", name, recent.len()),
        None => "Recent empty".into(),
    }
}

/// Compact on-screen hotkey reminder (VLC Help / F1 style).
pub fn format_hotkeys_help_osd() -> &'static str {
    "Space pause · ←→ seek · ↑↓ vol · M mute · F full · S snap · Esc quit"
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

fn playback_seek_ts(origin_us: i64, media_us: i64) -> i64 {
    origin_us.saturating_add(media_us.max(0))
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
