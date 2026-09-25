//! AV1 deblocking and constrained directional enhancement filtering.
use super::{
    av1_frame::Cdef,
    av1_picture::{Picture, Plane},
};

/// Samples p6..p0,q0..q6. Filter width is 4/8/16; chroma uses six taps.
#[inline]
pub(crate) fn edge(
    samples: [u16; 14],
    depth: u8,
    width: usize,
    chroma: bool,
    level: i32,
    sharpness: u8,
) -> [u16; 14] {
    let mut out = samples;
    let s = samples.map(i32::from);
    let scale = 1 << (depth - 8);
    let shift = u8::from(sharpness > 0) + u8::from(sharpness > 4);
    let mut limit = (level >> shift).max(1);
    if sharpness > 0 {
        limit = limit.min(9 - i32::from(sharpness));
    }
    let blimit = (2 * (level + 2) + limit) * scale;
    limit *= scale;
    let threshold = (level >> 4) * scale;
    let span = if width == 4 {
        1
    } else if chroma {
        2
    } else {
        3
    };
    let mut mask = 2 * (s[6] - s[7]).abs() + (s[5] - s[8]).abs() / 2 > blimit;
    for i in 1..=span {
        mask |= (s[6 - i] - s[7 - i]).abs() > limit || (s[7 + i] - s[6 + i]).abs() > limit;
    }
    if mask {
        return out;
    }
    let flat = width >= 8
        && (1..=span).all(|i| (s[6 - i] - s[6]).abs() <= scale && (s[7 + i] - s[7]).abs() <= scale);
    let flat2 = width == 16
        && (4..=6).all(|i| (s[6 - i] - s[6]).abs() <= scale && (s[7 + i] - s[7]).abs() <= scale);
    if flat {
        let (n, n2, bits) = if flat2 {
            (6i32, 1, 4)
        } else if chroma {
            (2, 1, 3)
        } else {
            (3, 0, 3)
        };
        for i in -n..n {
            let mut sum = 0;
            for j in -n..=n {
                let k = (i + j).clamp(-n - 1, n);
                sum += s[(7 + k) as usize] * if j.abs() <= n2 { 2 } else { 1 };
            }
            out[(7 + i) as usize] = ((sum + (1 << (bits - 1))) >> bits) as u16;
        }
    } else {
        let hev = (s[5] - s[6]).abs() > threshold || (s[8] - s[7]).abs() > threshold;
        let mid = 1 << (depth - 1);
        let clamp = |v: i32| v.clamp(-mid, mid - 1);
        let filter = clamp(if hev { clamp(s[5] - s[8]) } else { 0 } + 3 * (s[7] - s[6]));
        let f1 = clamp(filter + 4) >> 3;
        let f2 = clamp(filter + 3) >> 3;
        out[7] = (clamp(s[7] - mid - f1) + mid) as u16;
        out[6] = (clamp(s[6] - mid + f2) + mid) as u16;
        if !hev {
            let f = (f1 + 1) >> 1;
            out[8] = (clamp(s[8] - mid - f) + mid) as u16;
            out[5] = (clamp(s[5] - mid + f) + mid) as u16;
        }
    }
    out
}
#[inline]
fn direction(p: &Plane, x: usize, y: usize, depth: u8) -> (usize, i64) {
    let mut partial = [[0i64; 15]; 8];
    for i in 0..8 {
        for j in 0..8 {
            let v = i64::from(p.samples[(y + i) * p.width + x + j] >> (depth - 8)) - 128;
            for (d, k) in [
                i + j,
                i + j / 2,
                i,
                3 + i - j / 2,
                7 + i - j,
                3 + j - i / 2,
                j,
                i / 2 + j,
            ]
            .into_iter()
            .enumerate()
            {
                partial[d][k] += v;
            }
        }
    }
    let div = [0, 840, 420, 280, 210, 168, 140, 120, 105];
    let mut cost = [0i64; 8];
    for d in [2, 6] {
        cost[d] = partial[d][..8].iter().map(|v| v * v).sum::<i64>() * 105;
    }
    for d in [0, 4] {
        for i in 0..7 {
            cost[d] += (partial[d][i] * partial[d][i] + partial[d][14 - i] * partial[d][14 - i])
                * div[i + 1];
        }
        cost[d] += partial[d][7] * partial[d][7] * 105;
    }
    for d in [1, 3, 5, 7] {
        cost[d] = partial[d][3..8].iter().map(|v| v * v).sum::<i64>() * 105;
        for j in 0..3 {
            cost[d] += (partial[d][j] * partial[d][j] + partial[d][10 - j] * partial[d][10 - j])
                * div[2 * j + 2];
        }
    }
    let mut best = 0;
    for d in 1..8 {
        if cost[d] > cost[best] {
            best = d;
        }
    }
    (best, (cost[best] - cost[(best + 4) & 7]) >> 10)
}
const DIRECTIONS: [[(i32, i32); 2]; 8] = [
    [(-1, 1), (-2, 2)],
    [(0, 1), (-1, 2)],
    [(0, 1), (0, 2)],
    [(0, 1), (1, 2)],
    [(1, 1), (2, 2)],
    [(1, 0), (2, 1)],
    [(1, 0), (2, 0)],
    [(1, 0), (2, -1)],
];
/// One CDEF neighbour: its plane offset, its delta for bounds tests, and the
/// fixed threshold, right shift and weight applied to the sample difference.
#[derive(Clone, Copy, Default)]
struct Tap {
    offset: isize,
    dx: i32,
    dy: i32,
    threshold: i32,
    shift: u32,
    weight: i32,
}

#[inline(always)]
fn tap(dy: i32, dx: i32, sign: i32, width: usize, threshold: i32, shift: u32, weight: i32) -> Tap {
    let (dy, dx) = (dy * sign, dx * sign);
    Tap {
        offset: dy as isize * width as isize + dx as isize,
        dx,
        dy,
        threshold,
        shift,
        weight,
    }
}

#[inline(always)]
fn neighbour<const CHECK: bool>(
    src: &[u16],
    base: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    tap: &Tap,
    current: i32,
) -> i32 {
    if CHECK
        && (x as i32 + tap.dx < 0
            || y as i32 + tap.dy < 0
            || x as i32 + tap.dx >= width as i32
            || y as i32 + tap.dy >= height as i32)
    {
        return current;
    }
    i32::from(src[(base as isize + tap.offset) as usize])
}

/// Filters one `size` by `size` run of samples. `CHECK` selects the edge variant,
/// where a neighbour outside the plane contributes nothing.
fn block<const CHECK: bool>(
    source: &Plane,
    target: &mut [u16],
    x0: usize,
    y0: usize,
    size: usize,
    taps: &[Tap],
) {
    let (width, height) = (source.width, source.height);
    let src = &source.samples;
    for y in y0..y0 + size {
        for x in x0..x0 + size {
            let base = y * width + x;
            let current = i32::from(src[base]);
            let (mut lo, mut hi, mut sum) = (current, current, 0);
            for tap in taps {
                let value = neighbour::<CHECK>(src, base, x, y, width, height, tap, current);
                let difference = (value - current).abs();
                let filtered = difference.min((tap.threshold - (difference >> tap.shift)).max(0));
                sum += tap.weight * if value < current { -filtered } else { filtered };
                lo = lo.min(value);
                hi = hi.max(value);
            }
            target[base] = (current + ((8 + sum - i32::from(sum < 0)) >> 4)).clamp(lo, hi) as u16;
        }
    }
}

pub(crate) fn cdef(
    image: &mut Picture,
    params: &Cdef,
    indexes: &[i8],
    skip: &[bool],
    monochrome: bool,
) {
    if params.strengths == [[0; 4]; 8] {
        return;
    }
    let input = image.clone();
    let cols = image.planes[0].width / 4;
    let rows = image.planes[0].height / 4;
    for r in (0..rows).step_by(2) {
        for col in (0..cols).step_by(2) {
            let idx = indexes[(r / 16) * cols.div_ceil(16) + col / 16];
            if idx < 0
                || [
                    r * cols + col,
                    r * cols + col + 1,
                    (r + 1) * cols + col,
                    (r + 1) * cols + col + 1,
                ]
                .iter()
                .all(|i| skip[*i])
            {
                continue;
            }
            let (dir, variance) = direction(&input.planes[0], col * 4, r * 4, image.depth);
            let strength = params.strengths[idx as usize];
            let shift = image.depth - 8;
            let var_strength = if variance >> 6 != 0 {
                (variance >> 6).ilog2().min(12)
            } else {
                0
            };
            for p in 0..if monochrome { 1 } else { 3 } {
                let sub = usize::from(p > 0);
                let mut pri = i32::from(strength[if p == 0 { 0 } else { 2 }]) << shift;
                let sec = i32::from(strength[if p == 0 { 1 } else { 3 }]) << shift;
                let direction = if pri == 0 { 0 } else { dir };
                if p == 0 {
                    pri = if variance == 0 {
                        0
                    } else {
                        (pri * (4 + var_strength as i32) + 8) >> 4
                    };
                }
                let damping = i32::from(params.damping) + i32::from(shift) - i32::from(p > 0);
                let source = &input.planes[p];
                let target = &mut image.planes[p];
                let size = 8 >> sub;
                let pri_taps = if (pri >> shift) & 1 == 0 {
                    [4, 2]
                } else {
                    [3, 3]
                };
                // Four primary taps along the detected direction, then eight secondary
                // taps two steps further round. A zero threshold contributes nothing,
                // so those taps are left out and a plane with none stays untouched.
                let mut taps = [Tap::default(); 12];
                let mut count = 0;
                if pri != 0 {
                    let threshold_shift = (damping - pri.ilog2() as i32).max(0) as u32;
                    for k in 0..2 {
                        let (dy, dx) = DIRECTIONS[direction][k];
                        for sign in [-1, 1] {
                            taps[count] = tap(
                                dy,
                                dx,
                                sign,
                                source.width,
                                pri,
                                threshold_shift,
                                pri_taps[k],
                            );
                            count += 1;
                        }
                    }
                }
                if sec != 0 {
                    let threshold_shift = (damping - sec.ilog2() as i32).max(0) as u32;
                    for k in 0..2 {
                        for sign in [-1, 1] {
                            for offset in [2, 6] {
                                let (dy, dx) = DIRECTIONS[(direction + offset) & 7][k];
                                taps[count] = tap(
                                    dy,
                                    dx,
                                    sign,
                                    source.width,
                                    sec,
                                    threshold_shift,
                                    [2, 1][k],
                                );
                                count += 1;
                            }
                        }
                    }
                }
                if count == 0 {
                    continue;
                }
                let taps = &taps[..count];
                let (x0, y0) = ((col * 4) >> sub, (r * 4) >> sub);
                if x0 >= 2
                    && y0 >= 2
                    && x0 + size + 1 < source.width
                    && y0 + size + 1 < source.height
                {
                    block::<false>(source, &mut target.samples, x0, y0, size, taps);
                } else {
                    block::<true>(source, &mut target.samples, x0, y0, size, taps);
                }
            }
        }
    }
}
