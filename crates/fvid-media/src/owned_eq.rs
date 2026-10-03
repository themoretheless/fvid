//! Constant equalization through bounded per-plane sample tables.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Equalizer {
    tables: [[u8; 256]; 3],
    values: [f64; 8],
    high_depth: [std::sync::OnceLock<Result<Vec<u16>>>; 8],
}
impl Equalizer {
    pub fn parse(args: &str) -> Result<Self> {
        let names = [
            "contrast",
            "brightness",
            "saturation",
            "gamma",
            "gamma_r",
            "gamma_g",
            "gamma_b",
            "gamma_weight",
            "eval",
        ];
        let mut values = [1., 0., 1., 1., 1., 1., 1., 1.];
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let key = *names.get(positional).ok_or("too many eq options")?;
                positional += 1;
                (key, option)
            };
            let index = names
                .iter()
                .position(|k| *k == key.trim())
                .ok_or("unknown eq option")?;
            if index == 8 {
                if !matches!(text.trim(), "init" | "frame" | "0" | "1") {
                    return Err("invalid eq evaluation mode".into());
                }
                continue;
            }
            let value = crate::owned_expression::constant(text.trim())?;
            if !value.is_finite() {
                return Err("eq parameters must be finite".into());
            }
            let (min, max) = match index {
                0 => (-1000., 1000.),
                1 => (-1., 1.),
                2 => (0., 3.),
                7 => (0., 1.),
                _ => (0.1, 10.),
            };
            values[index] = f64::from(value.clamp(min, max) as f32);
        }
        let mut result = Self {
            tables: [[0; 256]; 3],
            values,
            high_depth: std::array::from_fn(|_| std::sync::OnceLock::new()),
        };
        let gammas = [
            values[3] * values[5],
            (values[6] / values[5]).sqrt(),
            (values[4] / values[5]).sqrt(),
        ];
        for plane in 0..3 {
            let contrast = if plane == 0 { values[0] } else { values[2] };
            let brightness = if plane == 0 { values[1] } else { 0. };
            let gamma = gammas[plane];
            for (sample, output) in result.tables[plane].iter_mut().enumerate() {
                let value = if contrast == 1. && brightness == 0. && gamma == 1. {
                    sample as i64
                } else if gamma == 1. && contrast.abs() < 7.9 {
                    let scale = (contrast * 4096.) as i64;
                    let bias = ((100. * brightness + 100.) as i64 * 511) / 200 - 128 - scale / 32;
                    ((sample as i64 * scale) >> 12) + bias
                } else {
                    let normalized = contrast * (sample as f64 / 255. - 0.5) + 0.5 + brightness;
                    if normalized <= 0. {
                        0
                    } else {
                        let corrected =
                            normalized * (1. - values[7]) + normalized.powf(1. / gamma) * values[7];
                        if corrected >= 1. {
                            255
                        } else {
                            (corrected * 256.) as i64
                        }
                    }
                };
                *output = value.clamp(0, 255) as u8;
            }
        }
        Ok(result)
    }
    fn build_high_depth_tables(&self, depth: u8) -> Result<Vec<u16>> {
        let count = 1usize << depth;
        let maximum = (count - 1) as f64;
        let mut tables = Vec::new();
        tables
            .try_reserve_exact(count * 3)
            .map_err(|e| e.to_string())?;
        let v = self.values;
        let gammas = [v[3] * v[5], (v[6] / v[5]).sqrt(), (v[4] / v[5]).sqrt()];
        for (plane, gamma) in gammas.into_iter().enumerate() {
            let contrast = if plane == 0 { v[0] } else { v[2] };
            let brightness = if plane == 0 { v[1] } else { 0.0 };
            for sample in 0..count {
                let output = if contrast == 1.0 && brightness == 0.0 && gamma == 1.0 {
                    sample as u16
                } else {
                    let normalized = contrast * (sample as f64 / maximum - 0.5) + 0.5 + brightness;
                    let corrected = if normalized <= 0.0 {
                        0.0
                    } else {
                        normalized * (1.0 - v[7]) + normalized.powf(1.0 / gamma) * v[7]
                    };
                    (corrected * count as f64).clamp(0.0, maximum) as u16
                };
                tables.push(output);
            }
        }
        Ok(tables)
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
            return Err("owned eq requires 8..=16-bit planar YUV".into());
        }
        let [sx, sy] = frame.subsampling.ok_or("eq requires planar YUV")?;
        if sx == 0 || sy == 0 {
            return Err("invalid eq subsampling".into());
        }
        let luma = frame
            .width
            .checked_mul(frame.height)
            .ok_or("eq geometry overflow")?;
        let chroma = frame
            .width
            .div_ceil(sx)
            .checked_mul(frame.height.div_ceil(sy))
            .ok_or("eq geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let expected = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("eq geometry overflow")?;
        if frame.data.len() != expected {
            return Err("eq frame length mismatch".into());
        }
        if depth > 8 {
            let count = 1usize << depth;
            if frame
                .data
                .chunks_exact(2)
                .any(|b| usize::from(u16::from_le_bytes([b[0], b[1]])) >= count)
            {
                return Err("eq sample exceeds precision".into());
            }
            let tables = self.high_depth[usize::from(depth - 9)]
                .get_or_init(|| self.build_high_depth_tables(depth))
                .as_ref()
                .map_err(Clone::clone)?;
            let (y, uv) = frame.data.split_at_mut(luma * 2);
            let (u, v) = uv.split_at_mut(chroma * 2);
            for (index, plane) in [y, u, v].into_iter().enumerate() {
                let table = &tables[index * count..(index + 1) * count];
                for sample in plane.chunks_exact_mut(2) {
                    let value = u16::from_le_bytes([sample[0], sample[1]]);
                    sample.copy_from_slice(&table[usize::from(value)].to_le_bytes());
                }
            }
            return Ok(());
        }
        let (y, uv) = frame.data.split_at_mut(luma);
        let (u, v) = uv.split_at_mut(chroma);
        for (plane, table) in [y, u, v].into_iter().zip(&self.tables) {
            for sample in plane {
                *sample = table[usize::from(*sample)];
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constant_contrast_and_saturation_have_exact_independent_plane_values() {
        let mut frame = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![16, 64, 128, 235, 160, 96],
        };
        Equalizer::parse("contrast=0:saturation=0")
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        assert_eq!(frame.data, [127; 6]);
    }
    #[test]
    fn high_depth_identity_precision_and_neutral_plane_tables() {
        for depth in 9..=16 {
            let maximum = ((1u32 << depth) - 1) as u16;
            let samples = [1u16, 2, 100, maximum, 17, maximum - 1];
            let mut frame = GeometryFrame {
                width: 2,
                height: 2,
                subsampling: Some([2, 2]),
                data: samples.into_iter().flat_map(u16::to_le_bytes).collect(),
            };
            let original = frame.data.clone();
            Equalizer::parse("")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            assert_eq!(frame.data, original);
            Equalizer::parse("contrast=0:saturation=0")
                .unwrap()
                .apply(&mut frame, depth)
                .unwrap();
            assert!(
                frame
                    .data
                    .chunks_exact(2)
                    .all(|b| u16::from_le_bytes([b[0], b[1]]) == 1u16 << (depth - 1))
            );
            if depth < 16 {
                frame.data[..2].copy_from_slice(&(1u16 << depth).to_le_bytes());
                let original = frame.data.clone();
                assert!(
                    Equalizer::parse("brightness=1")
                        .unwrap()
                        .apply(&mut frame, depth)
                        .is_err()
                );
                assert_eq!(frame.data, original);
            }
        }
    }
    #[test]
    fn high_depth_gamma_uses_full_precision_and_reuses_tables() {
        let mut frame = GeometryFrame {
            width: 3,
            height: 1,
            subsampling: Some([1, 1]),
            data: [0u16, 1024, 4095, 2048, 2048, 2048, 2048, 2048, 2048]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        let filter = Equalizer::parse("gamma=2").unwrap();
        filter.apply(&mut frame, 12).unwrap();
        let samples: Vec<u16> = frame
            .data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples, [0, 2048, 4095, 2048, 2048, 2048, 2048, 2048, 2048]);
        let table = filter.high_depth[3]
            .get()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ptr();
        filter.apply(&mut frame, 12).unwrap();
        assert_eq!(
            table,
            filter.high_depth[3]
                .get()
                .unwrap()
                .as_ref()
                .unwrap()
                .as_ptr()
        );
    }
    #[test]
    fn invalid_parameters_and_storage_cannot_mutate_samples() {
        for args in ["gamma=t", "gamma=NaN", "eval=unknown", "unknown=1"] {
            assert!(Equalizer::parse(args).is_err());
        }
        let mut frame = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![1; 5],
        };
        assert!(Equalizer::parse("").unwrap().apply(&mut frame, 8).is_err());
        assert_eq!(frame.data, [1; 5]);
    }
}

#[cfg(test)]
mod export_tests {

    #[test]
    fn high_depth_eq_exports_y4m_and_ffv1_without_fallback() {
        let directory =
            std::env::temp_dir().join(format!("fvid-eq-high-depth-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../tests/fixtures/playback-errors/eq-precision-{depth}.y4m"
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
                    eq: Some("gamma=2:saturation=0".into()),
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
                        let count = 1u32 << depth;
                        assert_eq!(samples[0], 0);
                        assert_eq!(samples[1], if depth == 12 { 64 } else { 256 });
                        assert_eq!(samples[2], if depth == 12 { 90 } else { 362 });
                        assert_eq!(samples[3], (count / 2) as u16);
                        assert_eq!(samples[4], (count - 1) as u16);
                        assert!(samples[9..].iter().all(|&n| n == (count / 2) as u16));
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
