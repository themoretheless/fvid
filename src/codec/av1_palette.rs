//! AV1 palette colors and diagonal color-index maps (spec 5.11.46–49).
use super::{Cdfs, SymbolDecoder, av1_cdfs};
use crate::{Result, invalid};

pub(super) fn colors(
    d: &mut SymbolDecoder<'_>,
    cache: &[u16],
    n: usize,
    depth: u8,
    chroma: bool,
) -> Result<([u16; 8], u32)> {
    let mut values = [0u16; 8];
    let mut index = 0;
    let mut hits = 0;
    for &color in cache {
        if index == n {
            break;
        }
        if d.bit()? {
            values[index] = color;
            index += 1;
            hits += 1;
        }
    }
    if index < n {
        values[index] = d.literal(depth)? as u16;
        index += 1;
    }
    let mut bits = if index < n {
        depth - 3 + d.literal(2)? as u8
    } else {
        0
    };
    let maximum = (1u32 << depth) - 1;
    while index < n {
        let delta = d.literal(bits)? + u32::from(!chroma);
        let value = (u32::from(values[index - 1]) + delta).min(maximum);
        values[index] = value as u16;
        let range = (1u32 << depth) - value - u32::from(!chroma);
        let ceil = if range <= 1 {
            0
        } else {
            32 - (range - 1).leading_zeros()
        };
        bits = bits.min(ceil as u8);
        index += 1;
    }
    values[..n].sort_unstable();
    Ok((values, hits))
}
pub(super) fn v_colors(d: &mut SymbolDecoder<'_>, n: usize, depth: u8) -> Result<[u16; 8]> {
    let mut values = [0u16; 8];
    if d.bit()? {
        let bits = depth - 4 + d.literal(2)? as u8;
        let modulus = 1i32 << depth;
        values[0] = d.literal(depth)? as u16;
        for index in 1..n {
            let mut delta = d.literal(bits)? as i32;
            if delta != 0 && d.bit()? {
                delta = -delta;
            }
            values[index] = (i32::from(values[index - 1]) + delta).rem_euclid(modulus) as u16;
        }
    } else {
        for value in &mut values[..n] {
            *value = d.literal(depth)? as u16;
        }
    }
    Ok(values)
}
fn uniform(d: &mut SymbolDecoder<'_>, n: usize) -> Result<usize> {
    let bits = n.ilog2() + 1;
    let threshold = (1usize << bits) - n;
    let value = d.literal((bits - 1) as u8)? as usize;
    Ok(if value < threshold {
        value
    } else {
        (value << 1) - threshold + usize::from(d.bit()?)
    })
}
fn context(
    map: &[u8],
    stride: usize,
    row: usize,
    col: usize,
    n: usize,
) -> Result<(usize, [usize; 8])> {
    let mut scores = [0usize; 8];
    let mut order = std::array::from_fn(|i| i);
    if col > 0 {
        scores[map[row * stride + col - 1] as usize] += 2;
    }
    if row > 0 && col > 0 {
        scores[map[(row - 1) * stride + col - 1] as usize] += 1;
    }
    if row > 0 {
        scores[map[(row - 1) * stride + col] as usize] += 2;
    }
    for index in 0..3 {
        let mut best = index;
        for next in index + 1..n {
            if scores[next] > scores[best] {
                best = next;
            }
        }
        let score = scores[best];
        let color = order[best];
        for next in (index + 1..=best).rev() {
            scores[next] = scores[next - 1];
            order[next] = order[next - 1];
        }
        scores[index] = score;
        order[index] = color;
    }
    let hash = scores[0] + 2 * scores[1] + 2 * scores[2];
    let ctx = [-1, -1, 0, -1, -1, 4, 3, 2, 1]
        .get(hash)
        .copied()
        .unwrap_or(-1);
    if ctx < 0 {
        return Err(invalid("AV1 invalid palette color context"));
    }
    Ok((ctx as usize, order))
}
pub(super) fn map(
    d: &mut SymbolDecoder<'_>,
    c: &mut Cdfs,
    n: usize,
    chroma: bool,
    size: [usize; 2],
    onscreen: [usize; 2],
) -> Result<Vec<u8>> {
    let [w, h] = size;
    let [width, height] = onscreen;
    if !(2..=8).contains(&n)
        || width == 0
        || height == 0
        || width > w
        || height > h
        || w > 64
        || h > 64
    {
        return Err(invalid("AV1 invalid palette map dimensions"));
    }
    let mut map = vec![0u8; w * h];
    map[0] = uniform(d, n)? as u8;
    for diagonal in 1..width + height - 1 {
        let last = diagonal.min(width - 1);
        let first = diagonal.saturating_sub(height - 1);
        for col in (first..=last).rev() {
            let row = diagonal - col;
            let (ctx, order) = context(&map, w, row, col, n)?;
            let id = if chroma {
                av1_cdfs::PALETTE_SIZE_2_UV_COLOR
            } else {
                av1_cdfs::PALETTE_SIZE_2_Y_COLOR
            } + n
                - 2;
            let index = super::symbol(d, c, id, [ctx])?;
            map[row * w + col] = order[index] as u8;
        }
    }
    for row in 0..height {
        for col in width..w {
            map[row * w + col] = map[row * w + width - 1];
        }
    }
    for row in height..h {
        for col in 0..w {
            map[row * w + col] = map[(height - 1) * w + col];
        }
    }
    Ok(map)
}
