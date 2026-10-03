//! Owned inverse-mapped planar YUV and packed RGB rotation with bilinear sample interpolation.
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;

/// Rotate clockwise, using the public truncated bounding-canvas convention.
pub fn rotate(
    frame: &GeometryFrame,
    degrees: f64,
    depth: u8,
    full_range: bool,
) -> Result<GeometryFrame> {
    if !degrees.is_finite() || !(8..=16).contains(&depth) || frame.width == 0 || frame.height == 0 {
        return Err("invalid rotation geometry, angle or depth".into());
    }
    let rgb = frame.subsampling.is_none();
    if rgb && depth != 8 {
        return Err("RGB rotation requires 8-bit samples".into());
    }
    let [sx, sy] = frame.subsampling.unwrap_or([1, 1]);
    if sx == 0 || sy == 0 {
        return Err("invalid rotation chroma geometry".into());
    }
    let angle = degrees.rem_euclid(360.0);
    let (sin, cos) = if angle == 0.0 {
        (0.0, 1.0)
    } else if angle == 90.0 {
        (1.0, 0.0)
    } else if angle == 180.0 {
        (0.0, -1.0)
    } else if angle == 270.0 {
        (-1.0, 0.0)
    } else {
        angle.to_radians().sin_cos()
    };
    let extent = |value: f64| -> Result<usize> {
        if !value.is_finite() || value >= usize::MAX as f64 {
            return Err("rotation canvas overflow".into());
        }
        Ok((value as usize).max(1))
    };
    // Match the public rotw/roth contract: truncate, without chroma alignment.
    // Plane storage independently rounds chroma dimensions upwards.
    let width = extent(frame.width as f64 * cos.abs() + frame.height as f64 * sin.abs())?;
    let height = extent(frame.width as f64 * sin.abs() + frame.height as f64 * cos.abs())?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let sizes = |w: usize, h: usize| {
        [
            (w, h, 1, 1),
            (w.div_ceil(sx), h.div_ceil(sy), sx, sy),
            (w.div_ceil(sx), h.div_ceil(sy), sx, sy),
        ]
    };
    let storage = |w, h| -> Result<usize> {
        let mut samples = 0usize;
        for (w, h, _, _) in sizes(w, h) {
            samples = samples
                .checked_add(w.checked_mul(h).ok_or("rotation plane overflow")?)
                .ok_or("rotation storage overflow")?;
        }
        samples
            .checked_mul(bytes)
            .ok_or("rotation storage overflow".into())
    };
    if storage(frame.width, frame.height)? != frame.data.len() {
        return Err("rotation source storage mismatch".into());
    }
    let maximum = (1u32 << depth) - 1;
    if bytes == 2
        && frame
            .data
            .chunks_exact(2)
            .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > maximum)
    {
        return Err("rotation sample exceeds depth".into());
    }
    let mut output = GeometryFrame {
        width,
        height,
        subsampling: frame.subsampling,
        data: buffer(storage(width, height)?)?,
    };
    let mut source_base = 0;
    let mut dest_base = 0;
    for (plane, ((iw, ih, dx, dy), (ow, oh, _, _))) in sizes(frame.width, frame.height)
        .into_iter()
        .zip(sizes(width, height))
        .enumerate()
    {
        let fill = if rgb {
            0
        } else if plane > 0 {
            128u32 << (depth - 8)
        } else if full_range {
            0
        } else {
            16u32 << (depth - 8)
        };
        let sample = |x: i64, y: i64| -> f64 {
            if x < 0 || y < 0 || x >= iw as i64 || y >= ih as i64 {
                return f64::from(fill);
            }
            let offset = if rgb {
                (y as usize * iw + x as usize) * 3 + plane
            } else {
                source_base + (y as usize * iw + x as usize) * bytes
            };
            if bytes == 1 {
                f64::from(frame.data[offset])
            } else {
                f64::from(u16::from_le_bytes([
                    frame.data[offset],
                    frame.data[offset + 1],
                ]))
            }
        };
        for y in 0..oh {
            for x in 0..ow {
                let px = (x as f64 + 0.5) * dx as f64 - width as f64 / 2.0;
                let py = (y as f64 + 0.5) * dy as f64 - height as f64 / 2.0;
                let ix = (cos * px + sin * py + frame.width as f64 / 2.0) / dx as f64 - 0.5;
                let iy = (-sin * px + cos * py + frame.height as f64 / 2.0) / dy as f64 - 0.5;
                let x0 = ix.floor() as i64;
                let y0 = iy.floor() as i64;
                let tx = ix - x0 as f64;
                let ty = iy - y0 as f64;
                let top = sample(x0, y0) * (1.0 - tx) + sample(x0 + 1, y0) * tx;
                let bottom = sample(x0, y0 + 1) * (1.0 - tx) + sample(x0 + 1, y0 + 1) * tx;
                let value = (top * (1.0 - ty) + bottom * ty)
                    .round()
                    .clamp(0.0, f64::from(maximum)) as u16;
                let offset = if rgb {
                    (y * ow + x) * 3 + plane
                } else {
                    dest_base + (y * ow + x) * bytes
                };
                if bytes == 1 {
                    output.data[offset] = value as u8;
                } else {
                    output.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        source_base += iw * ih * bytes;
        dest_base += ow * oh * bytes;
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_rgb_keeps_channels_in_their_pixels() {
        let frame = GeometryFrame {
            width: 2,
            height: 1,
            subsampling: None,
            data: vec![10, 20, 30, 40, 50, 60],
        };
        let turned = rotate(&frame, 180.0, 8, true).unwrap();
        assert_eq!(turned.data, vec![40, 50, 60, 10, 20, 30]);
        assert!(turned.subsampling.is_none());
    }

    #[test]
    fn synthetic_video_rotates_each_frame_without_changing_source_clock() {
        let bytes = include_bytes!("../../../tests/fixtures/playback-errors/rotate-grid.y4m");
        let mut count = 0;
        crate::owned_y4m_decode::visit_reader_transformed(
            std::io::Cursor::new(bytes),
            &Default::default(),
            |header, data, pts, duration| {
                let frame = GeometryFrame {
                    width: header.width,
                    height: header.height,
                    subsampling: Some([1, 1]),
                    data: data.to_vec(),
                };
                let turned = rotate(&frame, 90.0, header.depth(), true)?;
                assert_eq!(&turned.data[..6], &[4, 1, 5, 2, 6, 3]);
                assert_eq!(pts, count * 500_000_000);
                assert_eq!(duration, 500_000_000);
                count += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn quarter_turn_reorders_samples_exactly() {
        let frame = GeometryFrame {
            width: 3,
            height: 2,
            subsampling: Some([1, 1]),
            data: [vec![1, 2, 3, 4, 5, 6], vec![128; 12]].concat(),
        };
        let rotated = rotate(&frame, 90.0, 8, true).unwrap();
        assert_eq!((rotated.width, rotated.height), (2, 3));
        assert_eq!(&rotated.data[..6], &[4, 1, 5, 2, 6, 3]);
        let roundtrip = rotate(&rotated, -90.0, 8, true).unwrap();
        assert_eq!(roundtrip.data, frame.data);
    }
    #[test]
    fn oblique_rotation_fits_canvas_and_fills_limited_black() {
        let frame = GeometryFrame {
            width: 8,
            height: 8,
            subsampling: Some([2, 2]),
            data: [vec![940u16; 64], vec![512; 32]]
                .concat()
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        };
        let output = rotate(&frame, 45.0, 10, false).unwrap();
        assert_eq!((output.width, output.height), (11, 11));
        assert_eq!(u16::from_le_bytes([output.data[0], output.data[1]]), 64);
        let center = (5 * 11 + 5) * 2;
        assert_eq!(
            u16::from_le_bytes([output.data[center], output.data[center + 1]]),
            940
        );
        assert!(
            output.data[11 * 11 * 2..]
                .chunks_exact(2)
                .all(|p| u16::from_le_bytes([p[0], p[1]]) == 512)
        );
    }
}
