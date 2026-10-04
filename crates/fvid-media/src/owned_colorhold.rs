//! Owned RGB colour selection, without a filter graph or external codec.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct ColorHold {
    color: [u8; 3],
    similarity: f32,
    blend: f32,
}
impl ColorHold {
    pub fn parse(args: &str) -> Result<Self> {
        let mut filter = Self {
            color: [0; 3],
            similarity: 0.01,
            blend: 0.0,
        };
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = match option.split_once('=') {
                Some(pair) => pair,
                None => {
                    let names = ["color", "similarity", "blend"];
                    let name = *names.get(positional).ok_or("too many colorhold options")?;
                    positional += 1;
                    (name, option)
                }
            };
            match name.trim() {
                "color" => filter.color = parse_color(value.trim())?,
                "similarity" | "blend" => {
                    let number = crate::owned_expression::constant(value.trim())? as f32;
                    let minimum = if name.trim() == "similarity" {
                        0.00001
                    } else {
                        0.0
                    };
                    if !number.is_finite() || !(minimum..=1.0).contains(&number) {
                        return Err("invalid colorhold similarity or blend".into());
                    }
                    if name.trim() == "similarity" {
                        filter.similarity = number;
                    } else {
                        filter.blend = number;
                    }
                }
                _ => return Err("unknown colorhold option".into()),
            }
        }
        Ok(filter)
    }
    /// In-place RGB/RGBA processing. Samples above 8 bits are little-endian.
    /// Alpha is preserved; geometry and precision are validated before mutation.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !matches!(depth, 8 | 16) || !matches!(channels, 3 | 4) {
            return Err("colorhold requires RGB/RGBA at 8 or 16 bits".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let stride = channels * bytes;
        if data.len() % stride != 0 {
            return Err("colorhold RGB frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        let scale = 255.0 / maximum as f64;
        let reciprocal = 1.0f32 / self.blend;
        for pixel in data.chunks_exact_mut(stride) {
            let rgb: [u32; 3] = std::array::from_fn(|i| {
                if bytes == 1 {
                    pixel[i] as u32
                } else {
                    u16::from_le_bytes([pixel[2 * i], pixel[2 * i + 1]]) as u32
                }
            });
            let distance = rgb
                .iter()
                .zip(self.color)
                .map(|(v, key)| (*v as f64 * scale - key as f64).powi(2))
                .sum::<f64>()
                .sqrt()
                / (255.0 * 3.0f64.sqrt());
            let amount = if reciprocal < 10000.0 {
                ((distance - self.similarity as f64) * reciprocal as f64).clamp(0.0, 1.0)
                    * maximum as f64
            } else if distance > self.similarity as f64 {
                maximum as f64
            } else {
                0.0
            };
            let amount = amount as u64;
            if amount == 0 {
                continue;
            }
            let grey = rgb.iter().map(|v| *v as u64).sum::<u64>() / 3;
            for i in 0..3 {
                let sample = ((grey * amount
                    + rgb[i] as u64 * (maximum as u64 - amount)
                    + maximum as u64 / 2)
                    >> depth) as u16;
                if bytes == 1 {
                    pixel[i] = sample as u8;
                } else {
                    pixel[2 * i..2 * i + 2].copy_from_slice(&sample.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}
fn parse_color(value: &str) -> Result<[u8; 3]> {
    let named = match value.to_ascii_lowercase().as_str() {
        "black" => Some([0, 0, 0]),
        "white" => Some([255; 3]),
        "red" => Some([255, 0, 0]),
        "green" => Some([0, 128, 0]),
        "lime" => Some([0, 255, 0]),
        "blue" => Some([0, 0, 255]),
        "yellow" => Some([255, 255, 0]),
        "cyan" => Some([0, 255, 255]),
        "magenta" => Some([255, 0, 255]),
        _ => None,
    };
    if let Some(color) = named {
        return Ok(color);
    }
    let hex = value
        .strip_prefix('#')
        .or_else(|| value.strip_prefix("0x"))
        .unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("colorhold colour requires a supported name or RRGGBB hex".into());
    }
    let n = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_selected_colour_and_alpha_and_desaturates_other_pixels() {
        let mut rgb = vec![255, 0, 0, 17, 0, 255, 0, 99, 0, 0, 255, 255];
        ColorHold::parse("color=red")
            .unwrap()
            .apply_rgb(&mut rgb, 8, 4)
            .unwrap();
        assert_eq!(rgb, [255, 0, 0, 17, 85, 85, 85, 99, 85, 85, 85, 255]);
        let mut pixels = [65535u16, 0, 0, 123, 0, 65535, 0, 456]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        ColorHold::parse("0xFF0000")
            .unwrap()
            .apply_rgb(&mut pixels, 16, 4)
            .unwrap();
        let decoded: Vec<u16> = pixels
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(decoded, [65535, 0, 0, 123, 21845, 21845, 21845, 456]);
    }
    #[test]
    fn blend_and_rejection() {
        let mut hard = vec![255, 0, 0];
        let mut soft = hard.clone();
        ColorHold::parse("")
            .unwrap()
            .apply_rgb(&mut hard, 8, 3)
            .unwrap();
        ColorHold::parse("black:0.01:1")
            .unwrap()
            .apply_rgb(&mut soft, 8, 3)
            .unwrap();
        assert_eq!(hard, [85; 3]);
        assert!(soft[0] > hard[0] && soft[1] < hard[1]);
        for args in [
            "color=no-such-colour",
            "similarity=0",
            "blend=NaN",
            "blend=2",
            "wat=1",
        ] {
            assert!(ColorHold::parse(args).is_err());
        }
        let mut invalid = vec![1, 2];
        assert!(
            ColorHold::parse("")
                .unwrap()
                .apply_rgb(&mut invalid, 8, 3)
                .is_err()
        );
        assert_eq!(invalid, [1, 2]);
    }
}
