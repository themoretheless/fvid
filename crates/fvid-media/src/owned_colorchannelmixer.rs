//! Owned RGBA channel matrix with colour preservation.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct ColorChannelMixer {
    matrix: [[f64; 4]; 4],
    preserve: u8,
    amount: f32,
}
impl Default for ColorChannelMixer {
    fn default() -> Self {
        Self {
            matrix: std::array::from_fn(|r| {
                std::array::from_fn(|c| if r == c { 1.0 } else { 0.0 })
            }),
            preserve: 0,
            amount: 0.0,
        }
    }
}
impl ColorChannelMixer {
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "rr", "rg", "rb", "ra", "gr", "gg", "gb", "ga", "br", "bg", "bb", "ba", "ar", "ag",
            "ab", "aa", "pc", "pa",
        ];
        let mut result = Self::default();
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *names
                    .get(position)
                    .ok_or("too many colorchannelmixer options")?;
                position += 1;
                (name, option)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown colorchannelmixer option")?;
            if index == 16 {
                result.preserve = crate::owned_color_preserve::parse_mode(value)?;
            } else {
                let value = crate::owned_expression::constant(value.trim())?;
                let (low, high) = if index == 17 { (0.0, 1.0) } else { (-2.0, 2.0) };
                if !value.is_finite() || !(low..=high).contains(&value) {
                    return Err("invalid colorchannelmixer gain or amount".into());
                }
                if index == 17 {
                    result.amount = value as f32;
                } else {
                    result.matrix[index / 4][index % 4] = value;
                }
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
        crate::owned_yuv_rgb::filter_rgb16_sampled(
            frame,
            depth,
            full,
            matrix,
            crate::owned_yuv_rgb::ChromaSampling::Point,
            |rgb| self.apply_rgb(rgb, 16, 3),
        )
    }
    fn preserve(&self, input: [f32; 3], output: &mut [f32; 4], maximum: f32, integer: bool) {
        if self.preserve == 0 {
            return;
        }
        let measure = |rgb| crate::owned_color_preserve::measure(self.preserve, rgb, maximum);
        let original = [output[0], output[1], output[2]];
        let light = measure(original);
        let light = if light <= 0.0 {
            1.0 / (maximum * 2.0)
        } else {
            light
        };
        let ratio = measure(input) / light;
        for i in 0..3 {
            let clipped = if integer {
                original[i].clamp(0.0, maximum)
            } else {
                original[i]
            };
            let adjusted = clipped * ratio;
            let value = (adjusted - original[i]).mul_add(self.amount, original[i]);
            output[i] = if integer {
                value.round_ties_even()
            } else {
                value
            };
        }
    }
    /// Packed RGB/RGBA with little-endian samples above eight bits.
    /// Each matrix contribution is rounded before summation, matching integer semantics.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid colorchannelmixer RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = bytes * channels;
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % stride != 0 {
            return Err("colorchannelmixer RGB length mismatch".into());
        }
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("colorchannelmixer sample exceeds precision".into());
        }
        for pixel in data.chunks_exact_mut(stride) {
            let input: [f64; 4] = std::array::from_fn(|i| {
                if i >= channels {
                    0.0
                } else if bytes == 1 {
                    pixel[i] as f64
                } else {
                    u16::from_le_bytes([pixel[i * 2], pixel[i * 2 + 1]]) as f64
                }
            });
            let mut output: [f32; 4] = std::array::from_fn(|r| {
                self.matrix[r]
                    .iter()
                    .zip(input)
                    .map(|(gain, sample)| (gain * sample).round_ties_even() as i32)
                    .sum::<i32>() as f32
            });
            self.preserve(
                [input[0] as f32, input[1] as f32, input[2] as f32],
                &mut output,
                maximum,
                true,
            );
            for i in 0..channels {
                let value = output[i].clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    pixel[i] = value as u8;
                } else {
                    pixel[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
    }
    /// Packed float RGB/RGBA. Finite values outside the nominal 0–1 interval remain unclipped.
    pub fn apply_rgb_f32(&self, data: &mut [f32], channels: usize) -> Result<()> {
        if !matches!(channels, 3 | 4)
            || data.len() % channels != 0
            || data.iter().any(|v| !v.is_finite())
        {
            return Err("invalid colorchannelmixer float RGB frame".into());
        }
        for pixel in data.chunks_exact_mut(channels) {
            let input: [f64; 4] =
                std::array::from_fn(|i| if i < channels { pixel[i] as f64 } else { 0.0 });
            let mut output: [f32; 4] = std::array::from_fn(|r| {
                self.matrix[r]
                    .iter()
                    .zip(input)
                    .map(|(gain, sample)| gain * sample)
                    .sum::<f64>() as f32
            });
            self.preserve(
                [input[0] as f32, input[1] as f32, input[2] as f32],
                &mut output,
                1.0,
                false,
            );
            pixel.copy_from_slice(&output[..channels]);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matrix_reads_original_rgba_and_rounds_each_contribution() {
        let mut pixel = [1, 3, 5, 7];
        ColorChannelMixer::parse("rr=0:rg=1:gg=0:gr=1:ar=1:ag=0:ab=0:aa=0")
            .unwrap()
            .apply_rgb(&mut pixel, 8, 4)
            .unwrap();
        assert_eq!(pixel, [3, 1, 5, 1]);
        let mut pixel = [1, 1, 1];
        ColorChannelMixer::parse("rr=0.5:rg=0.5:rb=0.5")
            .unwrap()
            .apply_rgb(&mut pixel, 8, 3)
            .unwrap();
        assert_eq!(pixel, [0, 1, 1]);
    }
    #[test]
    fn float_matrix_keeps_headroom_and_errors_are_atomic() {
        let mut pixel = [1.5, -0.25, 0.5, 0.75];
        ColorChannelMixer::parse("rr=2")
            .unwrap()
            .apply_rgb_f32(&mut pixel, 4)
            .unwrap();
        assert_eq!(pixel, [3.0, -0.25, 0.5, 0.75]);
        for args in ["rr=3", "pa=-1", "pc=7", "pc=0.5", "ra=nan"] {
            assert!(ColorChannelMixer::parse(args).is_err());
        }
        let mut pixel = [1, 2, 3, 4];
        let before = pixel;
        assert!(ColorChannelMixer::default()
            .apply_rgb(&mut pixel, 8, 3)
            .is_err());
        assert_eq!(pixel, before);
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
                colorchannelmixer: Some("rr=0.5:rg=0.2:bb=0.7".into()),
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
                colorchannelmixer: Some("rr=0.5:rg=0.2:bb=0.7".into()),
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
    fn power_preservation_keeps_synthetic_halfway_pixels_platform_independent() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let input = std::fs::read(root.join("vibrance-grid-16.rgba")).unwrap();
        let expected = std::fs::read(root.join("colorchannelmixer-reference-16-11.raw")).unwrap();
        let mixer = super::ColorChannelMixer::parse("rr=0.5:rg=0.2:bb=0.7:pc=pwr:pa=1").unwrap();
        // These two synthetic blue samples rounded down on Windows cbrtf,
        // although the committed independent reference rounds them up.
        for pixel in [121, 168] {
            let offset = pixel * 8;
            let mut actual = input[offset..offset + 8].to_vec();
            mixer.apply_rgb(&mut actual, 16, 4).unwrap();
            assert_eq!(actual, expected[offset..offset + 8], "pixel={pixel}");
        }
    }

    #[test]
    fn three_frame_synthetic_rgba_matches_committed_references() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 16] {
            let input = std::fs::read(root.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
            assert_eq!(input.len(), 3 * 8 * 8 * 4 * if depth == 8 { 1 } else { 2 });
            for (case, args) in [
                "",
                "rr=0:rg=1:gg=0:gb=1:bb=0:br=1",
                "rr=0.5:rg=0.2:bb=0.7",
                "ra=0.5:ga=-0.5:ba=0.2:ar=0.2:ag=0.3:ab=0.5:aa=0",
                "rr=-1:rg=2:gg=0.5:gb=-0.2:bb=2",
                "rr=0.5:rg=0.5:rb=0.5",
                "rr=0.5:rg=0.2:bb=0.7:pc=lum:pa=1",
                "rr=0.5:rg=0.2:bb=0.7:pc=max:pa=0.5",
                "rr=0.5:rg=0.2:bb=0.7:pc=avg:pa=0.7",
                "rr=0.5:rg=0.2:bb=0.7:pc=sum:pa=1",
                "rr=0.5:rg=0.2:bb=0.7:pc=nrm:pa=1",
                "rr=0.5:rg=0.2:bb=0.7:pc=pwr:pa=1",
            ]
            .into_iter()
            .enumerate()
            {
                let mut actual = input.clone();
                for frame in actual.chunks_exact_mut(8 * 8 * 4 * if depth == 8 { 1 } else { 2 }) {
                    super::ColorChannelMixer::parse(args)
                        .unwrap()
                        .apply_rgb(frame, depth, 4)
                        .unwrap();
                }
                let expected = std::fs::read(
                    root.join(format!("colorchannelmixer-reference-{depth}-{case}.raw")),
                )
                .unwrap();
                assert_eq!(actual, expected, "depth={depth} case={case}");
            }
        }
    }
}

#[cfg(test)]
mod float_reference_acceptance {
    #[test]
    fn synthetic_headroom_rgba_matches_saved_float_reference_with_precision_bound() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let input: Vec<f32> = std::fs::read(root.join("vibrance-grid-8.rgba"))
            .unwrap()
            .iter()
            .map(|v| *v as f32 / 255.0 * 2.0 - 0.25)
            .collect();
        for (case, args) in [
            "",
            "rr=0:rg=1:gg=0:gb=1:bb=0:br=1",
            "rr=0.5:rg=0.2:bb=0.7",
            "ra=0.5:ga=-0.5:ba=0.2:ar=0.2:ag=0.3:ab=0.5:aa=0",
            "rr=-1:rg=2:gg=0.5:gb=-0.2:bb=2",
            "rr=0.5:rg=0.5:rb=0.5",
            "rr=0.5:rg=0.2:bb=0.7:pc=lum:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=max:pa=0.5",
            "rr=0.5:rg=0.2:bb=0.7:pc=avg:pa=0.7",
            "rr=0.5:rg=0.2:bb=0.7:pc=sum:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=nrm:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=pwr:pa=1",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            super::ColorChannelMixer::parse(args)
                .unwrap()
                .apply_rgb_f32(&mut actual, 4)
                .unwrap();
            let expected: Vec<f32> =
                std::fs::read(root.join(format!("colorchannelmixer-reference-float-{case}.raw")))
                    .unwrap()
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
            assert_eq!(actual.len(), 768);
            assert_eq!(expected.len(), actual.len());
            for (a, b) in actual.iter().zip(expected) {
                assert!(
                    a.is_finite()
                        && b.is_finite()
                        && (a - b).abs() <= 8.0 * f32::EPSILON * b.abs().max(1.0),
                    "case={case} own={a} reference={b}"
                );
            }
        }
    }
}
