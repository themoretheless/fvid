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
            0 | 2 | 5 | 6 => Ok(Self::Bt601),
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
    // Limited YUV uses nominal 8-bit code excursions scaled to RGB16;
    // 255 << 8 is the nominal RGB white, with headroom up to 65535.
    let rgb_white = if full { 65535.0 } else { 65280.0 };
    let (kr, kb) = match matrix {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
    };
    let kg = 1.0 - kr - kb;
    let quantize = |coefficient: f64| (coefficient * 8192.0).round() / 8192.0;
    let quantized = [
        quantize(255.0 / 219.0),
        quantize(2.0 * (1.0 - kr) * 255.0 / 224.0),
        quantize(2.0 * (1.0 - kb) * 255.0 / 224.0),
        quantize(2.0 * kb * (1.0 - kb) * 255.0 / (224.0 * kg)),
        quantize(2.0 * kr * (1.0 - kr) * 255.0 / (224.0 * kg)),
    ];
    let compat =
        !full && matches!(sampling, ChromaSampling::Point) && matches!(matrix, Matrix::Bt601);
    let reverse_rows: [[f64; 3]; 3] = [
        [0.299, 0.587, 0.114],
        [-0.169, -0.331, 0.5],
        [0.5, -0.419, -0.081],
    ];
    let reverse_coefficients = std::array::from_fn::<_, 3, _>(|i| {
        reverse_rows[i].map(|coefficient| {
            (coefficient * if i == 0 { 219.0 / 255.0 } else { 224.0 / 255.0 } * 32768.0).round()
                as i64
        })
    });
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
                    let (r, g, b) = if !full && matches!(sampling, ChromaSampling::Point) {
                        // Compatibility RGB conversion quantizes matrix coefficients
                        // to 13 fractional bits, before combining source samples.
                        let raw_y = (read(&frame.data, row * frame.width + col) as f64 - black)
                            * 256.0
                            / scale;
                        let raw_u = (read(&frame.data, y + cell) as f64 - center) * 256.0 / scale;
                        let raw_v =
                            (read(&frame.data, y + c + cell) as f64 - center) * 256.0 / scale;
                        let base = raw_y * quantized[0];
                        let red = base + raw_v * quantized[1];
                        let blue = base + raw_u * quantized[2];
                        let green = base - raw_u * quantized[3] - raw_v * quantized[4];
                        (red / rgb_white, green / rgb_white, blue / rgb_white)
                    } else {
                        (r, g, b)
                    };
                    for sample in [r, g, b] {
                        rgb.extend_from_slice(
                            &((sample * rgb_white).round().clamp(0.0, 65535.0) as u16)
                                .to_le_bytes(),
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
                        u16::from_le_bytes([rgb[at + i * 2], rgb[at + i * 2 + 1]]) as f64
                            / rgb_white
                    };
                    let r = channel(0);
                    let g = channel(1);
                    let b = channel(2);
                    let luma = kr * r + kg * g + kb * b;
                    let (output_y, uv) = if compat {
                        let raw = [r, g, b].map(|channel| (channel * rgb_white).round() as i64);
                        let values = reverse_coefficients.map(|coefficients| {
                            coefficients
                                .into_iter()
                                .zip(raw)
                                .map(|(a, b)| a * b)
                                .sum::<i64>()
                        });
                        let luma16 = (values[0] + (4096i64 << 15) + (1 << 14)) >> 15;
                        let u16 = (values[1] + (32768i64 << 15) + (1 << 14)) >> 15;
                        let v16 = (values[2] + (32768i64 << 15) + (1 << 14)) >> 15;
                        let reduced = if depth == 8 {
                            [
                                ordered_reduce8(luma16, col, row, 0) as f64,
                                ordered_reduce8(u16, cx, cy, 0) as f64,
                                ordered_reduce8(v16, cx, cy, 3) as f64,
                            ]
                        } else {
                            [
                                luma16 as f64 * scale / 256.0,
                                u16 as f64 * scale / 256.0,
                                v16 as f64 * scale / 256.0,
                            ]
                        };
                        (
                            reduced[0],
                            (
                                (reduced[1] - center) / crange,
                                (reduced[2] - center) / crange,
                            ),
                        )
                    } else {
                        (
                            black + luma * yrange,
                            (
                                (b - luma) / (2.0 * (1.0 - kb)),
                                (r - luma) / (2.0 * (1.0 - kr)),
                            ),
                        )
                    };
                    if rgb[at..at + 6] != before[at..at + 6] {
                        write(&mut output, row * frame.width + col, output_y);
                    }
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

#[cfg(test)]
mod nominal_range_tests {
    use super::*;
    #[test]
    fn limited_nominal_white_retains_rgb_headroom_at_all_yuv_precisions() {
        for depth in [8, 12, 16] {
            for full in [false, true] {
                let center = 1u16 << (depth - 1);
                let white = if full {
                    ((1u32 << depth) - 1) as u16
                } else {
                    235u16 << (depth - 8)
                };
                let samples = [white, center, center];
                let mut frame = GeometryFrame {
                    width: 1,
                    height: 1,
                    subsampling: Some([1, 1]),
                    data: if depth == 8 {
                        samples.map(|v| v as u8).to_vec()
                    } else {
                        samples.into_iter().flat_map(u16::to_le_bytes).collect()
                    },
                };
                let original = frame.data.clone();
                let mut calls = 0;
                filter_rgb16(&mut frame, depth, full, Matrix::Bt601, |rgb| {
                    let decoded: Vec<u16> = rgb
                        .chunks_exact(2)
                        .map(|p| u16::from_le_bytes([p[0], p[1]]))
                        .collect();
                    assert_eq!(decoded, vec![if full { 65535 } else { 65280 }; 3]);
                    calls += 1;
                    Ok(())
                })
                .unwrap();
                assert_eq!(calls, 1);
                assert_eq!(frame.data, original);
            }
        }
    }
}

#[cfg(test)]
mod forward_acceptance_tests {
    use super::*;
    #[test]
    fn quantized_forward_stage_matches_committed_synthetic_rgb_references() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let bytes = std::fs::read(root.join(format!("colorize-grid-{depth}.y4m"))).unwrap();
            let start = bytes.iter().position(|v| *v == b'\n').unwrap() + 7;
            let mut frame = GeometryFrame {
                width: 3,
                height: 3,
                subsampling: Some([2, 2]),
                data: bytes[start..start + 17 * if depth == 8 { 1 } else { 2 }].to_vec(),
            };
            let locations: [&[usize]; 4] = [&[0, 1, 3, 4], &[2, 5], &[6, 7], &[8]];
            let mut cell = 0;
            let mut actual = vec![0u8; 54];
            filter_rgb16_sampled(
                &mut frame,
                depth,
                false,
                Matrix::Bt601,
                ChromaSampling::Point,
                |rgb| {
                    for (pixel, index) in rgb.chunks_exact(6).zip(locations[cell]) {
                        actual[index * 6..index * 6 + 6].copy_from_slice(pixel);
                    }
                    cell += 1;
                    Ok(())
                },
            )
            .unwrap();
            let expected =
                std::fs::read(root.join(format!("colorhold-rgb-stage-{depth}.raw"))).unwrap();
            assert_eq!(expected.len(), 54);
            assert_eq!(actual, expected, "depth={depth}");
            assert_eq!(cell, 4);
        }
    }
}

// A three-level ordered threshold matrix, generated from two-bit ranks.
// Each 8x8 tile visits all 64 thresholds; V uses a shifted horizontal phase.
fn ordered_reduce8(sample: i64, x: usize, y: usize, phase: usize) -> i64 {
    let x = x.wrapping_add(phase);
    let ranks = [[1i64, 2, 3, 0], [0, 3, 2, 1], [2, 1, 0, 3]];
    let mut rank = 0;
    for (bit, weight) in [(0, 16), (1, 4), (2, 1)] {
        let index = (((y >> bit) & 1) << 1) | ((x >> bit) & 1);
        rank += ranks[bit][index] * weight;
    }
    (((sample >> 1) + rank * 2) >> 7).clamp(0, 255)
}

/// Frame-wide RGB processing for filters whose limits depend on all pixels.
/// RGB pixels follow chroma-cell traversal order, suitable for per-pixel filters
/// and frame-wide statistics. Both passes use the same traversal.
pub(crate) fn filter_rgb16_frame(
    frame: &mut GeometryFrame,
    depth: u8,
    full: bool,
    matrix: Matrix,
    mut filter: impl FnMut(&mut [u8]) -> Result<()>,
) -> Result<()> {
    let length = frame
        .width
        .checked_mul(frame.height)
        .and_then(|v| v.checked_mul(6))
        .ok_or("RGB frame geometry overflow")?;
    let mut rgb = Vec::new();
    filter_rgb16_sampled(frame, depth, full, matrix, ChromaSampling::Point, |cell| {
        if rgb.capacity() == 0 {
            rgb.try_reserve_exact(length)
                .map_err(|_| "RGB frame allocation failed")?;
        }
        rgb.extend_from_slice(cell);
        Ok(())
    })?;
    if rgb.len() != length {
        return Err("RGB frame traversal length mismatch".into());
    }
    filter(&mut rgb)?;
    let mut at = 0;
    filter_rgb16_sampled(frame, depth, full, matrix, ChromaSampling::Point, |cell| {
        let end = at + cell.len();
        cell.copy_from_slice(&rgb[at..end]);
        at = end;
        Ok(())
    })
}

#[cfg(test)]
mod whole_frame_tests {
    #[test]
    fn whole_frame_callback_sees_all_odd_edge_pixels_once_and_failure_is_atomic() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let source = std::fs::read(root.join("colorize-grid-8.y4m")).unwrap();
        let start = source.iter().position(|v| *v == b'\n').unwrap() + 7;
        let mut frame = crate::owned_frame::GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([2, 2]),
            data: source[start..start + 17].to_vec(),
        };
        let before = frame.data.clone();
        let mut calls = 0;
        let error = super::filter_rgb16_frame(&mut frame, 8, false, super::Matrix::Bt601, |rgb| {
            calls += 1;
            assert_eq!(rgb.len(), 3 * 3 * 6);
            rgb.fill(0);
            Err("synthetic filter error".into())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error, "synthetic filter error");
        assert_eq!(frame.data, before);
        super::filter_rgb16_frame(&mut frame, 8, false, super::Matrix::Bt601, |_| Ok(())).unwrap();
        assert_eq!(frame.data, before);
    }
}

/// Float RGB cells retain negative values and highlight headroom until final YUV quantization.
pub(crate) fn filter_rgb_f32_sampled(
    frame: &mut GeometryFrame,
    depth: u8,
    full: bool,
    matrix: Matrix,
    sampling: ChromaSampling,
    mut filter: impl FnMut(&mut [f32]) -> Result<()>,
) -> Result<()> {
    let [sx, sy] = frame
        .subsampling
        .ok_or("float YUV conversion requires planar samples")?;
    if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0 {
        return Err("invalid float YUV geometry or precision".into());
    }
    let y = frame
        .width
        .checked_mul(frame.height)
        .ok_or("float YUV geometry overflow")?;
    let cw = frame.width.div_ceil(sx);
    let ch = frame.height.div_ceil(sy);
    let c = cw.checked_mul(ch).ok_or("float YUV geometry overflow")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let maximum = (1u32 << depth) - 1;
    if c.checked_mul(2)
        .and_then(|v| v.checked_add(y))
        .and_then(|v| v.checked_mul(bytes))
        != Some(frame.data.len())
    {
        return Err("float YUV sample length mismatch".into());
    }
    let read = |i: usize| -> f64 {
        if bytes == 1 {
            frame.data[i] as f64
        } else {
            u16::from_le_bytes([frame.data[i * 2], frame.data[i * 2 + 1]]) as f64
        }
    };
    if (0..y + 2 * c).any(|i| read(i) > maximum as f64) {
        return Err("float YUV sample exceeds precision".into());
    }
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
    let size = frame
        .width
        .min(sx)
        .checked_mul(frame.height.min(sy))
        .and_then(|v| v.checked_mul(3))
        .ok_or("float RGB cell overflow")?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(frame.data.len())
        .map_err(|_| "float YUV output allocation failed")?;
    output.extend_from_slice(&frame.data);
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(size)
        .map_err(|_| "float RGB cell allocation failed")?;
    let mut before = Vec::new();
    before
        .try_reserve_exact(size)
        .map_err(|_| "float RGB copy allocation failed")?;
    let write = |data: &mut [u8], i: usize, value: f64| {
        let value = value.round_ties_even().clamp(0.0, maximum as f64) as u16;
        if bytes == 1 {
            data[i] = value as u8;
        } else {
            data[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
        }
    };
    for cy in 0..ch {
        for cx in 0..cw {
            let cell = cy * cw + cx;
            let x0 = cx * sx;
            let y0 = cy * sy;
            let x1 = x0.saturating_add(sx).min(frame.width);
            let y1 = y0.saturating_add(sy).min(frame.height);
            let u = (read(y + cell) - center) / crange;
            let v = (read(y + c + cell) - center) / crange;
            rgb.clear();
            for row in y0..y1 {
                for col in x0..x1 {
                    let luma = (read(row * frame.width + col) - black) / yrange;
                    let r = luma + 2.0 * (1.0 - kr) * v;
                    let b = luma + 2.0 * (1.0 - kb) * u;
                    let g = (luma - kr * r - kb * b) / kg;
                    rgb.extend([r as f32, g as f32, b as f32]);
                }
            }
            before.clear();
            before.extend_from_slice(&rgb);
            filter(&mut rgb)?;
            if rgb.iter().any(|v| !v.is_finite()) {
                return Err("float RGB filter returned nonfinite sample".into());
            }
            if rgb == before {
                continue;
            }
            let mut sums = [0.0f64; 2];
            let mut point = [0.0f64; 2];
            let mut at = 0;
            for row in y0..y1 {
                for col in x0..x1 {
                    let [r, g, b] = [
                        rgb[at * 3] as f64,
                        rgb[at * 3 + 1] as f64,
                        rgb[at * 3 + 2] as f64,
                    ];
                    let luma = kr * r + kg * g + kb * b;
                    write(&mut output, row * frame.width + col, black + yrange * luma);
                    let uv = [
                        (b - luma) / (2.0 * (1.0 - kb)),
                        (r - luma) / (2.0 * (1.0 - kr)),
                    ];
                    if at == 0 {
                        point = uv;
                    }
                    for i in 0..2 {
                        sums[i] += uv[i];
                    }
                    at += 1;
                }
            }
            let uv = if matches!(sampling, ChromaSampling::Point) {
                point
            } else {
                sums.map(|v| v / at as f64)
            };
            write(&mut output, y + cell, center + crange * uv[0]);
            write(&mut output, y + c + cell, center + crange * uv[1]);
        }
    }
    frame.data = output;
    Ok(())
}

/// Whole float RGB frame in raster order; publish YUV only after filtering succeeds.
pub(crate) fn filter_rgb_f32_frame(
    frame: &mut GeometryFrame,
    depth: u8,
    full: bool,
    matrix: Matrix,
    mut filter: impl FnMut(&mut [f32]) -> Result<()>,
) -> Result<()> {
    let [sx, sy] = frame
        .subsampling
        .ok_or("float RGB frame requires planar YUV")?;
    if sx == 0 || sy == 0 {
        return Err("invalid float RGB frame subsampling".into());
    }
    let width = frame.width;
    let height = frame.height;
    let cw = width.div_ceil(sx);
    let length = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(3))
        .ok_or("float RGB frame geometry overflow")?;
    let mut rgb = Vec::new();
    let mut cell_index = 0;
    filter_rgb_f32_sampled(frame, depth, full, matrix, ChromaSampling::Point, |cell| {
        if rgb.is_empty() {
            rgb.try_reserve_exact(length)
                .map_err(|_| "float RGB frame allocation failed")?;
            rgb.resize(length, 0.0);
        }
        let x0 = (cell_index % cw) * sx;
        let y0 = (cell_index / cw) * sy;
        let cell_width = (width - x0).min(sx);
        for (i, pixel) in cell.chunks_exact(3).enumerate() {
            let at = ((y0 + i / cell_width) * width + x0 + i % cell_width) * 3;
            rgb[at..at + 3].copy_from_slice(pixel);
        }
        cell_index += 1;
        Ok(())
    })?;
    filter(&mut rgb)?;
    cell_index = 0;
    filter_rgb_f32_sampled(frame, depth, full, matrix, ChromaSampling::Point, |cell| {
        let x0 = (cell_index % cw) * sx;
        let y0 = (cell_index / cw) * sy;
        let cell_width = (width - x0).min(sx);
        for (i, pixel) in cell.chunks_exact_mut(3).enumerate() {
            let at = ((y0 + i / cell_width) * width + x0 + i % cell_width) * 3;
            pixel.copy_from_slice(&rgb[at..at + 3]);
        }
        cell_index += 1;
        Ok(())
    })
}

#[cfg(test)]
mod float_frame_tests {
    #[test]
    fn whole_float_frame_has_raster_order_and_failure_preserves_input() {
        let input: Vec<u8> = (0..9).map(|i| 16 + i * 23).chain([128; 8]).collect();
        let mut frame = crate::owned_frame::GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([2, 2]),
            data: input.clone(),
        };
        let mut calls = 0;
        let result =
            super::filter_rgb_f32_frame(&mut frame, 8, false, super::Matrix::Bt601, |rgb| {
                calls += 1;
                assert_eq!(rgb.len(), 27);
                for (i, pixel) in rgb.chunks_exact(3).enumerate() {
                    let expected = (i as f64 * 23.0 / 219.0) as f32;
                    assert!(pixel.iter().all(|v| (*v - expected).abs() < f32::EPSILON));
                }
                Err("synthetic frame filter failure".into())
            });
        assert_eq!(calls, 1);
        assert_eq!(result.unwrap_err(), "synthetic frame filter failure");
        assert_eq!(frame.data, input);
        super::filter_rgb_f32_frame(&mut frame, 8, false, super::Matrix::Bt601, |_| Ok(()))
            .unwrap();
        assert_eq!(frame.data, input);
    }
}
