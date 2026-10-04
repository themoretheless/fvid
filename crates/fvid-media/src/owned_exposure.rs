//! Owned exposure and black-point correction for float and integer RGB.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Exposure {
    black: f32,
    scale: f32,
}
impl Exposure {
    pub fn parse(args: &str) -> Result<Self> {
        let mut exposure = 0.0f32;
        let mut black = 0.0f32;
        let mut position = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let name = *["exposure", "black"]
                    .get(position)
                    .ok_or("too many exposure options")?;
                position += 1;
                (name, option)
            };
            let bound = match name.trim() {
                "exposure" => 3.0,
                "black" => 1.0,
                _ => return Err("unknown exposure option".into()),
            };
            let value = crate::owned_expression::constant(value.trim())? as f32;
            if !value.is_finite() || !(-bound..=bound).contains(&value) {
                return Err("invalid exposure parameter".into());
            }
            if name.trim() == "exposure" {
                exposure = value;
            } else {
                black = value;
            }
        }
        let difference = ((-exposure).exp2() - black).abs();
        Ok(Self {
            black,
            scale: 1.0
                / if difference > 0.0 {
                    difference
                } else {
                    1.0 / 1024.0
                },
        })
    }
    /// Packed float RGB/RGBA. RGB headroom is retained; alpha is unchanged.
    pub fn apply_rgb_f32(&self, data: &mut [f32], channels: usize) -> Result<()> {
        if !matches!(channels, 3 | 4)
            || data.len() % channels != 0
            || data.iter().any(|v| !v.is_finite())
        {
            return Err("invalid exposure float RGB frame".into());
        }
        for pixel in data.chunks_exact_mut(channels) {
            for value in &mut pixel[..3] {
                *value = (*value - self.black) * self.scale;
            }
        }
        Ok(())
    }
    /// Integer RGB/RGBA has a normalized float working domain and saturates on output.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid exposure RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = bytes * channels;
        let maximum = ((1u32 << depth) - 1) as f32;
        if data.len() % stride != 0 {
            return Err("exposure RGB length mismatch".into());
        }
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("exposure sample exceeds precision".into());
        }
        for pixel in data.chunks_exact_mut(stride) {
            for channel in 0..3 {
                let value = if bytes == 1 {
                    pixel[channel] as f32
                } else {
                    u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]]) as f32
                };
                let output = (((value / maximum - self.black) * self.scale) * maximum)
                    .round_ties_even()
                    .clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    pixel[channel] = output as u8;
                } else {
                    pixel[channel * 2..channel * 2 + 2].copy_from_slice(&output.to_le_bytes());
                }
            }
        }
        Ok(())
    }
    pub fn apply_yuv(
        &self,
        frame: &mut crate::owned_frame::GeometryFrame,
        depth: u8,
        full: bool,
        matrix: crate::owned_yuv_rgb::Matrix,
    ) -> Result<()> {
        crate::owned_yuv_rgb::filter_rgb_f32_sampled(
            frame,
            depth,
            full,
            matrix,
            crate::owned_yuv_rgb::ChromaSampling::Point,
            |rgb| self.apply_rgb_f32(rgb, 3),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn float_headroom_alpha_and_zero_denominator() {
        let mut pixel = [0.25, 0.5, 1.5, 0.75];
        Exposure::parse("1")
            .unwrap()
            .apply_rgb_f32(&mut pixel, 4)
            .unwrap();
        assert_eq!(pixel, [0.5, 1.0, 3.0, 0.75]);
        let mut pixel = [0.5, 1.0, 1.5, 0.25];
        Exposure::parse("0:1")
            .unwrap()
            .apply_rgb_f32(&mut pixel, 4)
            .unwrap();
        assert_eq!(pixel, [-512.0, 0.0, 512.0, 0.25]);
    }
    #[test]
    fn integer_clipping_and_atomic_validation() {
        let mut pixel = [32, 64, 192, 17];
        Exposure::parse("1")
            .unwrap()
            .apply_rgb(&mut pixel, 8, 4)
            .unwrap();
        assert_eq!(pixel, [64, 128, 255, 17]);
        for args in ["exposure=4", "black=2", "black=nan", "unknown=1"] {
            assert!(Exposure::parse(args).is_err());
        }
        let mut invalid = [1, 2, 3, 4];
        let before = invalid;
        assert!(
            Exposure::parse("")
                .unwrap()
                .apply_rgb(&mut invalid, 8, 3)
                .is_err()
        );
        assert_eq!(invalid, before);
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
                exposure: Some("exposure=1".into()),
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
                exposure: Some("exposure=1".into()),
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
mod float_reference_acceptance {
    #[test]
    fn synthetic_headroom_rgba_matches_saved_float_reference_exactly() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let input: Vec<f32> = std::fs::read(root.join("vibrance-grid-8.rgba"))
            .unwrap()
            .iter()
            .map(|v| *v as f32 / 255.0 * 2.0 - 0.25)
            .collect();
        for (case, args) in [
            "",
            "exposure=1",
            "exposure=-1",
            "exposure=0.7:black=0.1",
            "exposure=-0.5:black=-0.2",
            "exposure=3:black=0.125",
            "exposure=0:black=1",
            "exposure=3:black=-1",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            super::Exposure::parse(args)
                .unwrap()
                .apply_rgb_f32(&mut actual, 4)
                .unwrap();
            let expected: Vec<f32> =
                std::fs::read(root.join(format!("exposure-reference-float-{case}.raw")))
                    .unwrap()
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
            assert_eq!(actual.len(), 768);
            assert_eq!(expected.len(), actual.len());
            assert_eq!(actual, expected, "case={case}");
        }
    }
}

#[cfg(test)]
mod integer_conversion_gap {
    fn samples(depth: u8, case: usize, args: &str) -> (Vec<u16>, Vec<u16>) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let mut input = std::fs::read(root.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
        let original = input.clone();
        super::Exposure::parse(args)
            .unwrap()
            .apply_rgb(&mut input, depth, 4)
            .unwrap();
        let bytes = if depth == 8 { 1 } else { 2 };
        assert!(
            input
                .chunks_exact(bytes * 4)
                .zip(original.chunks_exact(bytes * 4))
                .all(|(a, b)| a[bytes * 3..] == b[bytes * 3..])
        );
        let samples = |data: &[u8]| -> Vec<u16> {
            data.chunks_exact(bytes)
                .map(|v| {
                    if bytes == 1 {
                        v[0] as u16
                    } else {
                        u16::from_le_bytes([v[0], v[1]])
                    }
                })
                .collect()
        };
        let actual = samples(
            &input
                .chunks_exact(bytes * 4)
                .flat_map(|p| p[..bytes * 3].iter().copied())
                .collect::<Vec<_>>(),
        );
        let expected = samples(
            &std::fs::read(root.join(format!("exposure-reference-rgb-{depth}-{case}.raw")))
                .unwrap(),
        );
        assert_eq!(actual.len(), 576);
        assert_eq!(expected.len(), actual.len());
        (actual, expected)
    }
    #[test]
    fn synthetic_half_exposure_reproduces_eight_bit_conversion_difference() {
        let (actual, expected) = samples(8, 2, "exposure=-1");
        assert_ne!(actual, expected);
        assert_eq!(
            actual
                .iter()
                .zip(expected)
                .map(|(a, b)| a.abs_diff(b))
                .max(),
            Some(1)
        );
    }
    #[test]
    fn synthetic_identity_reproduces_sixteen_bit_conversion_difference() {
        let (actual, expected) = samples(16, 0, "");
        assert_ne!(actual, expected);
        assert_eq!(
            actual
                .iter()
                .zip(expected)
                .map(|(a, b)| a.abs_diff(b))
                .max(),
            Some(65)
        );
    }
    #[test]
    #[ignore = "acceptance pending complete integer-float RGB conversion compatibility"]
    fn complete_integer_float_rgb_pipeline_matches_reference() {
        for (depth, case, args) in [(8, 2, "exposure=-1"), (16, 0, "")] {
            let (actual, expected) = samples(depth, case, args);
            assert_eq!(actual, expected);
        }
    }
}

#[cfg(test)]
mod headroom_acceptance {
    #[test]
    fn exposure_preserves_highlights_until_final_yuv_quantization() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let input = std::fs::read(root.join(format!("exposure-headroom-{depth}.y4m"))).unwrap();
            let request = fvid_media_info::DecodeTransform {
                exposure: Some("1".into()),
                ..Default::default()
            };
            let scale = 1u16 << (depth - 8);
            let maximum = ((1u32 << depth) - 1) as u16;
            let mut frames = 0;
            crate::owned_y4m_decode::visit_reader_transformed(
                std::io::Cursor::new(input),
                &request,
                |_, data, _, _| {
                    let values: Vec<u16> = if depth == 8 {
                        data.iter().map(|v| *v as u16).collect()
                    } else {
                        data.chunks_exact(2)
                            .map(|v| u16::from_le_bytes([v[0], v[1]]))
                            .collect()
                    };
                    assert_eq!(
                        &values[..9],
                        &[[maximum, 240 * scale, 16 * scale][frames]; 9]
                    );
                    assert_eq!(&values[9..], &[128 * scale; 8]);
                    frames += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(frames, 3);
        }
    }
}
