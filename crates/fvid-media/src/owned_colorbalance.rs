//! Owned shadow, midtone and highlight RGB balancing.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Default)]
pub struct ColorBalance {
    adjustments: [f32; 9],
    preserve: bool,
}
impl ColorBalance {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["rs", "gs", "bs", "rm", "gm", "bm", "rh", "gh", "bh", "pl"];
        let mut result = Self::default();
        let mut position = 0;
        for part in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = part.split_once('=') {
                pair
            } else {
                let name = *names.get(position).ok_or("too many colorbalance options")?;
                position += 1;
                (name, part)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown colorbalance option")?;
            if index == 9 {
                result.preserve = match value.trim() {
                    "1" | "true" | "yes" | "on" => true,
                    "0" | "false" | "no" | "off" => false,
                    _ => return Err("invalid preserve lightness flag".into()),
                };
            } else {
                let value = crate::owned_expression::constant(value.trim())? as f32;
                if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
                    return Err("invalid colorbalance adjustment".into());
                }
                result.adjustments[index] = value;
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
    /// RGB/RGBA, little endian above eight bits; alpha remains unchanged.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid colorbalance RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % (channels * bytes) != 0 {
            return Err("colorbalance RGB length mismatch".into());
        }
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|s| u16::from_le_bytes([s[0], s[1]]) as f32 > maximum)
        {
            return Err("colorbalance sample exceeds precision".into());
        }
        for pixel in data.chunks_exact_mut(channels * bytes) {
            let input: [f32; 3] = std::array::from_fn(|i| {
                if bytes == 1 {
                    pixel[i] as f32 / maximum
                } else {
                    u16::from_le_bytes([pixel[2 * i], pixel[2 * i + 1]]) as f32 / maximum
                }
            });
            let lightness =
                input[0].max(input[1]).max(input[2]) + input[0].min(input[1]).min(input[2]);
            let shadow = ((0.333 - lightness) * 4.0 + 0.5).clamp(0.0, 1.0) * 0.7;
            let middle = ((lightness - 0.333) * 4.0 + 0.5).clamp(0.0, 1.0)
                * ((1.0 - lightness - 0.333) * 4.0 + 0.5).clamp(0.0, 1.0)
                * 0.7;
            let highlight = ((lightness + 0.333 - 1.0) * 4.0 + 0.5).clamp(0.0, 1.0) * 0.7;
            let mut output: [f32; 3] = std::array::from_fn(|i| {
                let mut v = input[i];
                v += self.adjustments[i] * shadow;
                v += self.adjustments[i + 3] * middle;
                v += self.adjustments[i + 6] * highlight;
                v.clamp(0.0, 1.0)
            });
            if self.preserve {
                output = restore_lightness(output, lightness * 0.5);
            }
            for i in 0..3 {
                let v = (output[i] * maximum).round_ties_even().clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    pixel[i] = v as u8;
                } else {
                    pixel[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}
fn restore_lightness(rgb: [f32; 3], lightness: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    let top = r.max(g).max(b);
    let bottom = r.min(g).min(b);
    let mut hue = if top == bottom {
        0.0
    } else if top == r {
        60.0 * ((g - b) / (top - bottom))
    } else if top == g {
        60.0 * (2.0 + (b - r) / (top - bottom))
    } else {
        60.0 * (4.0 + (r - g) / (top - bottom))
    };
    if hue < 0.0 {
        hue += 360.0;
    }
    let saturation = if top == 1.0 || bottom == 0.0 {
        0.0
    } else {
        (top - bottom) / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    let amplitude = saturation * lightness.min(1.0 - lightness);
    [0.0, 8.0, 4.0].map(|offset| {
        let phase = (offset + hue / 30.0) % 12.0;
        (lightness - amplitude * (phase - 3.0).min(9.0 - phase).min(1.0).max(-1.0)).clamp(0.0, 1.0)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_alpha_and_atomic_validation() {
        let mut pixels = [30, 70, 120, 17, 200, 210, 220, 99];
        let before = pixels;
        ColorBalance::parse("")
            .unwrap()
            .apply_rgb(&mut pixels, 8, 4)
            .unwrap();
        assert_eq!(pixels, before);
        ColorBalance::parse("rs=1:gm=-0.5:bh=1:pl=true")
            .unwrap()
            .apply_rgb(&mut pixels, 8, 4)
            .unwrap();
        assert_eq!(pixels[3], 17);
        assert_eq!(pixels[7], 99);
        let mut bad = [1, 0, 2, 0, 255, 255];
        let original = bad;
        assert!(
            ColorBalance::parse("")
                .unwrap()
                .apply_rgb(&mut bad, 12, 3)
                .is_err()
        );
        assert_eq!(bad, original);
        for args in ["rs=2", "gs=nan", "pl=maybe", "unknown=1"] {
            assert!(ColorBalance::parse(args).is_err());
        }
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
                colorbalance: Some("rs=0.2:gm=-0.1:bh=0.3:pl=1".into()),
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
                colorbalance: Some("rs=0.2:gm=-0.1:bh=0.3:pl=1".into()),
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
mod reference_acceptance {
    #[test]
    fn synthetic_rgba_video_matches_saved_reference() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 16] {
            let source = std::fs::read(root.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
            for (case, args) in [
                "",
                "rs=1:gs=-1:bs=0.2",
                "rm=0.7:gm=-0.3:bm=0.1",
                "rh=-0.8:gh=0.6:bh=0.2",
                "rs=0.3:gm=-0.2:bh=0.4:pl=1",
                "rs=1:gs=1:bs=-1:rm=-1:gm=1:bm=1:rh=1:gh=-1:bh=1:pl=1",
            ]
            .into_iter()
            .enumerate()
            {
                let mut actual = source.clone();
                super::ColorBalance::parse(args)
                    .unwrap()
                    .apply_rgb(&mut actual, depth, 4)
                    .unwrap();
                let reference =
                    std::fs::read(root.join(format!("colorbalance-reference-{depth}-{case}.raw")))
                        .unwrap();
                assert_eq!(actual, reference, "depth={depth} args={args}");
            }
        }
    }
}
