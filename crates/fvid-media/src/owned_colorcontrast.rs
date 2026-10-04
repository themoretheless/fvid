//! Owned RGB opponent-colour contrast processing.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Default)]
pub struct ColorContrast {
    values: [f32; 7],
}
impl ColorContrast {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["rc", "gm", "by", "rcw", "gmw", "byw", "pl"];
        let mut filter = Self::default();
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *names
                    .get(position)
                    .ok_or("too many colorcontrast options")?;
                position += 1;
                (name, option)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown colorcontrast option")?;
            let value = crate::owned_expression::constant(value.trim())? as f32;
            let minimum = if index < 3 { -1.0 } else { 0.0 };
            if !value.is_finite() || !(minimum..=1.0).contains(&value) {
                return Err("invalid colorcontrast parameter".into());
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
            return Err("invalid colorcontrast RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = bytes * channels;
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % stride != 0 {
            return Err("colorcontrast RGB length mismatch".into());
        }
        // Validate all samples before mutating any pixel.
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("colorcontrast sample exceeds precision".into());
        }
        let [rc, gm, by, rcw, gmw, byw, preserve] = self.values;
        let sum = gmw + byw + rcw;
        if sum <= f32::EPSILON {
            return Ok(());
        }
        let scale = 1.0 / sum;
        for pixel in data.chunks_exact_mut(stride) {
            let rgb: [f32; 3] = std::array::from_fn(|i| {
                if bytes == 1 {
                    pixel[i] as f32
                } else {
                    u16::from_le_bytes([pixel[2 * i], pixel[2 * i + 1]]) as f32
                }
            });
            let [r, g, b] = rgb;
            let gd = (g - (b + r) * 0.5) * (gm * 0.5);
            let bd = (b - (r + g) * 0.5) * (by * 0.5);
            let rd = (r - (g + b) * 0.5) * (rc * 0.5);
            let adjusted = [
                ((r - gd) * gmw + (r - bd) * byw + (r + rd) * rcw) * scale,
                ((g + gd) * gmw + (g - bd) * byw + (g - rd) * rcw) * scale,
                ((b - gd) * gmw + (b + bd) * byw + (b - rd) * rcw) * scale,
            ]
            .map(|v| v.clamp(0.0, maximum));
            let light = (r.max(g).max(b) + r.min(g).min(b))
                / (adjusted[0].max(adjusted[1]).max(adjusted[2])
                    + adjusted[0].min(adjusted[1]).min(adjusted[2])
                    + f32::EPSILON);
            for i in 0..3 {
                let value = (adjusted[i] + (adjusted[i] * light - adjusted[i]) * preserve)
                    .clamp(0.0, maximum) as u16;
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
mod tests {
    use super::*;
    #[test]
    fn opponent_contrast_preserves_alpha_and_lightness() {
        let mut pixel = [200, 100, 50, 17];
        ColorContrast::parse("rc=1:rcw=1")
            .unwrap()
            .apply_rgb(&mut pixel, 8, 4)
            .unwrap();
        assert_eq!(pixel, [255, 37, 0, 17]);
        let mut pixel = [200, 100, 50, 17];
        ColorContrast::parse("rc=1:rcw=1:pl=1")
            .unwrap()
            .apply_rgb(&mut pixel, 8, 4)
            .unwrap();
        assert_eq!(pixel, [250, 36, 0, 17]);
    }
    #[test]
    fn zero_weights_are_identity_and_invalid_input_is_atomic() {
        let mut data = [200, 100, 50];
        ColorContrast::parse("")
            .unwrap()
            .apply_rgb(&mut data, 8, 3)
            .unwrap();
        assert_eq!(data, [200, 100, 50]);
        for args in ["rc=2", "rcw=-1", "pl=nan", "unknown=1"] {
            assert!(ColorContrast::parse(args).is_err());
        }
        let mut data = [200, 100, 50, 1];
        let before = data;
        assert!(ColorContrast::parse("rcw=1")
            .unwrap()
            .apply_rgb(&mut data, 8, 3)
            .is_err());
        assert_eq!(data, before);
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
                colorcontrast: Some("rc=1:rcw=1:pl=0.5".into()),
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
                colorcontrast: Some("rc=1:rcw=1:pl=0.5".into()),
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
