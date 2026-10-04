//! Planar YUV sample conversion for owned RGB filters.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}
impl Matrix {
    pub fn from_code(code: u8) -> Result<Self> {
        match code {
            1 => Ok(Self::Bt709),
            2 | 5 | 6 => Ok(Self::Bt601),
            9 => Ok(Self::Bt2020),
            _ => Err("RGB filter colour matrix is not implemented".into()),
        }
    }
}
/// Run an RGB16 filter per chroma cell, preserving source precision and layout.
/// Chroma reconstruction uses the shared source cell; output chroma is averaged
/// over the actual luma samples, including partial cells at odd frame edges.
pub fn filter_rgb16(
    frame: &mut GeometryFrame,
    depth: u8,
    full: bool,
    matrix: Matrix,
    filter: impl FnMut(&mut [u8]) -> Result<()>,
) -> Result<()> {
    filter_rgb16_sampled(frame, depth, full, matrix, ChromaSampling::Average, filter)
}
#[derive(Clone, Copy, Debug)]
pub enum ChromaSampling {
    Average,
    Point,
}
pub fn filter_rgb16_sampled(
    frame: &mut GeometryFrame,
    depth: u8,
    full: bool,
    matrix: Matrix,
    sampling: ChromaSampling,
    mut filter: impl FnMut(&mut [u8]) -> Result<()>,
) -> Result<()> {
    let [sx, sy] = frame
        .subsampling
        .ok_or("YUV conversion requires planar samples")?;
    if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 {
        return Err("invalid YUV conversion geometry or precision".into());
    }
    let y = frame
        .width
        .checked_mul(frame.height)
        .ok_or("YUV geometry overflow")?;
    let cw = frame.width.div_ceil(sx);
    let ch = frame.height.div_ceil(sy);
    let c = cw.checked_mul(ch).ok_or("YUV geometry overflow")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    if c.checked_mul(2)
        .and_then(|n| n.checked_add(y))
        .and_then(|n| n.checked_mul(bytes))
        != Some(frame.data.len())
    {
        return Err("YUV sample length mismatch".into());
    }
    let maximum = (1u32 << depth) - 1;
    let read = |data: &[u8], i: usize| {
        if bytes == 1 {
            data[i] as u32
        } else {
            u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) as u32
        }
    };
    if bytes == 2 && (0..y + 2 * c).any(|i| read(&frame.data, i) > maximum) {
        return Err("YUV sample exceeds precision".into());
    }
    let write = |data: &mut [u8], i: usize, value: f64| {
        let value = value.round().clamp(0.0, maximum as f64) as u16;
        if bytes == 1 {
            data[i] = value as u8;
        } else {
            data[2 * i..2 * i + 2].copy_from_slice(&value.to_le_bytes());
        }
    };
    let scale = (1u32 << (depth - 8)) as f64;
    let (black, yrange, crange) = if full {
        (0.0, maximum as f64, maximum as f64)
    } else {
        (16.0 * scale, 219.0 * scale, 224.0 * scale)
    };
    let center = (1u32 << (depth - 1)) as f64;
    let (kr, kb) = match matrix {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
    };
    let kg = 1.0 - kr - kb;
    let mut output = Vec::new();
    output
        .try_reserve_exact(frame.data.len())
        .map_err(|e| e.to_string())?;
    output.extend_from_slice(&frame.data);
    let mut rgb = Vec::new();
    for cy in 0..ch {
        for cx in 0..cw {
            rgb.clear();
            let cell = cy * cw + cx;
            let u = (read(&frame.data, y + cell) as f64 - center) / crange;
            let v = (read(&frame.data, y + c + cell) as f64 - center) / crange;
            let x0 = cx * sx;
            let y0 = cy * sy;
            let x1 = x0.saturating_add(sx).min(frame.width);
            let y1 = y0.saturating_add(sy).min(frame.height);
            let count = (x1 - x0).checked_mul(y1 - y0).ok_or("YUV cell overflow")?;
            rgb.try_reserve(count.checked_mul(6).ok_or("RGB allocation overflow")?)
                .map_err(|e| e.to_string())?;
            for row in y0..y1 {
                for col in x0..x1 {
                    let luma = (read(&frame.data, row * frame.width + col) as f64 - black) / yrange;
                    let r = luma + 2.0 * (1.0 - kr) * v;
                    let b = luma + 2.0 * (1.0 - kb) * u;
                    let g = (luma - kr * r - kb * b) / kg;
                    for sample in [r, g, b] {
                        rgb.extend_from_slice(
                            &((sample.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes(),
                        );
                    }
                }
            }
            let before = rgb.clone();
            filter(&mut rgb)?;
            if before == rgb {
                continue;
            }
            let mut usum = 0.0;
            let mut vsum = 0.0;
            let mut index = 0;
            let mut point = (0.0, 0.0);
            for row in y0..y1 {
                for col in x0..x1 {
                    let at = index * 6;
                    let channel = |i: usize| {
                        u16::from_le_bytes([rgb[at + i * 2], rgb[at + i * 2 + 1]]) as f64 / 65535.0
                    };
                    let r = channel(0);
                    let g = channel(1);
                    let b = channel(2);
                    let luma = kr * r + kg * g + kb * b;
                    if rgb[at..at + 6] != before[at..at + 6] {
                        write(&mut output, row * frame.width + col, black + luma * yrange);
                    }
                    let uv = (
                        (b - luma) / (2.0 * (1.0 - kb)),
                        (r - luma) / (2.0 * (1.0 - kr)),
                    );
                    if index == 0 {
                        point = uv;
                    }
                    usum += uv.0;
                    vsum += uv.1;
                    index += 1;
                }
            }
            let (u, v) = match sampling {
                ChromaSampling::Average => (usum / count as f64, vsum / count as f64),
                ChromaSampling::Point => point,
            };
            write(&mut output, y + cell, center + crange * u);
            write(&mut output, y + c + cell, center + crange * v);
        }
    }
    frame.data = output;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn point_sampling_and_cell_average_remain_distinct_at_subsampled_edges() {
        let source = vec![128; 6];
        let mut point = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: source.clone(),
        };
        let mut average = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: source,
        };
        let paint = |rgb: &mut [u8]| {
            for (i, pixel) in rgb.chunks_exact_mut(6).enumerate() {
                let color = if i == 0 {
                    [65535u16, 0, 0]
                } else {
                    [0u16, 0, 65535]
                };
                for (sample, bytes) in color.into_iter().zip(pixel.chunks_exact_mut(2)) {
                    bytes.copy_from_slice(&sample.to_le_bytes());
                }
            }
            Ok(())
        };
        filter_rgb16_sampled(
            &mut point,
            8,
            true,
            Matrix::Bt601,
            ChromaSampling::Point,
            paint,
        )
        .unwrap();
        filter_rgb16(&mut average, 8, true, Matrix::Bt601, paint).unwrap();
        assert_eq!(&point.data[4..], &[85, 255]);
        assert!(average.data[4] > 128 && average.data[5] < 200);
        assert_eq!(&point.data[..4], &average.data[..4]);
    }
}
