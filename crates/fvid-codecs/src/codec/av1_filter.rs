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
/// One CDEF neighbour tap: the position relative to the filtered pixel plus the
/// fixed threshold, right shift and weight for the sample difference.
fn tap(dy: i32, dx: i32, sign: i32, threshold: i32, shift: i32, weight: i32) -> fvid_cpu::CdefTap {
    fvid_cpu::CdefTap {
        dx: dx * sign,
        dy: dy * sign,
        threshold: threshold as i16,
        shift: shift as i16,
        weight: weight as i16,
    }
}

/// Blocks whose tap window leaves the plane. A neighbour outside contributes nothing.
/// `strip` holds unmodified rows mapped so row `2` corresponds to plane row `y0`.
fn block_edge(
    strip: &[u16],
    width: usize,
    height: usize,
    target: &mut [u16],
    x0: usize,
    y0: usize,
    size: [usize; 2],
    taps: &[fvid_cpu::CdefTap],
) {
    for y in y0..y0 + size[1] {
        let strip_y = 2 + (y - y0);
        for x in x0..x0 + size[0] {
            let current = i32::from(strip[strip_y * width + x]);
            let (mut lo, mut hi, mut sum) = (current, current, 0);
            for tap in taps {
                let (nx, ny) = (x as i32 + tap.dx, y as i32 + tap.dy);
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let sy = (ny - y0 as i32 + 2) as usize;
                let value = i32::from(strip[sy * width + nx as usize]);
                let difference = (value - current).abs();
                let filtered = difference
                    .min((i32::from(tap.threshold) - (difference >> i32::from(tap.shift))).max(0));
                sum += i32::from(tap.weight) * if value < current { -filtered } else { filtered };
                lo = lo.min(value);
                hi = hi.max(value);
            }
            target[y * width + x] =
                (current + ((8 + sum - i32::from(sum < 0)) >> 4)).clamp(lo, hi) as u16;
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
    let has_chroma_strength =
        !monochrome && params.strengths.iter().any(|s| s[2] != 0 || s[3] != 0);
    let plane_count = if has_chroma_strength { 3 } else { 1 };
    let sub = image.subsampling.map(usize::from);
    // Keep only a rolling `(bh + 4)`-row unmodified window per active plane instead
    // of cloning all three full planes on every frame.
    let mut strips: [Plane; 3] = std::array::from_fn(|p| {
        if p >= plane_count {
            return Plane {
                width: 0,
                height: 0,
                samples: Vec::new(),
            };
        }
        let sub_y = if p == 0 { 0 } else { sub[1] };
        let bh = 8 >> sub_y;
        let width = image.planes[p].width;
        Plane {
            width,
            height: bh + 4,
            samples: vec![0u16; width * (bh + 4)],
        }
    });
    let cols = image.planes[0].width / 4;
    let rows = image.planes[0].height / 4;
    for r in (0..rows).step_by(2) {
        for p in 0..plane_count {
            let sub_y = if p == 0 { 0 } else { sub[1] };
            let bh = 8 >> sub_y;
            let y0 = (r * 4) >> sub_y;
            let w = image.planes[p].width;
            let h = image.planes[p].height;
            let strip = &mut strips[p].samples;
            if r == 0 {
                let copy_rows = (bh + 2).min(h);
                strip[2 * w..(2 + copy_rows) * w]
                    .copy_from_slice(&image.planes[p].samples[..copy_rows * w]);
            } else {
                // Shift the 4 unmodified boundary rows `(y0 - 2 .. y0 + 2)` from the
                // previous strip down to slots `0..4`, then append `(y0 + 2 .. y0 + bh + 2)`.
                strip.copy_within(bh * w..(bh + 4) * w, 0);
                let src_start = (y0 + 2).min(h);
                let src_end = (y0 + bh + 2).min(h);
                if src_end > src_start {
                    let dst_off = 4 * w;
                    let len = (src_end - src_start) * w;
                    strip[dst_off..dst_off + len]
                        .copy_from_slice(&image.planes[p].samples[src_start * w..src_end * w]);
                }
            }
        }
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
            let (dir, variance) = direction(&strips[0], col * 4, 2, image.depth);
            let strength = params.strengths[idx as usize];
            let shift = image.depth - 8;
            let var_strength = if variance >> 6 != 0 {
                (variance >> 6).ilog2().min(12)
            } else {
                0
            };
            for p in 0..plane_count {
                let [sub_x, sub_y] = if p == 0 { [0; 2] } else { sub };
                let mut pri = i32::from(strength[if p == 0 { 0 } else { 2 }]) << shift;
                let sec = i32::from(strength[if p == 0 { 1 } else { 3 }]) << shift;
                let direction = if pri == 0 {
                    0
                } else if p > 0 && sub_x != sub_y {
                    if sub_x == 1 {
                        [7, 0, 2, 4, 5, 6, 6, 6][dir]
                    } else {
                        [1, 2, 2, 2, 3, 4, 6, 0][dir]
                    }
                } else {
                    dir
                };
                if p == 0 {
                    pri = if variance == 0 {
                        0
                    } else {
                        (pri * (4 + var_strength as i32) + 8) >> 4
                    };
                }
                let damping = i32::from(params.damping) + i32::from(shift) - i32::from(p > 0);
                let strip = &strips[p];
                let target = &mut image.planes[p];
                let (width, height) = (target.width, target.height);
                let size = [8 >> sub_x, 8 >> sub_y];
                let pri_taps = if (pri >> shift) & 1 == 0 {
                    [4, 2]
                } else {
                    [3, 3]
                };
                // Four primary taps along the detected direction, then eight secondary
                // taps two steps further round. A zero threshold contributes nothing,
                // so those taps are left out and a plane with none stays untouched.
                let mut taps = [fvid_cpu::CdefTap::default(); 12];
                let mut count = 0;
                if pri != 0 {
                    let threshold_shift = (damping - pri.ilog2() as i32).max(0);
                    for k in 0..2 {
                        let (dy, dx) = DIRECTIONS[direction][k];
                        for sign in [-1, 1] {
                            taps[count] = tap(dy, dx, sign, pri, threshold_shift, pri_taps[k]);
                            count += 1;
                        }
                    }
                }
                if sec != 0 {
                    let threshold_shift = (damping - sec.ilog2() as i32).max(0);
                    for k in 0..2 {
                        for sign in [-1, 1] {
                            for offset in [2, 6] {
                                let (dy, dx) = DIRECTIONS[(direction + offset) & 7][k];
                                taps[count] = tap(dy, dx, sign, sec, threshold_shift, [2, 1][k]);
                                count += 1;
                            }
                        }
                    }
                }
                if count == 0 {
                    continue;
                }
                image.cdef_filtered_blocks[p] += 1;
                let taps = &taps[..count];
                let (x0, y0) = ((col * 4) >> sub_x, (r * 4) >> sub_y);
                if x0 >= 2
                    && y0 >= 2
                    && x0 + size[0] + 1 < width
                    && y0 + size[1] + 1 < height
                {
                    let dst_window =
                        &mut target.samples[(y0 - 2) * width..(y0 + size[1] + 2) * width];
                    fvid_cpu::av1_cdef_block(
                        &strip.samples,
                        dst_window,
                        width,
                        x0,
                        2,
                        size[0],
                        size[1],
                        taps,
                    );
                } else {
                    block_edge(
                        &strip.samples,
                        width,
                        height,
                        &mut target.samples,
                        x0,
                        y0,
                        size,
                        taps,
                    );
                }
            }
        }
    }
}
