//! Owned contrast-adaptive sharpening with replicated image boundaries.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Cas {
    denominator: f32,
    planes: u8,
}
#[derive(Clone, Copy)]
struct Plane {
    offset: usize,
    width: usize,
    height: usize,
    stride: usize,
    mask: u8,
}
impl Cas {
    pub fn parse(args: &str) -> Result<Self> {
        let mut strength = 0.0f32;
        let mut planes = 7;
        let mut position = 0;
        for part in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = part.split_once('=') {
                pair
            } else {
                let name = *["strength", "planes"]
                    .get(position)
                    .ok_or("too many cas options")?;
                position += 1;
                (name, part)
            };
            let value = crate::owned_expression::constant(value.trim())?;
            match name.trim() {
                "strength" if value.is_finite() && (0.0..=1.0).contains(&value) => {
                    strength = value as f32
                }
                "planes"
                    if value.is_finite()
                        && (0.0..=15.0).contains(&value)
                        && value.fract() == 0.0 =>
                {
                    planes = value as u8
                }
                _ => return Err("invalid cas option".into()),
            }
        }
        Ok(Self {
            denominator: -(4.01f32 - 16.0).mul_add(strength, 16.0),
            planes,
        })
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if let Some([sx, sy]) = frame.subsampling {
            if sx == 0 || sy == 0 {
                return Err("invalid cas subsampling".into());
            }
            let y = frame
                .width
                .checked_mul(frame.height)
                .ok_or("cas geometry overflow")?;
            let cw = frame.width.div_ceil(sx);
            let ch = frame.height.div_ceil(sy);
            let c = cw.checked_mul(ch).ok_or("cas geometry overflow")?;
            let count = c
                .checked_mul(2)
                .and_then(|n| n.checked_add(y))
                .ok_or("cas geometry overflow")?;
            self.filter(
                &mut frame.data,
                depth,
                count,
                &[
                    Plane {
                        offset: 0,
                        width: frame.width,
                        height: frame.height,
                        stride: 1,
                        mask: 1,
                    },
                    Plane {
                        offset: y,
                        width: cw,
                        height: ch,
                        stride: 1,
                        mask: 2,
                    },
                    Plane {
                        offset: y + c,
                        width: cw,
                        height: ch,
                        stride: 1,
                        mask: 4,
                    },
                ],
            )
        } else {
            self.apply_rgb(&mut frame.data, frame.width, frame.height, depth, 3)
        }
    }
    /// Packed RGB/RGBA, LE above eight bits. Plane bits follow G/B/R/A ordering.
    pub fn apply_rgb(
        &self,
        data: &mut Vec<u8>,
        width: usize,
        height: usize,
        depth: u8,
        channels: usize,
    ) -> Result<()> {
        if !matches!(channels, 3 | 4) {
            return Err("invalid cas RGB channels".into());
        }
        let count = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(channels))
            .ok_or("cas geometry overflow")?;
        let planes: [Plane; 4] = std::array::from_fn(|i| Plane {
            offset: i,
            width,
            height,
            stride: channels,
            mask: [4, 1, 2, 8][i],
        });
        self.filter(data, depth, count, &planes[..channels])
    }
    fn filter(&self, data: &mut Vec<u8>, depth: u8, count: usize, planes: &[Plane]) -> Result<()> {
        if !(8..=16).contains(&depth) || planes.iter().any(|p| p.width == 0 || p.height == 0) {
            return Err("invalid cas geometry or depth".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if count.checked_mul(bytes) != Some(data.len()) {
            return Err("cas frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 > maximum)
        {
            return Err("cas sample exceeds precision".into());
        }
        if self.planes == 0 {
            return Ok(());
        }
        let mut output = crate::owned_frame::buffer(data.len())?;
        output.copy_from_slice(data);
        for plane in planes {
            if self.planes & plane.mask == 0 {
                continue;
            }
            let sample = |x: usize, y: usize| -> u32 {
                let i = plane.offset + (y * plane.width + x) * plane.stride;
                if bytes == 1 {
                    data[i] as u32
                } else {
                    u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) as u32
                }
            };
            for y in 0..plane.height {
                for x in 0..plane.width {
                    let xs = [x.saturating_sub(1), x, (x + 1).min(plane.width - 1)];
                    let ys = [y.saturating_sub(1), y, (y + 1).min(plane.height - 1)];
                    let values: [u32; 9] = std::array::from_fn(|i| sample(xs[i % 3], ys[i / 3]));
                    let cross = [values[1], values[3], values[4], values[5], values[7]];
                    let cross_min = *cross.iter().min().unwrap();
                    let cross_max = *cross.iter().max().unwrap();
                    let low = cross_min + *values.iter().min().unwrap();
                    let high = cross_max + *values.iter().max().unwrap();
                    let amplitude = if high == 0 {
                        0.0
                    } else {
                        ((low.min(2 * maximum + 1 - high) as f32 / high as f32).clamp(0.0, 1.0))
                            .sqrt()
                    };
                    let weight = amplitude / self.denominator;
                    let neighbours = (values[1] + values[3] + values[5] + values[7]) as f32;
                    let value = (neighbours.mul_add(weight, values[4] as f32)
                        / weight.mul_add(4.0, 1.0))
                    .clamp(0.0, maximum as f32) as u16;
                    let i = plane.offset + (y * plane.width + x) * plane.stride;
                    if bytes == 1 {
                        output[i] = value as u8;
                    } else {
                        output[2 * i..2 * i + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        *data = output;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn black_edges_masks_and_invalid_precision_are_atomic() {
        for depth in 8..=16 {
            let bytes = if depth == 8 { 1 } else { 2 };
            for [width, height] in [[1usize, 1usize], [1, 3], [3, 1], [3, 5]] {
                let c = width.div_ceil(2) * height.div_ceil(2);
                let mut frame = GeometryFrame {
                    width,
                    height,
                    subsampling: Some([2, 2]),
                    data: vec![0; (width * height + 2 * c) * bytes],
                };
                Cas::parse("strength=1")
                    .unwrap()
                    .apply(&mut frame, depth)
                    .unwrap();
                assert!(frame.data.iter().all(|v| *v == 0));
            }
        }
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0, 0, 1, 0, 255, 255],
        };
        let before = frame.data.clone();
        assert!(
            Cas::parse("planes=0")
                .unwrap()
                .apply(&mut frame, 12)
                .is_err()
        );
        assert_eq!(frame.data, before);
        for args in [
            "strength=2",
            "planes=16",
            "planes=1.2",
            "strength=nan",
            "unknown=0",
        ] {
            assert!(Cas::parse(args).is_err());
        }
    }
    #[test]
    fn rgb_plane_mask_matches_gbr_order_and_preserves_alpha() {
        let mut data = vec![20, 90, 70, 11, 130, 50, 30, 22, 40, 190, 220, 33];
        let input = data.clone();
        Cas::parse("strength=1:planes=1")
            .unwrap()
            .apply_rgb(&mut data, 3, 1, 8, 4)
            .unwrap();
        for (out, source) in data.chunks_exact(4).zip(input.chunks_exact(4)) {
            assert_eq!(out[0], source[0]);
            assert_eq!(out[2..], source[2..]);
        }
        assert_ne!(data, input);
        Cas::parse("planes=0")
            .unwrap()
            .apply_rgb(&mut data, 3, 1, 8, 4)
            .unwrap();
    }
}

#[cfg(test)]
mod reference_acceptance {
    const CASES: [&str; 6] = [
        "",
        "strength=1",
        "planes=0",
        "strength=0.7:planes=1",
        "strength=0.3:planes=6",
        "strength=1:planes=15",
    ];
    #[test]
    fn synthetic_yuv_video_matches_saved_reference() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let input = std::fs::read(root.join(format!("cas-grid-{depth}.y4m"))).unwrap();
            for (case, args) in CASES.into_iter().enumerate() {
                let request = fvid_media_info::DecodeTransform {
                    cas: Some(args.into()),
                    ..Default::default()
                };
                assert!(crate::owned_y4m_decode::supported_request(&request));
                let mut actual = Vec::new();
                let mut frames = 0;
                crate::owned_y4m_decode::visit_reader_transformed(
                    std::io::Cursor::new(&input),
                    &request,
                    |_, frame, _, _| {
                        frames += 1;
                        actual.extend_from_slice(frame);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(frames, 3);
                let reference =
                    std::fs::read(root.join(format!("cas-reference-{depth}-{case}.raw"))).unwrap();
                assert_eq!(actual, reference, "depth={depth} args={args}");
            }
        }
    }
    #[test]
    fn synthetic_rgba_video_matches_saved_reference_including_alpha_mask() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 16] {
            let input = std::fs::read(root.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
            let frame_bytes = 8 * 8 * 4 * if depth == 8 { 1 } else { 2 };
            for (case, args) in CASES.into_iter().enumerate() {
                let mut actual = Vec::new();
                for frame in input.chunks_exact(frame_bytes) {
                    let mut output = frame.to_vec();
                    super::Cas::parse(args)
                        .unwrap()
                        .apply_rgb(&mut output, 8, 8, depth, 4)
                        .unwrap();
                    actual.extend_from_slice(&output);
                }
                let reference =
                    std::fs::read(root.join(format!("cas-rgb-reference-{depth}-{case}.raw")))
                        .unwrap();
                assert_eq!(actual, reference, "depth={depth} args={args}");
            }
        }
    }
}
