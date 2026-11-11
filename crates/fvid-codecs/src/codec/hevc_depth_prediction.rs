//! Annex I.8.4.4.2.5 depth sub-block partition DC prediction.
use super::hevc_depth_tables::Tables;
use crate::{Result, invalid};

pub(crate) fn single_value(depth: u8, selected: Option<u16>) -> Result<u16> {
    if !(8..=16).contains(&depth) {
        return Err(invalid("invalid HEVC single-sample depth"));
    }
    let value = selected.unwrap_or(1u16 << (depth - 1));
    if u32::from(value) >= (1u32 << depth) {
        return Err(invalid("HEVC single-sample neighbour exceeds range"));
    }
    Ok(value)
}

/// INTRA_SINGLE (I.8.4.4.2.4). Availability must be preserved: this mode does
/// not use ordinary intra-reference substitution for a missing neighbour.
pub fn single(
    size: usize,
    depth: u8,
    skip_mode: u8,
    top: &[Option<u16>],
    left: &[Option<u16>],
    output: &mut [u16],
) -> Result<()> {
    if !(4..=64).contains(&size)
        || !size.is_power_of_two()
        || !(8..=16).contains(&depth)
        || !matches!(skip_mode, 2 | 3)
        || top.len() != size
        || left.len() != size
        || output.len() != size * size
    {
        return Err(invalid("invalid HEVC single-sample prediction geometry"));
    }
    let neighbour = if skip_mode == 2 {
        left[size / 2]
    } else {
        top[size / 2]
    };
    let value = single_value(depth, neighbour)?;
    output.fill(value);
    Ok(())
}

/// I-79..80 contour mask from the corresponding reconstructed texture block.
/// Threshold is the truncated four-corner mean (no rounding addition).
/// Input and output are validated completely before output mutation.
pub fn contour_pattern(
    texture: &[u16],
    stride: usize,
    origin: [usize; 2],
    size: usize,
    texture_depth: u8,
    output: &mut [bool],
) -> Result<()> {
    if !(4..=64).contains(&size)
        || !size.is_power_of_two()
        || !(8..=16).contains(&texture_depth)
        || output.len() != size * size
    {
        return Err(invalid("invalid HEVC contour geometry"));
    }
    let right = origin[0]
        .checked_add(size)
        .filter(|&v| v <= stride)
        .ok_or_else(|| invalid("HEVC contour exceeds texture row"))?;
    let bottom = origin[1]
        .checked_add(size - 1)
        .ok_or_else(|| invalid("HEVC contour geometry overflow"))?;
    bottom
        .checked_mul(stride)
        .and_then(|v| v.checked_add(right))
        .filter(|&v| v <= texture.len())
        .ok_or_else(|| invalid("HEVC contour exceeds texture buffer"))?;
    let limit = (1u32 << texture_depth) - 1;
    for y in origin[1]..=bottom {
        if texture[y * stride + origin[0]..y * stride + right]
            .iter()
            .any(|&v| u32::from(v) > limit)
        {
            return Err(invalid("HEVC contour texture exceeds sample range"));
        }
    }
    let threshold = ([
        origin[1] * stride + origin[0],
        origin[1] * stride + right - 1,
        bottom * stride + origin[0],
        bottom * stride + right - 1,
    ]
    .iter()
    .map(|&i| u32::from(texture[i]))
    .sum::<u32>()
        >> 2) as u16;
    for y in 0..size {
        for x in 0..size {
            output[y * size + x] = texture[(origin[1] + y) * stride + origin[0] + x] > threshold;
        }
    }
    Ok(())
}

/// Neighbours must already have undergone the normative availability substitution.
/// Pattern and output use row-major order; pattern values select offset 0 or 1.
pub struct Partition<'a> {
    pub size: usize,
    pub top: &'a [u16],
    pub left: &'a [u16],
    pub pattern: &'a [bool],
    pub offsets: [i64; 2],
}

/// Write signed, unclipped prediction samples (I-82..94). No allocations.
/// All validation and lookup operations precede output mutation.
pub fn predict(
    tables: &Tables,
    layer: u8,
    block: Partition<'_>,
    output: &mut [i128],
) -> Result<()> {
    let n = block.size;
    if !(4..=64).contains(&n)
        || !n.is_power_of_two()
        || block.top.len() != 2 * n
        || block.left.len() != 2 * n
        || block.pattern.len() != n * n
        || output.len() != n * n
        || !(8..=16).contains(&tables.bit_depth)
    {
        return Err(invalid("invalid HEVC depth partition geometry"));
    }
    let limit = (1u32 << tables.bit_depth) - 1;
    if block
        .top
        .iter()
        .chain(block.left)
        .any(|&v| u32::from(v) > limit)
    {
        return Err(invalid(
            "HEVC depth partition neighbour exceeds sample range",
        ));
    }
    let first = block.pattern[0];
    let vertical = first != block.pattern[n - 1];
    let horizontal = first != block.pattern[(n - 1) * n];
    let average = |a: u16, b: u16| ((u32::from(a) + u32::from(b)) >> 1) as u16;
    let (lt, br) = if vertical == horizontal {
        let br = if horizontal {
            average(block.left[n - 1], block.top[n - 1])
        } else if block.top[0].abs_diff(block.top[2 * n - 1])
            > block.left[0].abs_diff(block.left[2 * n - 1])
        {
            block.top[2 * n - 1]
        } else {
            block.left[2 * n - 1]
        };
        (average(block.left[0], block.top[0]), br)
    } else if horizontal {
        (block.top[(n - 1) / 2], block.left[n - 1])
    } else {
        (block.left[(n - 1) / 2], block.top[n - 1])
    };
    let mut predictions = [0; 2];
    for segment in 0..2 {
        predictions[segment] = tables.dc_prediction(
            layer,
            if (segment != 0) == first { lt } else { br },
            block.offsets[segment],
        )?;
    }
    for (value, &segment) in output.iter_mut().zip(block.pattern) {
        *value = predictions[usize::from(segment)];
    }
    Ok(())
}
