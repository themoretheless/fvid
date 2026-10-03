//! Constant equalization through bounded per-plane sample tables.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Equalizer {
    tables: [[u8; 256]; 3],
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
            let value = text
                .trim()
                .parse::<f64>()
                .map_err(|_| "owned eq requires constant numeric parameters")?;
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
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if depth != 8 || frame.width == 0 || frame.height == 0 {
            return Err("owned eq requires 8-bit planar YUV".into());
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
        let expected = chroma
            .checked_mul(2)
            .and_then(|n| n.checked_add(luma))
            .ok_or("eq geometry overflow")?;
        if frame.data.len() != expected {
            return Err("eq frame length mismatch".into());
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
