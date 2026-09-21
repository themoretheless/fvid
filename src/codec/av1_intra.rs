//! AV1 intra predictors with normative edge filtering and upsampling.
use super::av1_tables::*;
use crate::{Result, invalid};
pub struct Edges<'a> {
    /// Above/left include w+h samples, with unavailable extension replicated.
    pub above: &'a [u16],
    pub left: &'a [u16],
    pub corner: u16,
    pub have_above: bool,
    pub have_left: bool,
    pub edge_filter: bool,
    pub smooth_neighbor: bool,
}
fn weights(n: usize) -> &'static [i32] {
    match n {
        4 => &SM_WEIGHTS_TX_4X4,
        8 => &SM_WEIGHTS_TX_8X8,
        16 => &SM_WEIGHTS_TX_16X16,
        32 => &SM_WEIGHTS_TX_32X32,
        _ => &SM_WEIGHTS_TX_64X64,
    }
}
fn strength(w: usize, h: usize, smooth: bool, delta: i32) -> usize {
    let sum = w + h;
    let d = delta.abs();
    if smooth {
        match sum {
            0..=8 => {
                if d >= 64 {
                    2
                } else {
                    usize::from(d >= 40)
                }
            }
            9..=16 => {
                if d >= 48 {
                    2
                } else {
                    usize::from(d >= 20)
                }
            }
            17..=24 => 3 * usize::from(d >= 4),
            _ => 3,
        }
    } else {
        match sum {
            0..=8 => usize::from(d >= 56),
            9..=16 => usize::from(d >= 40),
            17..=24 => {
                if d >= 32 {
                    3
                } else if d >= 16 {
                    2
                } else {
                    usize::from(d >= 8)
                }
            }
            25..=32 => {
                if d >= 32 {
                    3
                } else if d >= 4 {
                    2
                } else {
                    1
                }
            }
            _ => 3,
        }
    }
}
fn filter(edge: &mut [i32], size: usize, strength: usize) {
    if strength == 0 {
        return;
    }
    let original = edge.to_vec();
    let kernel = [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]][strength - 1];
    for i in 1..size {
        let mut s = 0;
        for (j, tap) in kernel.iter().enumerate() {
            let k = (i as i32 - 2 + j as i32).clamp(0, size as i32 - 1) as usize;
            s += tap * original[k + 1];
        }
        edge[i + 1] = (s + 8) >> 4;
    }
}
fn upsample(edge: &mut [i32], n: usize, depth: u8) {
    let copy = edge.to_vec();
    edge[0] = copy[1];
    for i in 0..n {
        let a = copy[i.max(1)];
        let b = copy[i + 1];
        let c = copy[i + 2];
        let d = copy[(i + 3).min(n + 1)];
        edge[2 * i + 1] = ((-a + 9 * b + 9 * c - d + 8) >> 4).clamp(0, (1 << depth) - 1);
        edge[2 * i + 2] = c;
    }
}
pub fn predict(
    size: [usize; 2],
    depth: u8,
    mode: usize,
    angle_delta: i32,
    filter_mode: Option<usize>,
    edges: Edges<'_>,
) -> Result<Vec<u16>> {
    let [w, h] = size;
    if ![4, 8, 16, 32, 64].contains(&w)
        || ![4, 8, 16, 32, 64].contains(&h)
        || ![8, 10, 12].contains(&depth)
        || mode > 12
        || !(-3..=3).contains(&angle_delta)
        || edges.above.len() < w + h
        || edges.left.len() < w + h
        || filter_mode.is_some_and(|v| v > 4)
    {
        return Err(invalid("invalid AV1 intra predictor configuration"));
    }
    let mut top = vec![i32::from(edges.corner); 2 * (w + h) + 3];
    let mut left = top.clone();
    for i in 0..w + h {
        top[i + 2] = i32::from(edges.above[i]);
        left[i + 2] = i32::from(edges.left[i]);
    }
    let mut out = vec![0u16; w * h];
    if let Some(mode) = filter_mode {
        for by in (0..h).step_by(2) {
            for bx in (0..w).step_by(4) {
                let mut p = [0i32; 7];
                for (i, v) in p.iter_mut().enumerate() {
                    *v = if i < 5 {
                        if by == 0 {
                            top[bx + i + 1]
                        } else if bx == 0 && i == 0 {
                            left[by + 1]
                        } else {
                            i32::from(out[(by - 1) * w + bx + i - 1])
                        }
                    } else if bx == 0 {
                        left[by + i - 3]
                    } else {
                        i32::from(out[(by + i - 5) * w + bx - 1])
                    };
                }
                for y in 0..2 {
                    for x in 0..4 {
                        let sum = INTRA_FILTER_TAPS[mode][y * 4 + x]
                            .iter()
                            .zip(p)
                            .map(|(a, b)| a * b)
                            .sum::<i32>();
                        let value = sum.signum() * ((sum.abs() + 8) >> 4);
                        out[(by + y) * w + bx + x] = value.clamp(0, (1 << depth) - 1) as u16;
                    }
                }
            }
        }
        return Ok(out);
    }
    if (1..=8).contains(&mode) {
        let angle = [0, 90, 180, 45, 135, 113, 157, 203, 67][mode] + angle_delta * 3;
        let mut up_top = 0;
        let mut up_left = 0;
        if edges.edge_filter && angle != 90 && angle != 180 {
            if angle > 90 && angle < 180 && w + h >= 24 {
                let v = (5 * left[2] + 6 * top[1] + 5 * top[2] + 8) >> 4;
                left[1] = v;
                top[1] = v;
            }
            if edges.have_above {
                filter(
                    &mut top,
                    w + if angle < 90 { h } else { 0 } + 1,
                    strength(w, h, edges.smooth_neighbor, angle - 90),
                );
            }
            if edges.have_left {
                filter(
                    &mut left,
                    h + if angle > 180 { w } else { 0 } + 1,
                    strength(w, h, edges.smooth_neighbor, angle - 180),
                );
            }
            let limit = if edges.smooth_neighbor { 8 } else { 16 };
            up_top = usize::from((1..40).contains(&(angle - 90).abs()) && w + h <= limit);
            up_left = usize::from((1..40).contains(&(angle - 180).abs()) && w + h <= limit);
            if up_top != 0 {
                upsample(&mut top, w + if angle < 90 { h } else { 0 }, depth);
            }
            if up_left != 0 {
                upsample(&mut left, h + if angle > 180 { w } else { 0 }, depth);
            }
        }
        let dx = if angle < 90 {
            DR_INTRA_DERIVATIVE[angle as usize]
        } else if angle > 90 && angle < 180 {
            DR_INTRA_DERIVATIVE[(180 - angle) as usize]
        } else {
            0
        };
        let dy = if angle > 180 {
            DR_INTRA_DERIVATIVE[(270 - angle) as usize]
        } else if angle > 90 && angle < 180 {
            DR_INTRA_DERIVATIVE[(angle - 90) as usize]
        } else {
            0
        };
        let interp = |e: &[i32], base: i32, shift: i32| -> Result<u16> {
            let index = usize::try_from(base + 2)
                .map_err(|_| invalid("AV1 directional index below edge"))?;
            let a = *e
                .get(index)
                .ok_or_else(|| invalid("AV1 directional edge overflow"))?;
            let b = *e
                .get(index + 1)
                .ok_or_else(|| invalid("AV1 directional edge overflow"))?;
            Ok(((a * (32 - shift) + b * shift + 16) >> 5) as u16)
        };
        for y in 0..h {
            for x in 0..w {
                out[y * w + x] = if angle == 90 {
                    top[x + 2] as u16
                } else if angle == 180 {
                    left[y + 2] as u16
                } else if angle < 90 {
                    let idx = (y as i32 + 1) * dx;
                    let base = (idx >> (6 - up_top)) + ((x as i32) << up_top);
                    let max = ((w + h - 1) << up_top) as i32;
                    if base >= max {
                        top[max as usize + 2] as u16
                    } else {
                        interp(&top, base, ((idx << up_top) >> 1) & 31)?
                    }
                } else if angle < 180 {
                    let idx = ((x as i32) << 6) - (y as i32 + 1) * dx;
                    let base = idx >> (6 - up_top);
                    if base >= -(1 << up_top) {
                        interp(&top, base, ((idx << up_top) >> 1) & 31)?
                    } else {
                        let idx = ((y as i32) << 6) - (x as i32 + 1) * dy;
                        interp(&left, idx >> (6 - up_left), ((idx << up_left) >> 1) & 31)?
                    }
                } else {
                    let idx = (x as i32 + 1) * dy;
                    let base = (idx >> (6 - up_left)) + ((y as i32) << up_left);
                    interp(&left, base, ((idx << up_left) >> 1) & 31)?
                };
            }
        }
        return Ok(out);
    }
    let count = usize::from(edges.have_above) * w + usize::from(edges.have_left) * h;
    let sum = if edges.have_above {
        top[2..w + 2].iter().sum::<i32>()
    } else {
        0
    } + if edges.have_left {
        left[2..h + 2].iter().sum::<i32>()
    } else {
        0
    };
    let dc = if count == 0 {
        1 << (depth - 1)
    } else {
        (sum + count as i32 / 2) / count as i32
    };
    let wx = weights(w);
    let wy = weights(h);
    for y in 0..h {
        for x in 0..w {
            let (a, b, c) = (left[y + 2], top[x + 2], top[1]);
            out[y * w + x] = match mode {
                0 => dc,
                9 | 10 | 11 => {
                    let v = wy[y] * b + (256 - wy[y]) * left[h + 1];
                    let h = wx[x] * a + (256 - wx[x]) * top[w + 1];
                    match mode {
                        9 => (v + h + 256) >> 9,
                        10 => (v + 128) >> 8,
                        _ => (h + 128) >> 8,
                    }
                }
                _ => {
                    let base = a + b - c;
                    let (da, db, dc) = ((base - a).abs(), (base - b).abs(), (base - c).abs());
                    if da <= db && da <= dc {
                        a
                    } else if db <= dc {
                        b
                    } else {
                        c
                    }
                }
            } as u16;
        }
    }
    Ok(out)
}
