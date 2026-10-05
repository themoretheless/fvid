
#[derive(Clone, Copy)]
struct Plane {
    offset: usize,
    width: usize,
    height: usize,
    bytes: usize,
    sx: usize,
    sy: usize,
}
fn planes(frame: &GeometryFrame, depth: u8) -> Result<Vec<Plane>> {
    if frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth) {
        return Err(invalid("invalid overlay geometry or sample depth"));
    }
    let mut result = Vec::with_capacity(3);
    let mut offset = 0usize;
    let sampling = match frame.subsampling {
        Some([sx, sy]) if matches!(sx, 1 | 2 | 4) && matches!(sy, 1 | 2 | 4) => {
            vec![(1, 1), (sx, sy), (sx, sy)]
        }
        Some(_) => return Err(invalid("unsupported overlay chroma sampling")),
        None if depth == 8 => vec![(1, 1)],
        None => return Err(invalid("RGB overlay requires 8-bit samples")),
    };
    for (sx, sy) in sampling {
        let width = frame.width.div_ceil(sx);
        let height = frame.height.div_ceil(sy);
        let bytes = if frame.subsampling.is_none() {
            3
        } else if depth > 8 {
            2
        } else {
            1
        };
        let size = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(bytes))
            .ok_or_else(|| invalid("overlay plane size overflow"))?;
        result.push(Plane {
            offset,
            width,
            height,
            bytes,
            sx,
            sy,
        });
        offset = offset
            .checked_add(size)
            .ok_or_else(|| invalid("overlay storage size overflow"))?;
    }
    if frame.data.len() != offset {
        return Err(invalid("overlay sample storage disagrees with geometry"));
    }
    if depth > 8 {
        let max = ((1u32 << depth) - 1) as u16;
        if frame
            .data
            .as_chunks::<2>().0.iter()
            .any(|p| u16::from_le_bytes([p[0], p[1]]) > max)
        {
            return Err(invalid("overlay sample exceeds declared depth"));
        }
    }
    Ok(result)
}
/// Replace the covered destination samples with an opaque foreground.
/// Inputs must already share colour encoding, range, depth and chroma sampling.
/// Offscreen pixels are clipped. Chroma-subsampled placement must be aligned;
/// rotation, colour conversion, alpha blending and frame scheduling are separate.
/// Both complete buffers are validated before any destination mutation.
pub fn overlay_opaque(
    destination: &mut GeometryFrame,
    foreground: &GeometryFrame,
    depth: u8,
    x: i64,
    y: i64,
) -> Result<()> {
    overlay_opaque_depth(destination, foreground, depth, depth, false, x, y)
}

/// Composite matching colour/range/sampling with integer depth conversion.
/// Limited-range codes scale by powers of two (rounded when reducing depth).
/// Full-range luma scales its endpoints; chroma also preserves its neutral centre.
/// Only covered samples are converted; no intermediate picture is allocated.
pub fn overlay_opaque_depth(
    destination: &mut GeometryFrame,
    foreground: &GeometryFrame,
    destination_depth: u8,
    foreground_depth: u8,
    full_range: bool,
    x: i64,
    y: i64,
) -> Result<()> {
    if destination.subsampling != foreground.subsampling {
        return Err(invalid("overlay requires matching chroma sampling"));
    }
    let target = planes(destination, destination_depth)?;
    let source = planes(foreground, foreground_depth)?;
    if let Some([sx, sy]) = destination.subsampling
        && (x % i64::try_from(sx).unwrap() != 0 || y % i64::try_from(sy).unwrap() != 0) {
            return Err(invalid("overlay placement must align with chroma samples"));
        }
    for (plane, (dst, src)) in target.iter().zip(source).enumerate() {
        let px = i128::from(x) / dst.sx as i128;
        let py = i128::from(y) / dst.sy as i128;
        let left = px.max(0);
        let top = py.max(0);
        let right = (px + src.width as i128).min(dst.width as i128);
        let bottom = (py + src.height as i128).min(dst.height as i128);
        if right <= left || bottom <= top {
            continue;
        }
        let dx = left as usize;
        let sx = (left - px) as usize;
        let count = (right - left) as usize * dst.bytes;
        for row in top..bottom {
            let to = dst.offset + (row as usize * dst.width + dx) * dst.bytes;
            let from = src.offset + ((row - py) as usize * src.width + sx) * src.bytes;
            if destination_depth == foreground_depth {
                destination.data[to..to + count]
                    .copy_from_slice(&foreground.data[from..from + count]);
            } else {
                for column in 0..(right - left) as usize {
                    let input = from + column * src.bytes;
                    let value = if src.bytes == 1 {
                        u32::from(foreground.data[input])
                    } else {
                        u32::from(u16::from_le_bytes([
                            foreground.data[input],
                            foreground.data[input + 1],
                        ]))
                    };
                    let value = convert_sample(
                        value,
                        foreground_depth,
                        destination_depth,
                        full_range,
                        plane != 0,
                    );
                    let output = to + column * dst.bytes;
                    if dst.bytes == 1 {
                        destination.data[output] = value as u8;
                    } else {
                        destination.data[output..output + 2]
                            .copy_from_slice(&(value as u16).to_le_bytes());
                    }
                }
            }
        }
    }
    Ok(())
}

fn convert_sample(value: u32, source: u8, target: u8, full: bool, chroma: bool) -> u32 {
    let maximum = (1u32 << target) - 1;
    if !full {
        return if target >= source {
            value << (target - source)
        } else {
            ((value + (1 << (source - target - 1))) >> (source - target)).min(maximum)
        };
    }
    let scale = |v: u32, from: u32, to: u32| (v * to + from / 2) / from;
    if chroma {
        let from = 1u32 << (source - 1);
        let to = 1u32 << (target - 1);
        if value <= from {
            scale(value, from, to)
        } else {
            to + scale(value - from, from - 1, to - 1)
        }
    } else {
        scale(value, (1u32 << source) - 1, maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precision_conversion_preserves_range_endpoints_and_neutral_chroma() {
        for target in [10, 12, 16] {
            let shift = target - 8;
            for value in [0, 16, 128, 235, 255] {
                assert_eq!(
                    convert_sample(value, 8, target, false, false),
                    value << shift
                );
                assert_eq!(
                    convert_sample(value << shift, target, 8, false, false),
                    value
                );
            }
            let max = (1u32 << target) - 1;
            assert_eq!(convert_sample(255, 8, target, true, false), max);
            assert_eq!(
                convert_sample(128, 8, target, true, true),
                1 << (target - 1)
            );
            assert_eq!(convert_sample(255, 8, target, true, true), max);
            assert_eq!(convert_sample(max, target, 8, false, false), 255);
        }
        assert_eq!(convert_sample(512, 10, 8, true, true), 128);
        assert_eq!(convert_sample(66, 10, 8, false, false), 17);
    }
    #[test]
    fn mixed_depth_clipping_and_invalid_source_are_atomic() {
        let mut dst = GeometryFrame {
            width: 4,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 24],
        };
        let src = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![16, 17, 18, 19, 128, 255],
        };
        overlay_opaque_depth(&mut dst, &src, 10, 8, false, 2, 0).unwrap();
        let expected: [u16; 12] = [0, 0, 64, 68, 0, 0, 72, 76, 0, 512, 0, 1020];
        assert_eq!(
            dst.data,
            expected
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        );
        let saved = dst.data.clone();
        let mut broken = src;
        broken.data.pop();
        assert!(overlay_opaque_depth(&mut dst, &broken, 10, 8, false, 2, 0).is_err());
        assert_eq!(dst.data, saved);
    }
    #[test]
    fn rgb_negative_offsets_clip_exact_pixels() {
        let mut dst = GeometryFrame {
            width: 3,
            height: 2,
            subsampling: None,
            data: vec![0; 18],
        };
        let src = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: None,
            data: (1..=12).collect(),
        };
        overlay_opaque(&mut dst, &src, 8, -1, 1).unwrap();
        assert_eq!(
            dst.data,
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 5, 6, 0, 0, 0, 0, 0, 0]
        );
        let saved = dst.data.clone();
        overlay_opaque(&mut dst, &src, 8, i64::MAX, i64::MIN).unwrap();
        assert_eq!(dst.data, saved);
    }
    #[test]
    fn odd_yuv420_preserves_plane_borders_and_high_depth() {
        for depth in [8, 10, 12, 16] {
            let pack = |v: u16| {
                if depth == 8 {
                    vec![v as u8]
                } else {
                    v.to_le_bytes().to_vec()
                }
            };
            let mut dst = GeometryFrame {
                width: 5,
                height: 3,
                subsampling: Some([2, 2]),
                data: pack(0).repeat(27),
            };
            let src = GeometryFrame {
                width: 3,
                height: 3,
                subsampling: Some([2, 2]),
                data: (1..=17).flat_map(pack).collect(),
            };
            overlay_opaque(&mut dst, &src, depth, 2, 0).unwrap();
            let expected = [
                0, 0, 1, 2, 3, 0, 0, 4, 5, 6, 0, 0, 7, 8, 9, 0, 10, 11, 0, 12, 13, 0, 14, 15, 0,
                16, 17,
            ];
            assert_eq!(
                dst.data,
                expected.into_iter().flat_map(pack).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn invalid_foreground_or_alignment_leaves_destination_unchanged() {
        let mut dst = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 12],
        };
        let mut src = GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 12],
        };
        let saved = dst.data.clone();
        assert!(overlay_opaque(&mut dst, &src, 10, 1, 0).is_err());
        assert_eq!(dst.data, saved);
        src.data[10..12].copy_from_slice(&1024u16.to_le_bytes());
        assert!(overlay_opaque(&mut dst, &src, 10, 0, 0).is_err());
        assert_eq!(dst.data, saved);
        src.data.pop();
        assert!(overlay_opaque(&mut dst, &src, 10, 0, 0).is_err());
        assert_eq!(dst.data, saved);
    }
}
