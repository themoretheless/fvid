//! Owned channel black/white point adjustment and frame-wide automatic ranges.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct ColorLevels {
    ranges: [[f64; 4]; 4],
    preserve: u8,
}
impl Default for ColorLevels {
    fn default() -> Self {
        Self {
            ranges: [[0.0, 1.0, 0.0, 1.0]; 4],
            preserve: 0,
        }
    }
}
impl ColorLevels {
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "rimin", "gimin", "bimin", "aimin", "rimax", "gimax", "bimax", "aimax", "romin",
            "gomin", "bomin", "aomin", "romax", "gomax", "bomax", "aomax", "preserve",
        ];
        let mut result = Self::default();
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *names.get(position).ok_or("too many colorlevels options")?;
                position += 1;
                (name, option)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown colorlevels option")?;
            if index == 16 {
                let modes = ["none", "lum", "max", "avg", "sum", "nrm", "pwr"];
                let mode = if let Some(mode) = modes.iter().position(|n| *n == value.trim()) {
                    mode as f64
                } else {
                    crate::owned_expression::constant(value.trim())?
                };
                if !mode.is_finite() || mode.fract() != 0.0 || !(0.0..=6.0).contains(&mode) {
                    return Err("invalid colorlevels preserve mode".into());
                }
                result.preserve = mode as u8;
            } else {
                let value = crate::owned_expression::constant(value.trim())?;
                let minimum = if index < 8 { -1.0 } else { 0.0 };
                if !value.is_finite() || !(minimum..=1.0).contains(&value) {
                    return Err("invalid colorlevels point".into());
                }
                result.ranges[index % 4][index / 4] = value;
            }
        }
        Ok(result)
    }
    pub fn apply_yuv(
        &self,
        frame: &mut crate::owned_frame::GeometryFrame,
        depth: u8,
        full: bool,
        matrix: crate::owned_yuv_rgb::Matrix,
    ) -> Result<()> {
        if self.ranges.iter().any(|r| r[0] < 0.0 || r[1] < 0.0) {
            crate::owned_yuv_rgb::filter_rgb16_frame(frame, depth, full, matrix, |rgb| {
                self.apply_rgb(rgb, 16, 3)
            })
        } else {
            crate::owned_yuv_rgb::filter_rgb16_sampled(
                frame,
                depth,
                full,
                matrix,
                crate::owned_yuv_rgb::ChromaSampling::Point,
                |rgb| self.apply_rgb(rgb, 16, 3),
            )
        }
    }
    /// Applies to one complete packed RGB/RGBA frame. Automatic limits are per channel.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid colorlevels RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = bytes * channels;
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % stride != 0 {
            return Err("colorlevels RGB length mismatch".into());
        }
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("colorlevels sample exceeds precision".into());
        }
        let read = |pixel: &[u8], channel: usize| -> f32 {
            if bytes == 1 {
                pixel[channel] as f32
            } else {
                u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]) as f32
            }
        };
        let mut limits = [[0.0f32; 3]; 4];
        for channel in 0..channels {
            let [low, high, out_low, out_high] =
                self.ranges[channel].map(|v| (v * maximum as f64).round_ties_even() as f32);
            let low = if low < 0.0 {
                data.chunks_exact(stride)
                    .map(|p| read(p, channel))
                    .fold(maximum, f32::min)
            } else {
                low
            };
            let high = if high < 0.0 {
                data.chunks_exact(stride)
                    .map(|p| read(p, channel))
                    .fold(0.0, f32::max)
            } else {
                high
            };
            let coefficient = if high == low {
                f32::NAN
            } else {
                ((out_high - out_low) as f64 / (high - low) as f64) as f32
            };
            limits[channel] = [low, out_low, coefficient];
        }
        for pixel in data.chunks_exact_mut(stride) {
            let original: [f32; 3] = std::array::from_fn(|i| read(pixel, i));
            let mut adjusted: [f32; 4] = std::array::from_fn(|i| {
                if i < channels {
                    let [low, out, coefficient] = limits[i];
                    (read(pixel, i) - low).mul_add(coefficient, out) as i32 as f32
                } else {
                    0.0
                }
            });
            if self.preserve != 0 {
                let measure = |rgb: [f32; 3]| -> f32 {
                    let [r, g, b] = rgb;
                    match self.preserve {
                        1 => r.max(g).max(b) + r.min(g).min(b),
                        2 => r.max(g).max(b),
                        3 => (r + g + b + 1.0) / 3.0,
                        4 => r + g + b,
                        5 => {
                            let [r, g, b] = rgb.map(|v| v / maximum);
                            (r * r + g * g + b * b).sqrt()
                        }
                        _ => {
                            let [r, g, b] = rgb.map(|v| v / maximum);
                            (r * r * r + g * g * g + b * b * b).cbrt()
                        }
                    }
                };
                let output = measure([adjusted[0], adjusted[1], adjusted[2]]);
                if output > 0.0 {
                    let ratio = measure(original) / output;
                    for value in &mut adjusted[..3] {
                        *value = (*value * ratio) as i32 as f32;
                    }
                }
            }
            for i in 0..channels {
                let value = adjusted[i].clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    pixel[i] = value as u8
                } else {
                    pixel[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_ranges_use_whole_frame_and_alpha_can_be_adjusted() {
        let mut pixels = [50, 10, 30, 20, 150, 110, 230, 220];
        ColorLevels::parse(
            "rimin=-1:rimax=-1:gimin=-1:gimax=-1:bimin=-1:bimax=-1:aimin=-1:aimax=-1",
        )
        .unwrap()
        .apply_rgb(&mut pixels, 8, 4)
        .unwrap();
        assert_eq!(pixels, [0, 0, 0, 0, 255, 255, 255, 255]);
    }
    #[test]
    fn identity_and_invalid_input_are_atomic() {
        let mut pixels = [23, 81, 171, 19];
        ColorLevels::parse("")
            .unwrap()
            .apply_rgb(&mut pixels, 8, 4)
            .unwrap();
        assert_eq!(pixels, [23, 81, 171, 19]);
        for args in [
            "rimax=2",
            "romin=-1",
            "preserve=7",
            "preserve=0.5",
            "aimin=nan",
        ] {
            assert!(ColorLevels::parse(args).is_err());
        }
        let before = pixels;
        assert!(ColorLevels::parse("")
            .unwrap()
            .apply_rgb(&mut pixels, 8, 3)
            .is_err());
        assert_eq!(pixels, before);
    }
}

#[cfg(test)]
mod synthetic_acceptance {
    #[test]
    fn synthetic_video_decodes_and_transforms_all_frames_without_legacy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let source = root.join(format!("colorize-grid-{depth}.y4m"));
            let transform = fvid_media_info::DecodeTransform {
                colorlevels: Some("romin=0.1:bomax=0.7".into()),
                ..Default::default()
            };
            assert!(crate::owned_y4m_decode::supported_request(&transform));
            let stats =
                crate::owned_y4m_decode::decode_video_transformed(&source, transform).unwrap();
            assert_eq!(stats.video_frames, 3);
            let input = std::fs::read(&source).unwrap();
            let collect = |request: &fvid_media_info::DecodeTransform| {
                let mut frames = Vec::new();
                crate::owned_y4m_decode::visit_reader_transformed(
                    std::io::Cursor::new(&input),
                    request,
                    |_, data, _, _| {
                        frames.push(data.to_vec());
                        Ok(())
                    },
                )
                .unwrap();
                frames
            };
            let original = collect(&Default::default());
            let transformed = collect(&fvid_media_info::DecodeTransform {
                colorlevels: Some("romin=0.1:bomax=0.7".into()),
                ..Default::default()
            });
            assert_eq!(original.len(), transformed.len());
            for (before, after) in original.iter().zip(&transformed) {
                assert_eq!(before.len(), after.len());
                assert_ne!(before, after);
            }
        }
    }
}

#[cfg(test)]
mod rgb_reference_acceptance {
    #[test]
    fn three_frame_synthetic_rgba_matches_committed_references() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 16] {
            let input = std::fs::read(root.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
            assert_eq!(input.len(), 3 * 8 * 8 * 4 * if depth == 8 { 1 } else { 2 });
            for (case, args) in [
                "",
                "rimin=0.2:rimax=0.8",
                "romin=0.1:bomax=0.7",
                "aimin=0.2:aimax=0.8:aomin=0.1:aomax=0.9",
                "rimin=-1:rimax=-1:gimin=-1:gimax=-1:bimin=-1:bimax=-1:aimin=-1:aimax=-1",
                "rimin=0.8:rimax=0.2",
                "romin=0.1:bomax=0.7:preserve=lum",
                "romin=0.1:bomax=0.7:preserve=max",
                "romin=0.1:bomax=0.7:preserve=avg",
                "romin=0.1:bomax=0.7:preserve=sum",
                "romin=0.1:bomax=0.7:preserve=nrm",
                "romin=0.1:bomax=0.7:preserve=pwr",
            ]
            .into_iter()
            .enumerate()
            {
                let mut actual = input.clone();
                for frame in actual.chunks_exact_mut(8 * 8 * 4 * if depth == 8 { 1 } else { 2 }) {
                    super::ColorLevels::parse(args)
                        .unwrap()
                        .apply_rgb(frame, depth, 4)
                        .unwrap();
                }
                let expected =
                    std::fs::read(root.join(format!("colorlevels-reference-{depth}-{case}.raw")))
                        .unwrap();
                assert_eq!(actual, expected, "depth={depth} case={case}");
            }
        }
    }
}
