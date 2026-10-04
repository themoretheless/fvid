//! Owned RGB adaptive colour saturation processing.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Vibrance {
    values: [f32; 8],
}
impl Default for Vibrance {
    fn default() -> Self {
        Self {
            values: [0.0, 1.0, 1.0, 1.0, 0.212656, 0.715158, 0.072186, 0.0],
        }
    }
}
impl Vibrance {
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "intensity",
            "rbal",
            "gbal",
            "bbal",
            "rlum",
            "glum",
            "blum",
            "alternate",
        ];
        let mut filter = Self::default();
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *names.get(position).ok_or("too many vibrance options")?;
                position += 1;
                (name, option)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown vibrance option")?;
            let value = if index == 7 {
                match value.trim() {
                    "true" | "yes" | "on" => 1.0,
                    "false" | "no" | "off" => 0.0,
                    _ => crate::owned_expression::constant(value.trim())? as f32,
                }
            } else {
                crate::owned_expression::constant(value.trim())? as f32
            };
            let (minimum, maximum) = match index {
                0 => (-2.0, 2.0),
                1..=3 => (-10.0, 10.0),
                _ => (0.0, 1.0),
            };
            if !value.is_finite()
                || !(minimum..=maximum).contains(&value)
                || (index == 7 && value != 0.0 && value != 1.0)
            {
                return Err("invalid vibrance parameter".into());
            }
            filter.values[index] = value;
        }
        Ok(filter)
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
    /// Packed RGB/RGBA, with little-endian samples above eight bits. Alpha is preserved.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid vibrance RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = bytes * channels;
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % stride != 0 {
            return Err("vibrance RGB length mismatch".into());
        }
        // Validate all samples before mutating any pixel.
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("vibrance sample exceeds precision".into());
        }
        let [intensity, rbal, gbal, bbal, rlum, glum, blum, alternate] = self.values;
        let balance = [rbal, gbal, bbal].map(|v| v * intensity);
        let polarity = if alternate == 1.0 { 1.0 } else { -1.0 };
        let scale = 1.0 / maximum;
        for pixel in data.chunks_exact_mut(stride) {
            let rgb: [f32; 3] = std::array::from_fn(|i| {
                if bytes == 1 {
                    pixel[i] as f32
                } else {
                    u16::from_le_bytes([pixel[2 * i], pixel[2 * i + 1]]) as f32
                }
            });
            let [r, g, b] = rgb.map(|v| v * scale);
            let saturation = r.max(g).max(b) - r.min(g).min(b);
            // Preserve one rounded red product followed by fused green accumulation.
            let luma = g.mul_add(glum, r * rlum) + b * blum;
            let adjusted: [f32; 3] = std::array::from_fn(|i| {
                let sign = if balance[i] > 0.0 {
                    1.0
                } else if balance[i] < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                let factor = 1.0 + balance[i] * (1.0 - polarity * sign * saturation);
                (luma + ([r, g, b][i] - luma) * factor) * maximum
            });
            for i in 0..3 {
                let value = adjusted[i].clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    pixel[i] = value as u8;
                } else {
                    pixel[2 * i..2 * i + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
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
                vibrance: Some("intensity=1".into()),
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
                vibrance: Some("intensity=1".into()),
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
mod tests {
    use super::*;
    #[test]
    fn channel_balance_preserves_alpha_and_validates_before_mutation() {
        let mut data = [255, 0, 0, 17];
        Vibrance::parse("intensity=1:rbal=0:gbal=0:bbal=0")
            .unwrap()
            .apply_rgb(&mut data, 8, 4)
            .unwrap();
        assert_eq!(data, [255, 0, 0, 17]);
        let mut data = [127, 127, 127, 19];
        Vibrance::parse("intensity=2")
            .unwrap()
            .apply_rgb(&mut data, 8, 4)
            .unwrap();
        assert_eq!(data, [127, 127, 127, 19]);
        for args in [
            "intensity=3",
            "rbal=-11",
            "rlum=2",
            "alternate=0.5",
            "glum=nan",
        ] {
            assert!(Vibrance::parse(args).is_err());
        }
        let mut data = [200, 100, 50, 1];
        let before = data;
        assert!(Vibrance::parse("1")
            .unwrap()
            .apply_rgb(&mut data, 8, 3)
            .is_err());
        assert_eq!(data, before);
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
                "intensity=1",
                "intensity=-1",
                "intensity=2:alternate=1",
                "intensity=0.5:rbal=-1:gbal=0.2:bbal=2",
                "intensity=-0.7:rlum=0.3:glum=0.6:blum=0.1",
            ]
            .into_iter()
            .enumerate()
            {
                let mut actual = input.clone();
                super::Vibrance::parse(args)
                    .unwrap()
                    .apply_rgb(&mut actual, depth, 4)
                    .unwrap();
                let expected =
                    std::fs::read(root.join(format!("vibrance-reference-{depth}-{case}.raw")))
                        .unwrap();
                assert_eq!(actual, expected, "depth={depth} case={case}");
            }
        }
    }
}
