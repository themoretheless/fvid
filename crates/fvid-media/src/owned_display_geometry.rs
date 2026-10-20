//! Exact stored crop/quarter-turn normalization; no color conversion or depth narrowing.
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;
fn named(sub: [usize; 2]) -> bool {
    matches!(sub, [1, 1] | [2, 1] | [2, 2] | [1, 2] | [4, 1] | [4, 4])
}
pub(crate) fn needs(frame: &GeometryFrame, crop: [u64; 4], rotation: u16) -> bool {
    crop != [0; 4] || rotation != 0 || frame.subsampling.is_some_and(|s| !named(s))
}
fn plane(
    data: &[u8],
    size: [usize; 2],
    rect: [usize; 4],
    rotation: u16,
    unit: usize,
) -> Result<Vec<u8>> {
    let [width, height] = size;
    let [left, top, w, h] = rect;
    if width == 0
        || height == 0
        || w == 0
        || h == 0
        || unit == 0
        || left.checked_add(w).is_none_or(|n| n > width)
        || top.checked_add(h).is_none_or(|n| n > height)
        || width.checked_mul(height).and_then(|n| n.checked_mul(unit)) != Some(data.len())
    {
        return Err("invalid stored display plane geometry".into());
    }
    let ow = if matches!(rotation, 90 | 270) { h } else { w };
    let mut output = buffer(
        w.checked_mul(h)
            .and_then(|n| n.checked_mul(unit))
            .ok_or("stored display size overflow")?,
    )?;
    for y in 0..h {
        for x in 0..w {
            let (xx, yy) = match rotation {
                0 => (x, y),
                90 => (h - 1 - y, x),
                180 => (w - 1 - x, h - 1 - y),
                270 => (y, w - 1 - x),
                _ => return Err("invalid stored display rotation".into()),
            };
            let from = ((top + y) * width + left + x) * unit;
            let to = (yy * ow + xx) * unit;
            output[to..to + unit].copy_from_slice(&data[from..from + unit]);
        }
    }
    Ok(output)
}
/// Materialize coded-coordinate crop and clockwise quarter turns at source depth.
/// Chroma is replicated to 4:4:4 when crop alignment or output axes require it.
/// Optional alpha remains full-resolution and unassociated.
pub fn apply(
    frame: &GeometryFrame,
    alpha: Option<&[u8]>,
    depth: u8,
    crop: [u64; 4],
    rotation: u16,
) -> Result<(GeometryFrame, Option<Vec<u8>>)> {
    if !(8..=16).contains(&depth) || !matches!(rotation, 0 | 90 | 180 | 270) {
        return Err("invalid stored display depth or rotation".into());
    }
    let (width, height) = (frame.width, frame.height);
    let mut borders = [0usize; 4];
    for (to, from) in borders.iter_mut().zip(crop) {
        *to = usize::try_from(from).map_err(|_| "stored display crop overflow")?;
    }
    let [left, top, right, bottom] = borders;
    let w = width
        .checked_sub(left)
        .and_then(|n| n.checked_sub(right))
        .filter(|&n| n > 0)
        .ok_or("stored crop removes picture width")?;
    let h = height
        .checked_sub(top)
        .and_then(|n| n.checked_sub(bottom))
        .filter(|&n| n > 0)
        .ok_or("stored crop removes picture height")?;
    let bytes = if depth == 8 { 1 } else { 2 };
    let quarter = matches!(rotation, 90 | 270);
    let size = if quarter { [h, w] } else { [w, h] };
    let alpha = alpha
        .map(|a| plane(a, [width, height], [left, top, w, h], rotation, bytes))
        .transpose()?;
    let (subsampling, data) = if let Some([sx, sy]) = frame.subsampling {
        if sx == 0 || sy == 0 {
            return Err("invalid stored display chroma geometry".into());
        }
        let out_sub = if quarter { [sy, sx] } else { [sx, sy] };
        let promote = left % sx != 0 || top % sy != 0 || !named(out_sub);
        let [ox, oy] = if promote { [1, 1] } else { out_sub };
        let count = width
            .checked_mul(height)
            .ok_or("stored display size overflow")?;
        let chroma = width
            .div_ceil(sx)
            .checked_mul(height.div_ceil(sy))
            .ok_or("stored display size overflow")?;
        if count
            .checked_add(
                chroma
                    .checked_mul(2)
                    .ok_or("stored display size overflow")?,
            )
            .and_then(|n| n.checked_mul(bytes))
            != Some(frame.data.len())
        {
            return Err("invalid stored display sample storage".into());
        }
        let total = size[0]
            .checked_mul(size[1])
            .and_then(|n| {
                size[0]
                    .div_ceil(ox)
                    .checked_mul(size[1].div_ceil(oy))
                    .and_then(|c| c.checked_mul(2))
                    .and_then(|c| n.checked_add(c))
            })
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("stored display output size overflow")?;
        let mut output = buffer(total)?;
        let mut input_offset = 0;
        let mut output_offset = 0;
        for component in 0..3 {
            let (dx, dy) = if component == 0 { (1, 1) } else { (sx, sy) };
            let (pw, ph) = (width.div_ceil(dx), height.div_ceil(dy));
            let length = pw * ph * bytes;
            let input = &frame.data[input_offset..input_offset + length];
            input_offset += length;
            let pixels = if promote && component != 0 {
                let mut expanded = buffer(
                    count
                        .checked_mul(bytes)
                        .ok_or("stored display promotion size overflow")?,
                )?;
                for y in 0..height {
                    for x in 0..width {
                        let from = ((y / dy) * pw + x / dx) * bytes;
                        let to = (y * width + x) * bytes;
                        expanded[to..to + bytes].copy_from_slice(&input[from..from + bytes]);
                    }
                }
                plane(
                    &expanded,
                    [width, height],
                    [left, top, w, h],
                    rotation,
                    bytes,
                )?
            } else {
                plane(
                    input,
                    [pw, ph],
                    [left / dx, top / dy, w.div_ceil(dx), h.div_ceil(dy)],
                    rotation,
                    bytes,
                )?
            };
            output[output_offset..output_offset + pixels.len()].copy_from_slice(&pixels);
            output_offset += pixels.len();
        }
        if output_offset != output.len() {
            return Err("stored display output planes disagree".into());
        }
        (Some([ox, oy]), output)
    } else {
        (
            None,
            plane(
                &frame.data,
                [width, height],
                [left, top, w, h],
                rotation,
                3 * bytes,
            )?,
        )
    };
    Ok((
        GeometryFrame {
            width: size[0],
            height: size[1],
            subsampling,
            data,
        },
        alpha,
    ))
}
