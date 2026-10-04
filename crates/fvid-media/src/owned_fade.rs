//! Frame-count and time-based fade over owned packed RGB and planar YUV samples.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;

/// Exact frame presentation clock; scale is ticks per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTime {
    pub ticks: i128,
    pub scale: u64,
    pub quantum: u64,
}
impl FrameTime {
    pub fn new(ticks: u128, scale: u64) -> Result<Self> {
        if scale == 0 {
            return Err("zero fade clock".into());
        }
        Ok(Self {
            ticks: i128::try_from(ticks).map_err(|_| "fade timestamp overflow")?,
            scale,
            quantum: 1,
        })
    }
    pub fn from_seconds(seconds: f64) -> Result<Self> {
        let ticks = seconds * 1e9;
        if !ticks.is_finite() || ticks.abs() >= i128::MAX as f64 {
            return Err("invalid fade timestamp".into());
        }
        Ok(Self {
            ticks: ticks.round() as i128,
            scale: 1_000_000_000,
            quantum: 1,
        })
    }
    pub fn with_quantum(mut self, quantum: u64) -> Result<Self> {
        if quantum == 0 {
            return Err("zero fade clock quantum".into());
        }
        self.quantum = quantum;
        Ok(self)
    }
    pub fn seconds(self) -> f64 {
        self.ticks as f64 / self.scale as f64
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FadeState {
    phase: Option<(u64, i128, u64, u64)>,
    done: bool,
}
/// A per-stream clock context; do not parse/reinitialize it for every frame.
pub struct FadeClock {
    filter: Fade,
    state: std::cell::Cell<FadeState>,
}
impl FadeClock {
    pub fn parse(args: &str) -> Result<Self> {
        Ok(Self {
            filter: Fade::parse(args)?,
            state: Default::default(),
        })
    }
    pub fn at(&self, n: u64, time: Option<FrameTime>) -> Result<Fade> {
        let mut state = self.state.get();
        let filter = self.filter.at(n, time, &mut state)?;
        self.state.set(state);
        Ok(filter)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    out: bool,
    start: u32,
    frames: u32,
    color: [u8; 3],
    start_us: u64,
    duration_us: u64,
    evaluated: Option<i64>,
}
impl Fade {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 1024 || args.contains('\0') {
            return Err("invalid fade options".into());
        }
        let mut result = Self {
            out: false,
            start: 0,
            frames: 25,
            color: [0; 3],
            start_us: 0,
            duration_us: 0,
            evaluated: None,
        };
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *[
                    "type",
                    "start_frame",
                    "nb_frames",
                    "alpha",
                    "start_time",
                    "duration",
                    "color",
                ]
                .get(position)
                .ok_or("too many fade options")?;
                position += 1;
                (name, option)
            };
            let value = value.trim();
            match name.trim() {
                "type" | "t" => {
                    result.out = match value {
                        "in" | "0" => false,
                        "out" | "1" => true,
                        _ => return Err("invalid fade direction".into()),
                    }
                }
                "start_frame" | "s" | "nb_frames" | "n" => {
                    let number = crate::owned_expression::constant(value)?;
                    let minimum = if matches!(name.trim(), "n" | "nb_frames") {
                        1.0
                    } else {
                        0.0
                    };
                    if !number.is_finite()
                        || number.fract() != 0.0
                        || !(minimum..=i32::MAX as f64).contains(&number)
                    {
                        return Err("invalid fade frame parameter".into());
                    }
                    if matches!(name.trim(), "n" | "nb_frames") {
                        result.frames = number as u32;
                    } else {
                        result.start = number as u32;
                    }
                }
                "alpha" => {
                    if value != "0" && value != "false" {
                        return Err("owned fade alpha mode is not implemented".into());
                    }
                }
                "start_time" | "st" | "duration" | "d" => {

                    let number = parse_control_time(value)?;
                    if matches!(name.trim(), "start_time" | "st") {
                        result.start_us = number;
                    } else {
                        result.duration_us = number;
                    }
                }
                "color" | "c" => {
                    // Alpha fades remain separately unsupported; do not silently
                    // reinterpret a translucent fade color as opaque.
                    let (rgb, opacity) = value
                        .split_once('@')
                        .map_or((value, None), |(c, a)| (c, Some(a)));
                    if let Some(opacity) = opacity {
                        let opaque = if let Some(hex) = opacity.strip_prefix("0x") {
                            u32::from_str_radix(hex, 16).ok() == Some(255)
                        } else {
                            opacity.parse::<f64>().ok() == Some(1.0)
                        };
                        if !opaque {
                            return Err("owned fade translucent colors are not implemented".into());
                        }
                    }
                    let hex = rgb
                        .strip_prefix('#')
                        .or_else(|| rgb.strip_prefix("0x"))
                        .unwrap_or(rgb);
                    if hex.len() == 8
                        && hex.bytes().all(|b| b.is_ascii_hexdigit())
                        && !hex[6..].eq_ignore_ascii_case("ff")
                    {
                        return Err("owned fade translucent colors are not implemented".into());
                    }
                    result.color = crate::owned_colorhold::parse_color(value)?;
                }
                _ => return Err("unknown fade option".into()),
            }
        }
        Ok(result)
    }

    pub fn has_time(self) -> bool {
        self.start_us != 0 || self.duration_us != 0
    }
    /// Rescale decimal microsecond options into the supplied source time base,
    /// rounding half ticks upward. Keep the start gates and irreversible end state.
    pub fn at(self, n: u64, time: Option<FrameTime>, state: &mut FadeState) -> Result<Self> {
        let mut candidate = *state;
        let evaluated = self.at_inner(n, time, &mut candidate)?;
        *state = candidate;
        Ok(evaluated)
    }
    fn at_inner(mut self, n: u64, time: Option<FrameTime>, state: &mut FadeState) -> Result<Self> {
        if !self.has_time() {
            return Ok(self);
        }
        let clock = time.ok_or("time-based fade requires a presentation clock")?;
        if clock.scale == 0 {
            return Err("zero fade clock".into());
        }
        if n == 0 {
            *state = Default::default();
        }
        let ticks = |us: u64| -> Result<i128> {
            if clock.quantum == 0 {
                return Err("zero fade clock quantum".into());
            }
            let denominator = 1_000_000u128 * u128::from(clock.quantum);
            let value = u128::from(us)
                .checked_mul(u128::from(clock.scale))
                .and_then(|v| v.checked_add(denominator / 2))
                .ok_or("fade option clock overflow")?
                / denominator;
            let value = value
                .checked_mul(u128::from(clock.quantum))
                .ok_or("fade option clock overflow")?;
            i128::try_from(value).map_err(|_| "fade option clock overflow".into())
        };
        let start = ticks(self.start_us)?;
        let duration = ticks(self.duration_us)?;
        if let Some((_, _, scale, quantum)) = state.phase {
            if scale != clock.scale || quantum != clock.quantum {
                return Err("fade time base changed within a stream".into());
            }
        }
        if state.phase.is_none() && clock.ticks >= start && n >= u64::from(self.start) {
            let time_start = if start == 0 && self.start != 0 {
                clock.ticks
            } else {
                start
            };
            let frame_start = if start != 0 && self.start == 0 {
                n
            } else {
                u64::from(self.start)
            };
            state.phase = Some((frame_start, time_start, clock.scale, clock.quantum));
        }
        let amount = if state.done {
            65535
        } else if let Some((frame_start, time_start, _, _)) = state.phase {
            if duration == 0 {
                let length = if self.duration_us != 0 {
                    0
                } else {
                    u64::from(self.frames)
                };
                let elapsed = n.saturating_sub(frame_start);
                if elapsed > length {
                    state.done = true;
                    65535
                } else {
                    (elapsed * (65536 / u64::from(self.frames))).min(65535) as i64
                }
            } else {
                let elapsed = clock
                    .ticks
                    .checked_sub(time_start)
                    .ok_or("fade clock subtraction overflow")?;
                if elapsed > duration {
                    state.done = true;
                    65535
                } else {
                    let value = elapsed
                        .checked_mul(65535)
                        .ok_or("fade clock multiplication overflow")?
                        / duration;
                    value.clamp(0, 65535) as i64
                }
            }
        } else {
            0
        };
        self.evaluated = Some(if self.out { 65535 - amount } else { amount });
        Ok(self)
    }
    fn require_clock(self) -> Result<()> {
        if self.has_time() && self.evaluated.is_none() {
            return Err("time-based fade requires a stream clock context".into());
        }
        Ok(())
    }
    fn factor(self, n: u64) -> i64 {

        if let Some(factor) = self.evaluated {
            return factor;
        }
        let elapsed = n.saturating_sub(u64::from(self.start));
        let amount = if n < u64::from(self.start) {
            0
        } else if elapsed > u64::from(self.frames) {
            65535
        } else {
            (elapsed * (65536 / u64::from(self.frames))).min(65535) as i64
        };
        if self.out { 65535 - amount } else { amount }
    }
    /// Packed RGB/RGBA, at 8 or 16 bits. Alpha is retained unchanged.
    pub fn apply_rgb(self, data: &mut [u8], depth: u8, channels: usize, n: u64) -> Result<()> {

        self.require_clock()?;
        if !matches!(depth, 8 | 16) {
            return Err("colored fade RGB requires 8 or 16 bits".into());
        }
        self.apply_rgb_scaled(data, depth, channels, n, (1u32 << depth) - 1)
    }
    fn apply_rgb_scaled(
        self,
        data: &mut [u8],
        depth: u8,
        channels: usize,
        n: u64,
        white: u32,
    ) -> Result<()> {
        if !matches!(channels, 3 | 4) {
            return Err("fade requires RGB or RGBA".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if data.len() % (bytes * channels) != 0 {
            return Err("fade RGB length mismatch".into());
        }
        let factor = self.factor(n);
        if factor == 65535 {
            return Ok(());
        }
        for pixel in data.chunks_exact_mut(bytes * channels) {
            for channel in 0..3 {
                let sample = &mut pixel[channel * bytes..(channel + 1) * bytes];
                let value = if bytes == 1 {
                    i64::from(sample[0])
                } else {
                    i64::from(u16::from_le_bytes([sample[0], sample[1]]))
                };
                let target = i64::from(u32::from(self.color[channel]) * white / 255);
                let output = (target + (((value - target) * factor + 32768) >> 16)) as u16;
                if bytes == 1 {
                    sample[0] = output as u8;
                } else {
                    sample.copy_from_slice(&output.to_le_bytes());
                }
            }
        }
        Ok(())
    }
    /// Colored YUV fades use the same owned RGB16 working domain as other RGB filters.
    pub fn apply_colour(
        self,
        frame: &mut GeometryFrame,
        depth: u8,
        full: bool,
        matrix_code: u8,
        n: u64,
    ) -> Result<()> {

        self.require_clock()?;
        if self.color == [0; 3] || frame.subsampling.is_none() {
            return self.apply(frame, depth, full, n);
        }
        let matrix = crate::owned_yuv_rgb::Matrix::from_code(if matrix_code == 0 {
            6
        } else {
            matrix_code
        })?;
        let white = if full { 65535 } else { 65280 };
        crate::owned_yuv_rgb::filter_rgb16_sampled(
            frame,
            depth,
            full,
            matrix,
            crate::owned_yuv_rgb::ChromaSampling::Point,
            |data| self.apply_rgb_scaled(data, 16, 3, n, white),
        )
    }
    /// Frame index counts inputs before temporal selection. RGB is packed RGB24.
    /// Validate the entire geometry/precision before any mutation.
    pub fn apply(
        self,
        frame: &mut GeometryFrame,
        depth: u8,
        full_range: bool,
        n: u64,
    ) -> Result<()> {

        self.require_clock()?;
        if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
            return Err("invalid fade frame".into());
        }
        let luma = frame
            .width
            .checked_mul(frame.height)
            .ok_or("fade geometry overflow")?;
        let samples = if let Some([sx, sy]) = frame.subsampling {
            if !matches!(sx, 1 | 2 | 4) || !matches!(sy, 1 | 2 | 4) {
                return Err("invalid fade chroma layout".into());
            }
            frame
                .width
                .div_ceil(sx)
                .checked_mul(frame.height.div_ceil(sy))
                .and_then(|c| c.checked_mul(2))
                .and_then(|c| c.checked_add(luma))
                .ok_or("fade chroma overflow")?
        } else {
            if depth != 8 {
                return Err("owned fade packed RGB requires 8 bits".into());
            }
            luma.checked_mul(3).ok_or("fade RGB overflow")?
        };
        let bytes = if depth == 8 { 1 } else { 2 };
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("fade frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|s| u32::from(u16::from_le_bytes([s[0], s[1]])) > maximum)
        {
            return Err("fade sample exceeds precision".into());
        }
        if self.color != [0; 3] {
            if frame.subsampling.is_none() {
                return self.apply_rgb(&mut frame.data, depth, 3, n);
            }
            return self.apply_colour(frame, depth, full_range, 6, n);
        }
        let factor = self.factor(n);
        if factor == 65535 {
            return Ok(());
        }
        for (index, sample) in frame.data.chunks_exact_mut(bytes).enumerate() {
            let value = if bytes == 1 {
                i64::from(sample[0])
            } else {
                i64::from(u16::from_le_bytes([sample[0], sample[1]]))
            };
            let black = if frame.subsampling.is_none() {
                0
            } else if index >= luma {
                1i64 << (depth - 1)
            } else if full_range {
                0
            } else {
                16i64 << (depth - 8)
            };
            // The 8-bit chroma compatibility rule rounds exact half-way
            // samples downward; higher precision and luma use the Q16 half unit.
            let rounding = if frame.subsampling.is_some() && index >= luma && depth == 8 {
                32759
            } else {
                32768
            };
            let output = (black + (((value - black) * factor + rounding) >> 16)) as u16;
            if bytes == 1 {
                sample[0] = output as u8;
            } else {
                sample.copy_from_slice(&output.to_le_bytes());
            }
        }
        Ok(())
    }
}

/// Filter durations use integer microseconds; precision below a microsecond is
/// discarded, including fractional microsecond suffix values.
fn parse_control_time(text: &str) -> Result<u64> {
    let (number, scale, digits) = if let Some(number) = text.strip_suffix("ms") {
        (number, 1_000u64, 3usize)
    } else if let Some(number) = text.strip_suffix("us") {
        (number, 1u64, 0usize)
    } else {
        (text.strip_suffix('s').unwrap_or(text), 1_000_000u64, 6usize)
    };
    let number = number.strip_prefix('+').unwrap_or(number);
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if number.is_empty() || number == "."
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("fade time must be nonnegative seconds, milliseconds or microseconds".into());
    }
    let whole = if whole.is_empty() { 0 } else {
        whole.parse::<u64>().map_err(|_| "fade time overflow")?
    };
    let used = fraction.len().min(digits);
    let fraction = if used == 0 { 0 } else {
        fraction[..used].parse::<u64>().map_err(|_| "invalid fade time fraction")?
            * 10u64.pow((digits - used) as u32)
    };
    whole.checked_mul(scale).and_then(|v| v.checked_add(fraction))
        .filter(|&v| v <= i64::MAX as u64)
        .ok_or_else(|| "fade time overflow".into())
}

#[cfg(test)]
mod duration_tests {
    use super::parse_control_time;
    #[test]
    fn units_precision_and_checked_bounds() {
        for (text, expected) in [
            ("0.1s",100000), ("100ms",100000), ("100000us",100000),
            ("+100.9999ms",100999), ("1.9us",1), ("0.1000009",100000),
            (".1",100000), ("1.",1000000),
            ("9223372036854775807us",i64::MAX as u64),
            ("9223372036854.775807s",i64::MAX as u64),
        ] { assert_eq!(parse_control_time(text).unwrap(),expected,"{text}"); }
        for text in ["", ".", "s", "ms", "-1us", "1e3", "NaN", "1.2.3", "1m", "1MS",
            "9223372036854775808us", "9223372036854.775808s", "18446744073709551615s"] {
            assert!(parse_control_time(text).is_err(),"{text}");
        }
    }
}
