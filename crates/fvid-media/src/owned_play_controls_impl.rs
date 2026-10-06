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

// HDR tonemap curve lookup table cache for performance
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static TONEMAP_LUT_CACHE: OnceLock<Mutex<HashMap<(u32, i32), Vec<f32>>>> = OnceLock::new();

/// Get or create cached tonemap LUT for given bits and strength
fn get_tonemap_lut(bits: u32, strength_milli: i32) -> Vec<f32> {
    let key = (bits, strength_milli);
    
    TONEMAP_LUT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
        .lock().unwrap()
        .entry(key)
        .or_insert_with(|| {
            let lut_size = 1u32 << bits;
            let mut lut = Vec::with_capacity(lut_size as usize);
            
            for i in 0..lut_size {
                let value = i as f32 / lut_size as f32;
                let toned = mobius_tonemap(value, strength_milli as f32 / 1000.0);
                lut.push(toned);
            }
            
            lut
        }).clone()
}

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
    /// Mollweide equal-area projection.
    Mollweide,
    /// Hammer azimuthal equal-area projection.
    Hammer,
    /// Stereographic fisheye, distinct from the little-planet remap.
    StereographicFisheye,
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
        SphericalProjection::AzimuthalEquidistant => SphericalProjection::Mollweide,
        SphericalProjection::Mollweide => SphericalProjection::Hammer,
        SphericalProjection::Hammer => SphericalProjection::StereographicFisheye,
        SphericalProjection::StereographicFisheye => SphericalProjection::Equirect,
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
        SphericalProjection::Mollweide => "Mollweide",
        SphericalProjection::Hammer => "Hammer",
        SphericalProjection::StereographicFisheye => "Stereo fisheye",
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
        SphericalProjection::Mollweide => project_mollweide_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Hammer => project_hammer_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::StereographicFisheye => project_stereographic_fisheye_view(
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

/// Mollweide equal-area projection (approximate inverse).
pub fn project_mollweide_view(
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
        let y = (pitch + ny * half).clamp(-1.0, 1.0);
        let mut theta = y;
        for _ in 0..4 {
            let f = 2.0 * theta + theta.sin() - std::f32::consts::PI * y;
            let df = 2.0 + theta.cos();
            theta -= f / df.max(1e-3);
        }
        let lat = theta.asin().clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        let cos_t = theta.cos().max(1e-3);
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + (nx * half * aspect * std::f32::consts::SQRT_2) / cos_t;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Hammer equal-area azimuthal projection.
pub fn project_hammer_view(
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
            let x = nx * half * aspect;
            let y = pitch + ny * half;
            let z2 = 1.0 - (x * x) / 8.0 - (y * y) / 2.0;
            if z2 <= 0.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let z = z2.sqrt();
            let lon = yaw + 2.0 * (z * x / (2.0 * (2.0 * z2 - 1.0).max(1e-3))).atan();
            let lat = (y * z).asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Stereographic fisheye: r = 2*tan(theta/2).
pub fn project_stereographic_fisheye_view(
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
    let r_max = 2.0 * (fov * 0.5).tan().max(1e-6);
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
            let theta = 2.0 * (r * r_max * 0.5).atan();
            let phi = ny.atan2(nx);
            let x_cam = theta.sin() * phi.cos();
            let y_cam = theta.sin() * phi.sin();
            let z_cam = theta.cos();
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

/// Scale content by MaxCLL versus display peak (HDR content-light mapping).
pub fn content_light_scale_milli(maxcll: u32, display_peak_nits: u32) -> u32 {
    let content = clamp_hdr_maxcll(maxcll).max(1);
    let display = clamp_hdr_nits(display_peak_nits).max(1);
    ((display as u64 * 1_000) / content as u64).min(4_000) as u32
}

pub fn format_content_light_scale_osd(scale_milli: u32) -> String {
    format!("CLL scale ×{:.2}", scale_milli as f32 / 1_000.0)
}

/// Estimate an ambient light compensation gain.
pub fn ambient_compensation_gain_milli(ambient_nits: u32, reference_nits: u32) -> u32 {
    let amb = ambient_nits.clamp(1, 10_000);
    let refer = reference_nits.clamp(1, 1_000);
    let ratio = (amb as f32 / refer as f32).sqrt().clamp(0.5, 2.0);
    (ratio * 1_000.0).round() as u32
}

pub fn format_ambient_compensation_osd(gain_milli: u32) -> String {
    format!("Ambient ×{:.2}", gain_milli as f32 / 1_000.0)
}

pub fn apply_ambient_gain_channel(value: u8, gain_milli: u32) -> u8 {
    let gain = gain_milli.clamp(250, 2_000);
    ((u32::from(value) * gain) / 1_000).min(255) as u8
}

pub fn hdr_tonemap_modes_differ(a: HdrTonemap, b: HdrTonemap) -> bool {
    if a == b {
        return false;
    }
    apply_hdr_tonemap_pixel(180, 180, 180, a, COLOR_TRC_SMPTE2084)
        != apply_hdr_tonemap_pixel(180, 180, 180, b, COLOR_TRC_SMPTE2084)
}

pub fn spherical_projections_differ(
    a: SphericalProjection,
    b: SphericalProjection,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) -> bool {
    if a == b || src.is_empty() || src_w == 0 || src_h == 0 {
        return false;
    }
    let va = project_spherical_view(src_w, src_h, src, 8, 8, a, 30_000, 10_000, 0, 90_000);
    let vb = project_spherical_view(src_w, src_h, src, 8, 8, b, 30_000, 10_000, 0, 90_000);
    va != vb
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
        "mollweide" => Ok(SphericalProjection::Mollweide),
        "hammer" | "hammer-aitoff" => Ok(SphericalProjection::Hammer),
        "stereographic-fisheye" | "stereo-fisheye" | "sg-fisheye" => {
            Ok(SphericalProjection::StereographicFisheye)
        }
        other => Err(format!(
            "unknown spherical projection `{other}` (equirect|dual-fisheye|cubemap|little-planet|eac|panini|cylindrical|mercator|dual-fisheye-tb|octahedral|equisolid|orthographic|gnomonic|sinusoidal|miller|azimuthal|mollweide|hammer|stereographic-fisheye)"
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

