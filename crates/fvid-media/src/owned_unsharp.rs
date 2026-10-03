//! Planar unsharp masking using separable integer binomial convolution.
use crate::owned_frame::{buffer, GeometryFrame};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub struct Unsharp {
    planes: [[i32; 3]; 2],
}
impl Unsharp {
    pub fn parse(args: &str) -> Result<Self> {
        let mut result = Self {
            planes: [[5, 5, 65536], [5, 5, 0]],
        };
        let names = [
            "luma_msize_x",
            "luma_msize_y",
            "luma_amount",
            "chroma_msize_x",
            "chroma_msize_y",
            "chroma_amount",
        ];
        let aliases = ["lx", "ly", "la", "cx", "cy", "ca"];
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, text) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let key = *names.get(positional).ok_or("too many unsharp options")?;
                positional += 1;
                (key, option)
            };
            let index = names
                .iter()
                .zip(aliases)
                .position(|(name, alias)| *name == key.trim() || alias == key.trim())
                .ok_or("unknown unsharp option")?;
            let slot = index % 3;
            result.planes[index / 3][slot] = if slot == 2 {
                let amount = text
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| "unsharp requires numeric amounts")?;
                if !amount.is_finite() || !(-2.0..=5.0).contains(&amount) {
                    return Err("unsharp amount must be -2..5".into());
                }
                (f64::from(amount) * 65536.) as i32
            } else {
                let size = text
                    .trim()
                    .parse::<i32>()
                    .map_err(|_| "unsharp requires integer matrix sizes")?;
                if !(3..=63).contains(&size) || size % 2 == 0 {
                    return Err("unsharp requires odd matrix sizes 3..63".into());
                }
                size
            };
        }
        if result.planes.iter().any(|p| p[0] + p[1] - 2 > 25) {
            return Err("unsharp matrix scale exceeds 25 bits".into());
        }
        Ok(result)
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
            return Err("invalid unsharp sample geometry".into());
        }
        let [sx, sy] = frame.subsampling.ok_or("unsharp requires planar YUV")?;
        if sx == 0 || sy == 0 {
            return Err("invalid unsharp subsampling".into());
        }
        let width = frame.width;
        let height = frame.height;
        let cw = width.div_ceil(sx);
        let ch = height.div_ceil(sy);
        let y = width
            .checked_mul(height)
            .ok_or("unsharp geometry overflow")?;
        let c = cw.checked_mul(ch).ok_or("unsharp geometry overflow")?;
        let bps = if depth == 8 { 1 } else { 2 };
        let length = c
            .checked_mul(2)
            .and_then(|n| n.checked_add(y))
            .and_then(|n| n.checked_mul(bps))
            .ok_or("unsharp geometry overflow")?;
        if frame.data.len() != length {
            return Err("unsharp frame length mismatch".into());
        }
        let max = (1i64 << depth) - 1;
        if bps == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|v| i64::from(u16::from_le_bytes([v[0], v[1]])) > max)
        {
            return Err("unsharp sample exceeds precision".into());
        }
        let mut output = buffer(length)?;
        output.copy_from_slice(&frame.data);
        for (plane, (w, h, start)) in [(width, height, 0), (cw, ch, y), (cw, ch, y + c)]
            .into_iter()
            .enumerate()
        {
            let [mx, my, amount] = self.planes[usize::from(plane != 0)];
            if amount == 0 {
                continue;
            }
            let rx = mx as usize / 2;
            let ry = my as usize / 2;
            let weights = |size: i32| {
                let mut result = [0u64; 23];
                result[0] = 1;
                for i in 1..size as usize {
                    result[i] = result[i - 1] * (size as u64 - i as u64) / i as u64;
                }
                result
            };
            let wx = weights(mx);
            let wy = weights(my);
            let read = |index: usize| -> u64 {
                if bps == 1 {
                    u64::from(frame.data[index])
                } else {
                    u64::from(u16::from_le_bytes([
                        frame.data[2 * index],
                        frame.data[2 * index + 1],
                    ]))
                }
            };
            let count = w.checked_mul(h).ok_or("unsharp geometry overflow")?;
            let mut horizontal = Vec::new();
            horizontal
                .try_reserve_exact(count)
                .map_err(|e| e.to_string())?;
            horizontal.resize(count, 0u64);
            for row in 0..h {
                for x in 0..w {
                    let mut sum = 0;
                    for k in 0..mx as usize {
                        let px =
                            (x as i128 + k as i128 - rx as i128).clamp(0, w as i128 - 1) as usize;
                        sum += read(start + row * w + px) * wx[k];
                    }
                    horizontal[row * w + x] = sum;
                }
            }
            let shift = (mx + my - 2) as u32;
            for row in 0..h {
                for x in 0..w {
                    let mut sum = 0u64;
                    for k in 0..my as usize {
                        let py =
                            (row as i128 + k as i128 - ry as i128).clamp(0, h as i128 - 1) as usize;
                        sum += horizontal[py * w + x] * wy[k];
                    }
                    let blurred = (sum as u32).wrapping_add(1u32 << (shift - 1)) >> shift;
                    let index = start + row * w + x;
                    let original = read(index) as i64;
                    let value = (original
                        + (((original - i64::from(blurred)) * i64::from(amount)) >> 16))
                        .clamp(0, max);
                    if bps == 1 {
                        output[index] = value as u8;
                    } else {
                        output[2 * index..2 * index + 2]
                            .copy_from_slice(&(value as u16).to_le_bytes());
                    }
                }
            }
        }
        frame.data = output;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impulse_blur_and_sharpen_match_independent_binomial_values() {
        let mut original = GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([1, 1]),
            data: vec![0; 27],
        };
        original.data[4] = 100;
        original.data[9..].fill(128);
        let mut blurred = GeometryFrame {
            width: original.width,
            height: original.height,
            subsampling: original.subsampling,
            data: original.data.clone(),
        };
        Unsharp::parse("3:3:-1:3:3:0")
            .unwrap()
            .apply(&mut blurred, 8)
            .unwrap();
        assert_eq!(&blurred.data[..9], &[6, 13, 6, 13, 25, 13, 6, 13, 6]);
        assert_eq!(&blurred.data[9..], &[128; 18]);
        Unsharp::parse("3:3:1:3:3:0")
            .unwrap()
            .apply(&mut original, 8)
            .unwrap();
        assert_eq!(original.data[4], 175);
    }
    #[test]
    fn invalid_matrix_and_sample_storage_fail_atomically() {
        for args in ["4:5", "23:23", "la=NaN", "la=6", "unknown=1"] {
            assert!(Unsharp::parse(args).is_err());
        }
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0, 4, 0, 0, 0, 0],
        };
        let before = frame.data.clone();
        assert!(Unsharp::parse("").unwrap().apply(&mut frame, 10).is_err());
        assert_eq!(frame.data, before);
    }
}
