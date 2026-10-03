//! Constant hue/saturation/brightness on tightly packed planar YUV.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Hue {
    cosine: i64,
    sine: i64,
    brightness: f32,
}
impl Hue {
    pub fn parse(args: &str) -> Result<Self> {
        let (mut degrees, mut radians, mut saturation, mut brightness) = (None, None, 1f32, 0f32);
        let mut positional = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = match entry.split_once('=') {
                Some(pair) => pair,
                None => {
                    let key = *["h", "s", "H", "b"]
                        .get(positional)
                        .ok_or("too many hue options")?;
                    positional += 1;
                    (key, entry)
                }
            };
            let value = value
                .trim()
                .parse::<f32>()
                .map_err(|_| "owned hue requires constant numeric parameters")?;
            if !value.is_finite() {
                return Err("hue parameters must be finite".into());
            }
            match key.trim() {
                "h" => degrees = Some(value),
                "H" => radians = Some(value),
                "s" => saturation = value.clamp(-10., 10.),
                "b" => brightness = value.clamp(-10., 10.),
                _ => return Err("unknown hue option".into()),
            }
        }
        if degrees.is_some() && radians.is_some() {
            return Err("h and H cannot both be specified".into());
        }
        let angle =
            radians.unwrap_or_else(|| degrees.unwrap_or(0.) * (std::f64::consts::PI / 180.) as f32);
        let angle = f64::from(angle);
        Ok(Self {
            cosine: (angle.cos() * 65536. * f64::from(saturation)).round_ties_even() as i64,
            sine: (angle.sin() * 65536. * f64::from(saturation)).round_ties_even() as i64,
            brightness,
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("hue requires planar YUV")?;
        if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0
        {
            return Err("owned hue requires 8..=16-bit planar YUV".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("hue geometry overflow")?;
        let chroma = frame
            .width
            .div_ceil(sx)
            .checked_mul(frame.height.div_ceil(sy))
            .ok_or("hue geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let expected = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(y))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("hue geometry overflow")?;
        if frame.data.len() != expected {
            return Err("hue frame length mismatch".into());
        }
        let read = |data: &[u8], index: usize| -> i64 {
            if bytes == 1 {
                i64::from(data[index])
            } else {
                i64::from(u16::from_le_bytes([data[2 * index], data[2 * index + 1]]))
            }
        };
        let write = |data: &mut [u8], index: usize, value: i64| {
            if bytes == 1 {
                data[index] = value as u8;
            } else {
                data[2 * index..2 * index + 2].copy_from_slice(&(value as u16).to_le_bytes());
            }
        };
        let max = (1i64 << depth) - 1;
        if depth > 8
            && frame
                .data
                .chunks_exact(2)
                .any(|b| i64::from(u16::from_le_bytes([b[0], b[1]])) > max)
        {
            return Err("hue sample exceeds precision".into());
        }
        let center = 1i64 << (depth - 1);
        for i in 0..chroma {
            let u = read(&frame.data, y + i) - center;
            let v = read(&frame.data, y + chroma + i) - center;
            let a = ((self.cosine * u - self.sine * v + 32768) >> 16) + center;
            let b = ((self.sine * u + self.cosine * v + 32768) >> 16) + center;
            write(&mut frame.data, y + i, a.clamp(0, max));
            write(&mut frame.data, y + chroma + i, b.clamp(0, max));
        }
        if self.brightness != 0. {
            let offset = self.brightness
                * if depth == 8 {
                    25.5
                } else {
                    (1u32 << depth) as f32 / 10.0
                };
            for i in 0..y {
                let value = (read(&frame.data, i) as f32 + offset) as i64;
                write(&mut frame.data, i, value.clamp(0, max));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotates_chroma_and_keeps_luma() {
        let mut frame = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![16, 64, 128, 235, 160, 96],
        };
        Hue::parse("h=90").unwrap().apply(&mut frame, 8).unwrap();
        assert_eq!(frame.data, [16, 64, 128, 235, 160, 160]);
        Hue::parse("s=0:b=1").unwrap().apply(&mut frame, 8).unwrap();
        assert_eq!(frame.data, [41, 89, 153, 255, 128, 128]);
    }
    #[test]
    fn all_integer_depths_rotate_chroma_and_reject_excess_precision() {
        for depth in 9..=16 {
            let center = 1u16 << (depth - 1);
            let samples = [16u16, 100, 200, 235, center + 32, center - 16];
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: samples.into_iter().flat_map(u16::to_le_bytes).collect(),
            };
            Hue::parse("h=90")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let result: Vec<u16> = frame
                .data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert_eq!(result, [16, 100, 200, 235, center + 16, center + 32]);
            Hue::parse("s=0:b=1")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            let value = u16::from_le_bytes([frame.data[0], frame.data[1]]);
            assert_eq!(value, (16.0 + (1u32 << depth) as f32 / 10.0) as u16);
            assert_eq!(u16::from_le_bytes([frame.data[8], frame.data[9]]), center);
            if depth < 16 {
                frame.data[..2].copy_from_slice(&(1u16 << depth).to_le_bytes());
                let original = frame.data.clone();
                assert!(
                    Hue::parse("h=90")
                        .unwrap()
                        .apply(&mut frame, depth)
                        .is_err()
                );
                assert_eq!(frame.data, original);
            }
        }
    }
    #[test]
    fn rejects_expressions_conflicts_and_invalid_storage_without_mutation() {
        for args in ["h=t", "h=1:H=2", "s=NaN", "unknown=1"] {
            assert!(Hue::parse(args).is_err());
        }
        let mut frame = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 5],
        };
        assert!(Hue::parse("").unwrap().apply(&mut frame, 8).is_err());
        assert_eq!(frame.data, vec![0; 5]);
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn high_depth_hue_exports_y4m_and_ffv1_without_fallback() {
        let directory =
            std::env::temp_dir().join(format!("fvid-hue-high-depth-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../tests/fixtures/playback-errors/hue-chroma-{depth}.y4m"
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
                    hue: Some("h=90".into()),
                    ..Default::default()
                };
                assert!(crate::owned_lossless::supports(
                    input,
                    &transform,
                    &Default::default()
                ));
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
                let mut frames = 0;
                crate::owned_video_decode::decode_ffv1(
                    &output,
                    &Default::default(),
                    Some(&mut |frame| {
                        assert_eq!((frame.width, frame.height, frame.depth), (3, 3, depth));
                        let samples: Vec<u16> = frame
                            .pixels
                            .chunks_exact(2)
                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                            .collect();
                        assert_eq!(&samples[..9], &[16, 100, 200, 300, 400, 500, 600, 700, 800]);
                        let center = 1u16 << (depth - 1);
                        assert!(samples[9..18].iter().all(|&n| n == center + 200));
                        assert!(samples[18..].iter().all(|&n| n == center + 100));
                        assert_eq!(frame.pts_ns, frames * 500_000_000);
                        assert_eq!(frame.duration_ns, Some(500_000_000));
                        frames += 1;
                        Ok(())
                    }),
                    None,
                )
                .unwrap()
                .unwrap();
                assert_eq!(frames, 3);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
