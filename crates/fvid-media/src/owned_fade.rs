//! Frame-count fade over owned packed RGB and planar YUV samples.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    out: bool,
    start: u32,
    frames: u32,
    color: [u8; 3],
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
                    if crate::owned_expression::constant(value)? != 0.0 {
                        return Err("owned fade time-based mode is not implemented".into());
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
    fn factor(self, n: u64) -> i64 {
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
