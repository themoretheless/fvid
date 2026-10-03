//! Direct bilateral convolution: Gaussian spatial and sample-distance weights.
//! Integer planar YUV, clipped neighborhoods and independently rounded chroma.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Bilateral {
    spatial: f64,
    range: f64,
    planes: u8,
}
impl Bilateral {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["sigmaS", "sigmaR", "planes"];
        let mut values = [0.1, 0.1, 1.0];
        let mut positional = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = match entry.split_once('=') {
                Some(pair) => pair,
                None => {
                    let name = *names.get(positional).ok_or("too many bilateral options")?;
                    positional += 1;
                    (name, entry)
                }
            };
            let index = names
                .iter()
                .position(|&name| name == key)
                .ok_or("unknown bilateral option")?;
            let value: f64 = value.parse().map_err(|_| "invalid bilateral number")?;
            if !value.is_finite()
                || !match index {
                    0 => (0.0..=512.0).contains(&value),
                    1 => (0.0..=1.0).contains(&value),
                    _ => (0.0..=15.0).contains(&value) && value.fract() == 0.0,
                }
            {
                return Err("bilateral option out of range".into());
            }
            values[index] = value;
        }
        Ok(Self {
            spatial: values[0],
            range: values[1],
            planes: values[2] as u8,
        })
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("bilateral requires planar YUV")?;
        if sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth)
        {
            return Err("invalid bilateral geometry or depth".into());
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
                .checked_add(w.checked_mul(h).ok_or("bilateral plane overflow")?)
                .ok_or("bilateral storage overflow")?;
        }
        if samples.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("bilateral storage mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > maximum)
        {
            return Err("bilateral sample exceeds depth".into());
        }
        // Zero spatial width or zero range width has the identity limit.
        if self.spatial == 0.0 || self.range == 0.0 || self.planes & 7 == 0 {
            return Ok(());
        }
        let radius = (4.0 * self.spatial).ceil() as usize;
        let spatial_denominator = 2.0 * self.spatial * self.spatial;
        let range_sigma = self.range * f64::from(maximum);
        let range_denominator = 2.0 * range_sigma * range_sigma;
        if spatial_denominator == 0.0 || range_denominator == 0.0 {
            return Ok(());
        }
        let mut base = 0;
        for (plane, (w, h)) in sizes.into_iter().enumerate() {
            let area = w * h;
            if self.planes & (1 << plane) != 0 {
                let mut source = Vec::new();
                source.try_reserve_exact(area).map_err(|e| e.to_string())?;
                for i in 0..area {
                    let offset = base + i * bytes;
                    source.push(if bytes == 1 {
                        f64::from(frame.data[offset])
                    } else {
                        f64::from(u16::from_le_bytes([
                            frame.data[offset],
                            frame.data[offset + 1],
                        ]))
                    });
                }
                for y in 0..h {
                    for x in 0..w {
                        let center = source[y * w + x];
                        let (mut sum, mut weight_sum) = (0.0, 0.0);
                        for yy in y.saturating_sub(radius)..=y.saturating_add(radius).min(h - 1) {
                            for xx in x.saturating_sub(radius)..=x.saturating_add(radius).min(w - 1)
                            {
                                let dx = x.abs_diff(xx) as f64;
                                let dy = y.abs_diff(yy) as f64;
                                let sample = source[yy * w + xx];
                                let difference = sample - center;
                                let weight = (-(dx * dx + dy * dy) / spatial_denominator
                                    - difference * difference / range_denominator)
                                    .exp();
                                weight_sum += weight;
                                sum += sample * weight;
                            }
                        }
                        let value =
                            (sum / weight_sum).round().clamp(0.0, f64::from(maximum)) as u16;
                        let offset = base + (y * w + x) * bytes;
                        if bytes == 1 {
                            frame.data[offset] = value as u8;
                        } else {
                            frame.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                        }
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
    fn two_samples_have_analytic_weighted_mean_and_keep_unselected_planes() {
        let mut frame = GeometryFrame {
            width: 2,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0, 10, 128, 128, 128, 128],
        };
        Bilateral::parse("sigmaS=1:sigmaR=1:planes=1")
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        // Cross weight exp(-1/2 - (10/255)^2/2) gives rounded means 4 and 6.
        assert_eq!(frame.data, [4, 6, 128, 128, 128, 128]);
    }
    #[test]
    fn strong_edge_is_retained_and_zero_sigmas_are_identity() {
        let frame = GeometryFrame {
            width: 2,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0, 255, 128, 128, 128, 128],
        };
        for args in ["1:0.1:1", "0:1:7", "1:0:7", "1e-300:1:7", "1:1e-300:7"] {
            let mut filtered = GeometryFrame {
                width: frame.width,
                height: frame.height,
                subsampling: frame.subsampling,
                data: frame.data.clone(),
            };
            Bilateral::parse(args)
                .unwrap()
                .apply(&mut filtered, 8)
                .unwrap();
            assert_eq!(filtered.data, frame.data);
        }
    }
    #[test]
    fn odd_high_depth_constant_frame_and_validation() {
        let mut frame = GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([2, 2]),
            data: (0..17).flat_map(|_| 777u16.to_le_bytes()).collect(),
        };
        let original = frame.data.clone();
        Bilateral::parse("2:0.2:7")
            .unwrap()
            .apply(&mut frame, 10)
            .unwrap();
        assert_eq!(frame.data, original);
        for args in [
            "sigmaS=NaN",
            "sigmaR=2",
            "planes=1.5",
            "planes=16",
            "sigmaS=-1",
            "bad=1",
        ] {
            assert!(Bilateral::parse(args).is_err());
        }
        frame.data[0..2].copy_from_slice(&1024u16.to_le_bytes());
        assert!(Bilateral::parse("").unwrap().apply(&mut frame, 10).is_err());
    }
}

#[cfg(test)]
mod export_tests {

    #[test]
    fn bilateral_export_accepts_y4m_and_ffv1_and_keeps_packet_clock() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/bilateral-noise-edge.y4m");
        let temp = std::env::temp_dir().join(format!("fvid-bilateral-{}", std::process::id()));
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
                bilateral: Some("sigmaS=1:sigmaR=0.1:planes=1".into()),
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
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(8, 4, 1 << 20).unwrap();
            for index in 0..3 {
                assert_eq!(reader.packets[index].pts_ns, index as i64 * 500_000_000);
                assert_eq!(reader.packets[index].duration_ns, Some(500_000_000));
                let frame = decoder
                    .decode(&reader.read_packet(index).unwrap())
                    .unwrap()
                    .frame;
                for row in frame.data[..32].chunks_exact(8) {
                    assert!(row[..4].iter().all(|&v| (61..=63).contains(&v)));
                    assert!(row[4..].iter().all(|&v| (201..=203).contains(&v)));
                }
                assert!(frame.data[32..].iter().all(|v| *v == 128));
            }
        }
        std::fs::remove_dir_all(temp).unwrap();
    }
}
