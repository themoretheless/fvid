//! Separable Gaussian convolution of integer YUV planes with replicated edges.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct GaussianBlur {
    sigma: [f64; 2],
    steps: usize,
    planes: u8,
}
impl GaussianBlur {
    pub fn parse(args: &str) -> Result<Self> {
        let mut values = [0.5, -1.0, 1.0, 15.0];
        let names = ["sigma", "sigmaV", "steps", "planes"];
        let mut positional = 0;
        if !args.is_empty() {
            for entry in args.split(':') {
                let (key, value) = if let Some(pair) = entry.split_once('=') {
                    pair
                } else {
                    let key = *names.get(positional).ok_or("too many gblur options")?;
                    positional += 1;
                    (key, entry)
                };
                let index = names
                    .iter()
                    .position(|n| *n == key)
                    .ok_or("unknown gblur option")?;
                let n: f64 = value.parse().map_err(|_| "invalid gblur number")?;
                let valid = n.is_finite()
                    && match index {
                        0 => (0.0..=1024.0).contains(&n),
                        1 => (-1.0..=1024.0).contains(&n),
                        2 => (1.0..=6.0).contains(&n) && n.fract() == 0.0,
                        _ => (0.0..=15.0).contains(&n) && n.fract() == 0.0,
                    };
                if !valid {
                    return Err("gblur option out of range".into());
                }
                values[index] = n;
            }
        }
        Ok(Self {
            sigma: [
                values[0],
                if values[1] < 0.0 {
                    values[0]
                } else {
                    values[1]
                },
            ],
            steps: values[2] as usize,
            planes: values[3] as u8,
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("gblur requires planar YUV")?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err("invalid gblur geometry or depth".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let sizes = [
            (frame.width, frame.height),
            (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
            (frame.width.div_ceil(sx), frame.height.div_ceil(sy)),
        ];
        let mut samples = 0usize;
        for (w, h) in sizes {
            samples = samples
                .checked_add(w.checked_mul(h).ok_or("gblur geometry overflow")?)
                .ok_or("gblur storage overflow")?;
        }
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("gblur storage mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > maximum)
        {
            return Err("gblur sample exceeds depth".into());
        }
        if self.planes & 7 == 0 || self.sigma == [0.0, 0.0] {
            return Ok(());
        }
        let mut base = 0;
        for (plane, (w, h)) in sizes.into_iter().enumerate() {
            let area = w * h;
            if self.planes & (1 << plane) != 0 {
                let mut data = Vec::new();
                data.try_reserve_exact(area).map_err(|e| e.to_string())?;
                for i in 0..area {
                    let offset = base + i * bytes;
                    data.push(if bytes == 1 {
                        f64::from(frame.data[offset])
                    } else {
                        f64::from(u16::from_le_bytes([
                            frame.data[offset],
                            frame.data[offset + 1],
                        ]))
                    });
                }
                let mut scratch = Vec::new();
                scratch.try_reserve_exact(area).map_err(|e| e.to_string())?;
                scratch.resize(area, 0.0);
                for _ in 0..self.steps {
                    for axis in 0..2 {
                        let sigma = self.sigma[axis] / (self.steps as f64).sqrt();
                        if sigma == 0.0 {
                            continue;
                        }
                        let radius = (sigma * 4.0).ceil() as usize;
                        let mut weights = Vec::new();
                        weights
                            .try_reserve_exact(2 * radius + 1)
                            .map_err(|e| e.to_string())?;
                        for i in 0..=2 * radius {
                            let d = i as f64 - radius as f64;
                            weights.push((-d * d / (2.0 * sigma * sigma)).exp());
                        }
                        let sum: f64 = weights.iter().sum();
                        for weight in &mut weights {
                            *weight /= sum;
                        }
                        for y in 0..h {
                            for x in 0..w {
                                let mut value = 0.0;
                                for (i, weight) in weights.iter().enumerate() {
                                    let delta = i as i64 - radius as i64;
                                    let xx = if axis == 0 {
                                        (x as i64 + delta).clamp(0, w as i64 - 1) as usize
                                    } else {
                                        x
                                    };
                                    let yy = if axis == 1 {
                                        (y as i64 + delta).clamp(0, h as i64 - 1) as usize
                                    } else {
                                        y
                                    };
                                    value += data[yy * w + xx] * weight;
                                }
                                scratch[y * w + x] = value;
                            }
                        }
                        std::mem::swap(&mut data, &mut scratch);
                    }
                }
                for (i, value) in data.into_iter().enumerate() {
                    let n = value.round().clamp(0.0, f64::from(maximum)) as u16;
                    let offset = base + i * bytes;
                    if bytes == 1 {
                        frame.data[offset] = n as u8;
                    } else {
                        frame.data[offset..offset + 2].copy_from_slice(&n.to_le_bytes());
                    }
                }
            }
            base += area * bytes;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constant_odd_high_depth_planes_remain_constant() {
        let mut frame = GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([2, 2]),
            data: (0..17).flat_map(|_| 777u16.to_le_bytes()).collect(),
        };
        let before = frame.data.clone();
        GaussianBlur::parse("sigma=2:steps=3")
            .unwrap()
            .apply(&mut frame, 10)
            .unwrap();
        assert_eq!(frame.data, before);
    }
    #[test]
    fn impulse_spreads_symmetrically_and_unselected_chroma_is_untouched() {
        let mut frame = GeometryFrame {
            width: 9,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0; 27],
        };
        frame.data[4] = 255;
        frame.data[9..].fill(123);
        GaussianBlur::parse("sigma=1:sigmaV=0:planes=1")
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        assert_eq!(frame.data[4], 102);
        assert_eq!(frame.data[3], 62);
        assert_eq!(frame.data[2], 14);
        for x in 0..4 {
            assert_eq!(frame.data[x], frame.data[8 - x]);
        }
        assert!(frame.data[9..].iter().all(|v| *v == 123));
    }
    #[test]
    fn invalid_options_are_refused() {
        for args in [
            "sigma=NaN",
            "steps=0",
            "steps=1.5",
            "planes=16",
            "sigmaV=-2",
            "unknown=1",
        ] {
            assert!(GaussianBlur::parse(args).is_err(), "{args}");
        }
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn gaussian_export_accepts_y4m_and_ffv1_and_keeps_packet_clock() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/gblur-impulse.y4m");
        let temp = std::env::temp_dir().join(format!("fvid-gblur-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let raw = temp.join("input.mkv");
        crate::owned_lossless::transcode_lossless(
            &source,
            &raw,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        for (case, input) in [("y4m", &source), ("ffv1", &raw)] {
            let output = temp.join(format!("{case}.mkv"));
            let transform = fvid_media_info::LosslessTransform {
                gblur: Some("sigma=1:sigmaV=0:planes=1".into()),
                ..Default::default()
            };
            assert!(crate::owned_lossless::supports(
                input,
                &transform,
                &Default::default()
            ));
            crate::owned_lossless::transcode_lossless(
                input,
                &output,
                transform,
                &Default::default(),
            )
            .unwrap();
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), 3);
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(10, 4, 1 << 20).unwrap();
            for index in 0..3 {
                assert_eq!(reader.packets[index].pts_ns, index as i64 * 500_000_000);
                assert_eq!(reader.packets[index].duration_ns, Some(500_000_000));
                let frame = decoder
                    .decode(&reader.read_packet(index).unwrap())
                    .unwrap()
                    .frame;
                assert_eq!(frame.data[4], 102);
                assert_eq!(frame.data[3], 62);
                assert_eq!(frame.data[2], 14);
                assert!(frame.data[40..].iter().all(|v| *v == 123));
            }
        }
        std::fs::remove_dir_all(temp).unwrap();
    }
}
