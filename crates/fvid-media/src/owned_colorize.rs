//! Constant HSL tint of planar YUV samples, retaining a chosen fraction of luma.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Colorize {
    rgb: [f32; 3],
    mix: f32,
}
impl Colorize {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["hue", "saturation", "lightness", "mix"];
        let mut values = [0f32, 0.5, 0.5, 1.0];
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = match option.split_once('=') {
                Some(pair) => pair,
                None => {
                    let key = *names.get(positional).ok_or("too many colorize options")?;
                    positional += 1;
                    (key, option)
                }
            };
            let index = names
                .iter()
                .position(|name| *name == key.trim())
                .ok_or("unknown colorize option")?;
            let value = crate::owned_expression::constant(value.trim())? as f32;
            let maximum = if index == 0 { 360.0 } else { 1.0 };
            if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                return Err(
                    "colorize hue must be 0..360; saturation, lightness and mix must be 0..1"
                        .into(),
                );
            }
            values[index] = value;
        }
        let [h, saturation, lightness, mix] = values;
        let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let sector = (h / 60.0) % 6.0;
        let secondary = chroma * (1.0 - ((sector % 2.0) - 1.0).abs());
        let channels = match sector as u32 {
            0 => [chroma, secondary, 0.0],
            1 => [secondary, chroma, 0.0],
            2 => [0.0, chroma, secondary],
            3 => [0.0, secondary, chroma],
            4 => [secondary, 0.0, chroma],
            _ => [chroma, 0.0, secondary],
        };
        let offset = lightness - chroma / 2.0;
        Ok(Self {
            rgb: channels.map(|c| c + offset),
            mix,
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("colorize requires planar YUV")?;
        if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0
        {
            return Err("invalid colorize geometry or depth".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("colorize geometry overflow")?;
        let c = frame
            .width
            .div_ceil(sx)
            .checked_mul(frame.height.div_ceil(sy))
            .ok_or("colorize geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let expected = c
            .checked_mul(2)
            .and_then(|c| c.checked_add(y))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("colorize geometry overflow")?;
        if expected != frame.data.len() {
            return Err("colorize frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|sample| u16::from_le_bytes([sample[0], sample[1]]) as u32 > maximum)
        {
            return Err("colorize sample exceeds precision".into());
        }
        // Constant tint follows normalized BT.709 limited-excursion coefficients.
        let [r, g, b] = self.rgb.map(f64::from);
        let m = f64::from(maximum);
        let tint = [
            ((0.21260 * r + 0.71520 * g + 0.07220 * b) * 219.0 / 255.0 * m) as u32,
            ((-0.11457 * r - 0.38543 * g + 0.5 * b) * 224.0 / 255.0 + 0.5).mul_add(m, 0.0) as u32,
            ((0.5 * r - 0.45415 * g - 0.04585 * b) * 224.0 / 255.0 + 0.5).mul_add(m, 0.0) as u32,
        ];
        for index in 0..y + 2 * c {
            let original = if bytes == 1 {
                frame.data[index] as u32
            } else {
                u16::from_le_bytes([frame.data[2 * index], frame.data[2 * index + 1]]) as u32
            };
            let value = if index < y {
                (tint[0] as f32 + (original as f32 - tint[0] as f32) * self.mix) as u32
            } else if index < y + c {
                tint[1]
            } else {
                tint[2]
            };
            if bytes == 1 {
                frame.data[index] = value as u8;
            } else {
                frame.data[2 * index..2 * index + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn red_tint_preserves_or_replaces_luma_at_every_integer_depth() {
        for depth in 8..=16 {
            let maximum = (1u32 << depth) - 1;
            let samples = [
                0u16,
                1,
                (maximum / 3) as u16,
                maximum as u16,
                0,
                maximum as u16,
            ];
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: if depth == 8 {
                    samples.map(|s| s as u8).to_vec()
                } else {
                    samples.into_iter().flat_map(u16::to_le_bytes).collect()
                },
            };
            Colorize::parse("0:1:0.5:1")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let result: Vec<u16> = if depth == 8 {
                frame.data.iter().map(|s| *s as u16).collect()
            } else {
                frame
                    .data
                    .chunks_exact(2)
                    .map(|s| u16::from_le_bytes([s[0], s[1]]))
                    .collect()
            };
            assert_eq!(&result[..4], &samples[..4]);
            assert_eq!(
                result[4],
                ((0.5 - 0.11457 * 224.0 / 255.0) * maximum as f64) as u16
            );
            assert_eq!(
                result[5],
                ((0.5 + 0.5 * 224.0 / 255.0) * maximum as f64) as u16
            );
            Colorize::parse("hue=360:saturation=1:lightness=0.5:mix=0")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let first = if depth == 8 {
                frame.data[0] as u16
            } else {
                u16::from_le_bytes([frame.data[0], frame.data[1]])
            };
            assert_eq!(first, (0.21260 * 219.0 / 255.0 * maximum as f64) as u16);
        }
    }
    #[test]
    fn green_half_mix_matches_committed_reference_values() {
        for (depth, expected) in [
            (8, [66u16, 67, 109, 194, 84, 76]),
            (12, [1068, 1068, 1750, 3115, 1354, 1230]),
            (16, [17098, 17099, 28021, 49866, 21673, 19695]),
        ] {
            let maximum = (1u32 << depth) - 1;
            let source = [0u16, 1, (maximum / 3) as u16, maximum as u16, 0, 0];
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: if depth == 8 {
                    source.map(|n| n as u8).to_vec()
                } else {
                    source.into_iter().flat_map(u16::to_le_bytes).collect()
                },
            };
            Colorize::parse("hue=120:saturation=0.5:lightness=0.5:mix=0.5")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let actual: Vec<u16> = if depth == 8 {
                frame.data.iter().map(|n| *n as u16).collect()
            } else {
                frame
                    .data
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect()
            };
            assert_eq!(actual, expected, "depth={depth}");
        }
    }
    #[test]
    fn malformed_parameters_and_samples_refuse_before_mutation() {
        for args in [
            "hue=-1",
            "hue=361",
            "saturation=2",
            "mix=NaN",
            "lightness=inf",
            "unknown=0",
            "1:1:1:1:1",
        ] {
            assert!(Colorize::parse(args).is_err(), "{args}");
        }
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0xff; 6],
        };
        let old = frame.data.clone();
        assert!(Colorize::parse("")
            .unwrap()
            .apply(&mut frame, 10)
            .unwrap_err()
            .contains("precision"));
        assert_eq!(frame.data, old);
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn colorize_y4m_and_ffv1_export_acceptance_preserves_precision_and_timestamps() {
        let directory = std::env::temp_dir().join(format!("fvid-colorize-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let raw = directory.join(format!("raw-{depth}.mkv"));
            crate::owned_lossless::transcode_lossless(
                &source,
                &raw,
                Default::default(),
                &Default::default(),
            )
            .unwrap();
            for (kind, input) in [("y4m", &source), ("ffv1", &raw)] {
                let output = directory.join(format!("{kind}-{depth}.mkv"));
                let transform = fvid_media_info::LosslessTransform {
                    colorize: Some("hue=0:saturation=1:lightness=0.5:mix=1".into()),
                    ..Default::default()
                };
                assert!(crate::owned_lossless::supports(
                    input,
                    &transform,
                    &Default::default()
                ));
                assert_eq!(
                    crate::owned_video_decode::decode_video_transformed(
                        input,
                        fvid_media_info::DecodeTransform {
                            colorize: transform.colorize.clone(),
                            ..Default::default()
                        }
                    )
                    .unwrap()
                    .video_frames,
                    3
                );
                let plan = crate::owned_lossless::plan_transcode_lossless(
                    input,
                    &transform,
                    &Default::default(),
                    None,
                )
                .unwrap();
                assert!(plan.steps.iter().any(|step| step.action == "filter"));
                assert_eq!(
                    crate::owned_lossless::transcode_lossless(
                        input,
                        &output,
                        transform,
                        &Default::default()
                    )
                    .unwrap()
                    .video_frames,
                    3
                );
                let mut index = 0u64;
                crate::owned_video_decode::decode_ffv1(
                    &output,
                    &Default::default(),
                    Some(&mut |frame| {
                        assert_eq!((frame.width, frame.height, frame.depth), (3, 3, depth));
                        let samples: Vec<u16> = if depth == 8 {
                            frame.pixels.iter().map(|s| *s as u16).collect()
                        } else {
                            frame
                                .pixels
                                .chunks_exact(2)
                                .map(|s| u16::from_le_bytes([s[0], s[1]]))
                                .collect()
                        };
                        let maximum = (1u64 << depth) - 1;
                        let expected: Vec<u16> = (0..9)
                            .map(|n| ((maximum * n / 8 + index) % (maximum + 1)) as u16)
                            .collect();
                        assert_eq!(&samples[..9], expected);
                        let u = ((0.5 - 0.11457 * 224.0 / 255.0) * maximum as f64) as u16;
                        let v = ((0.5 + 0.5 * 224.0 / 255.0) * maximum as f64) as u16;
                        assert_eq!(&samples[9..13], &[u; 4]);
                        assert_eq!(&samples[13..], &[v; 4]);
                        assert_eq!(frame.pts_ns, index as i64 * 500_000_000);
                        assert_eq!(frame.duration_ns, Some(500_000_000));
                        index += 1;
                        Ok(())
                    }),
                    None,
                )
                .unwrap()
                .unwrap();
                assert_eq!(index, 3);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
