//! Chroma-selective grayscale conversion on packed planar YUV samples.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Monochrome {
    spot: [f32; 2],
    size: f32,
    high: f32,
}
impl Monochrome {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["cb", "cr", "size", "high"];
        let mut values = [0f32, 0.0, 1.0, 0.0];
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = match option.split_once('=') {
                Some(pair) => pair,
                None => {
                    let key = *names.get(positional).ok_or("too many monochrome options")?;
                    positional += 1;
                    (key, option)
                }
            };
            let index = names
                .iter()
                .position(|name| *name == key.trim())
                .ok_or("unknown monochrome option")?;
            let value = crate::owned_expression::constant(value.trim())? as f32;
            let (minimum, maximum) = match index {
                0 | 1 => (-1.0, 1.0),
                2 => (0.1, 10.0),
                _ => (0.0, 1.0),
            };
            if !value.is_finite() || !(minimum..=maximum).contains(&value) {
                return Err("monochrome cb/cr must be -1..1, size 0.1..10, high 0..1".into());
            }
            values[index] = value;
        }
        Ok(Self {
            spot: [values[0] * 0.5, values[1] * 0.5],
            size: values[2].recip(),
            high: values[3],
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("monochrome requires planar YUV")?;
        if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0
        {
            return Err("invalid monochrome geometry or depth".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("monochrome geometry overflow")?;
        let cw = frame.width.div_ceil(sx);
        let c = cw
            .checked_mul(frame.height.div_ceil(sy))
            .ok_or("monochrome geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let expected = c
            .checked_mul(2)
            .and_then(|n| n.checked_add(y))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("monochrome geometry overflow")?;
        if expected != frame.data.len() {
            return Err("monochrome frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        let read = |data: &[u8], index: usize| -> u32 {
            if bytes == 1 {
                data[index] as u32
            } else {
                u16::from_le_bytes([data[2 * index], data[2 * index + 1]]) as u32
            }
        };
        let write = |data: &mut [u8], index: usize, value: u32| {
            if bytes == 1 {
                data[index] = value as u8;
            } else {
                data[2 * index..2 * index + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
        };
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|b| u16::from_le_bytes([b[0], b[1]]) as u32 > maximum)
        {
            return Err("monochrome sample exceeds precision".into());
        }
        let inverse = (maximum as f32).recip();
        // Chroma gain is shared by each subsampled cell; calculate exp once.
        for cy in 0..frame.height.div_ceil(sy) {
            for cx in 0..cw {
                let chroma = cy * cw + cx;
                let blue = read(&frame.data, y + chroma) as f32 * inverse - 0.5;
                let red = read(&frame.data, y + c + chroma) as f32 * inverse - 0.5;
                let db = self.spot[0] - blue;
                let dr = self.spot[1] - red;
                let response = (-((db * db + dr * dr) * self.size).clamp(0.0, 1.0)).exp();
                let row = cy * sy;
                let column = cx * sx;
                for row in row..row.saturating_add(sy).min(frame.height) {
                    for column in column..column.saturating_add(sx).min(frame.width) {
                        let index = row * frame.width + column;
                        let luma = read(&frame.data, index) as f32 * inverse;
                        let envelope = if luma < 0.6 {
                            let t = (luma / 0.6 - 1.0).abs();
                            1.0 - t * t
                        } else {
                            let t = (1.0 - luma) / (1.0 - 0.6);
                            t * t * (3.0 - 2.0 * t)
                        };
                        let weight = envelope + (1.0 - envelope) * (1.0 - self.high);
                        let result = (1.0 - weight) * luma + weight * response * luma;
                        write(
                            &mut frame.data,
                            index,
                            (result * maximum as f32)
                                .round_ties_even()
                                .clamp(0.0, maximum as f32) as u32,
                        );
                    }
                }
            }
        }
        for index in y..y + 2 * c {
            write(&mut frame.data, index, 1u32 << (depth - 1));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_input_keeps_luma_and_clears_chroma_at_every_integer_depth() {
        for depth in 8..=16 {
            let maximum = (1u32 << depth) - 1;
            let samples = [
                0u16,
                1,
                (maximum / 3) as u16,
                maximum as u16,
                (1u32 << (depth - 1)) as u16,
                (1u32 << (depth - 1)) as u16,
            ];
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: if depth == 8 {
                    samples.map(|n| n as u8).to_vec()
                } else {
                    samples.into_iter().flat_map(u16::to_le_bytes).collect()
                },
            };
            let original = frame.data.clone();
            Monochrome::parse("")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            assert_eq!(frame.data, original, "depth={depth}");
        }
    }
    #[test]
    fn highlight_strength_retains_white_and_chroma_filter_attenuates_midtones() {
        let source = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0, 100, 200, 255, 0, 255],
        };
        let mut full = GeometryFrame {
            width: source.width,
            height: source.height,
            subsampling: source.subsampling,
            data: source.data.clone(),
        };
        Monochrome::parse("size=0.1:high=0")
            .unwrap()
            .apply(&mut full, 8)
            .unwrap();
        assert_eq!(full.data, [0, 37, 74, 94, 128, 128]);
        let mut highlights = source;
        Monochrome::parse("size=0.1:high=1")
            .unwrap()
            .apply(&mut highlights, 8)
            .unwrap();
        assert_eq!((highlights.data[0], highlights.data[3]), (0, 255));
        assert!(highlights.data[1] < 100 && highlights.data[2] < 200);
    }
    #[test]
    fn invalid_options_geometry_and_precision_refuse_before_mutation() {
        for args in [
            "cb=-2",
            "cr=2",
            "size=0",
            "size=11",
            "high=2",
            "high=NaN",
            "size=inf",
            "unknown=1",
            "0:0:1:0:1",
        ] {
            assert!(Monochrome::parse(args).is_err(), "{args}");
        }
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0xff; 6],
        };
        let original = frame.data.clone();
        assert!(Monochrome::parse("")
            .unwrap()
            .apply(&mut frame, 10)
            .unwrap_err()
            .contains("precision"));
        assert_eq!(frame.data, original);
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn monochrome_y4m_and_ffv1_export_acceptance_preserves_precision_and_timestamps() {
        let directory =
            std::env::temp_dir().join(format!("fvid-monochrome-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../tests/fixtures/playback-errors/monochrome-grid-{depth}.y4m"
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
                    monochrome: Some("cb=0.5:cr=-0.5:size=0.2:high=0.75".into()),
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
                            monochrome: transform.monochrome.clone(),
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
                        assert!(samples[..9].iter().zip(&expected).all(|(a, b)| a <= b));
                        assert!(samples[..9].iter().zip(&expected).any(|(a, b)| a < b));
                        let center = 1u16 << (depth - 1);
                        assert_eq!(&samples[9..13], &[center; 4]);
                        assert_eq!(&samples[13..], &[center; 4]);
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
