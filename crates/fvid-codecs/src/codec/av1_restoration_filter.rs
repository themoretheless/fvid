//! AV1 7.17 restoration filtering, using separate deblocked/CDEF sources.
use super::super::Plane;
use super::Unit;
const SGR: [[i64; 4]; 16] = [
    [2, 12, 1, 4],
    [2, 15, 1, 6],
    [2, 18, 1, 8],
    [2, 21, 1, 9],
    [2, 24, 1, 10],
    [2, 29, 1, 11],
    [2, 36, 1, 12],
    [2, 45, 1, 13],
    [2, 56, 1, 14],
    [2, 68, 1, 15],
    [0, 0, 1, 5],
    [0, 0, 1, 8],
    [0, 0, 1, 11],
    [0, 0, 1, 14],
    [2, 30, 0, 0],
    [2, 75, 0, 0],
];
const SGR_A2_LUT: [u16; 255] = {
    let mut lut = [1u16; 255];
    let mut z = 1usize;
    while z < 255 {
        lut[z] = (((z << 8) + z / 2) / (z + 1)) as u16;
        z += 1;
    }
    lut
};
fn round(v: i64, b: u32) -> i64 {
    if b == 0 { v } else { (v + (1 << (b - 1))) >> b }
}
pub(super) struct Source<'a> {
    pub top: [&'a [u16]; 2],
    pub bottom: [&'a [u16]; 2],
    pub cdef_stripe: &'a [u16],
    pub stripe_y0: usize,
    pub width: usize,
    pub height: usize,
    pub start: i64,
    pub end: i64,
    pub depth: u8,
}
impl Source<'_> {
    #[inline(always)]
    fn row_slice(&self, y: i64) -> &[u16] {
        let y = y.clamp(0, self.height as i64 - 1);
        if y < self.start {
            if y <= self.start - 2 {
                self.top[0]
            } else {
                self.top[1]
            }
        } else if y > self.end {
            if y == self.end + 1 {
                self.bottom[0]
            } else {
                self.bottom[1]
            }
        } else {
            let base = (y as usize - self.stripe_y0) * self.width;
            &self.cdef_stripe[base..base + self.width]
        }
    }
    #[inline(always)]
    fn current(&self, x: usize, y: usize) -> i64 {
        i64::from(self.cdef_stripe[(y - self.stripe_y0) * self.width + x])
    }
}
pub(super) fn block(
    source: &Source<'_>,
    output: &mut Plane,
    unit: &Unit,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) {
    match unit {
        Unit::None => {}
        Unit::Wiener(taps) => wiener(source, output, taps, x, y, w, h),
        Unit::Sgr { set, weights } => {
            let parameters = SGR[*set as usize];
            let mut f0_buf = [0i64; 64 * 64];
            let mut f1_buf = [0i64; 64 * 64];
            let has_f0 = box_filter(
                source,
                x,
                y,
                w,
                h,
                parameters[0],
                parameters[1],
                0,
                &mut f0_buf,
            );
            let has_f1 = box_filter(
                source,
                x,
                y,
                w,
                h,
                parameters[2],
                parameters[3],
                1,
                &mut f1_buf,
            );
            let w0 = i64::from(weights[0]);
            let w1 = i64::from(weights[1]);
            let w2 = 128 - w0 - w1;
            let max_sample = (1i64 << source.depth) - 1;
            for row in 0..h {
                for col in 0..w {
                    let at = row * w + col;
                    let u = source.current(x + col, y + row) << 4;
                    let v0 = if has_f0 { f0_buf[at] } else { u };
                    let v1 = if has_f1 { f1_buf[at] } else { u };
                    let v = w1 * u + w0 * v0 + w2 * v1;
                    output.samples[(y + row) * output.width + x + col] =
                        round(v, 11).clamp(0, max_sample) as u16;
                }
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn box_filter(
    s: &Source<'_>,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    r: i64,
    eps: i64,
    pass: usize,
    result: &mut [i64; 64 * 64],
) -> bool {
    if r == 0 {
        return false;
    }
    let r_usize = r as usize;
    let win = 2 * r_usize + 1;
    let stride = w + 2;
    let rows_out = h + 2;
    let pad_w = stride + 2 * r_usize;
    let pad_h = rows_out + 2 * r_usize;
    // w, h <= 64 and r <= 2 -> stride <= 66, rows_out <= 66, pad_w <= 70, pad_h <= 70.
    let mut h_sum = [0i64; 70 * 66];
    let mut h_sq = [0i64; 70 * 66];
    let mut row_v = [0i64; 70];
    let max_x = s.width as i64 - 1;
    for py in 0..pad_h {
        let ry = y as i64 + py as i64 - 1 - r;
        let src_row = s.row_slice(ry);
        for (px, dst) in row_v[..pad_w].iter_mut().enumerate() {
            let rx = (x as i64 + px as i64 - 1 - r).clamp(0, max_x) as usize;
            *dst = i64::from(src_row[rx]);
        }
        let mut s1 = 0i64;
        let mut s2 = 0i64;
        for &v in &row_v[..win] {
            s1 += v;
            s2 += v * v;
        }
        let row_base = py * stride;
        h_sum[row_base] = s1;
        h_sq[row_base] = s2;
        for col_idx in 1..stride {
            let add_v = row_v[col_idx + win - 1];
            let sub_v = row_v[col_idx - 1];
            s1 += add_v - sub_v;
            s2 += add_v * add_v - sub_v * sub_v;
            h_sum[row_base + col_idx] = s1;
            h_sq[row_base + col_idx] = s2;
        }
    }
    let mut a = [0i64; 66 * 66];
    let mut b = [0i64; 66 * 66];
    let n = (2 * r + 1) * (2 * r + 1);
    let n2e = n * n * eps;
    let scale = ((1 << 20) + n2e / 2) / n2e;
    let reciprocal = ((1 << 12) + n / 2) / n;
    let d_shift = u32::from(s.depth - 8);
    for row_idx in 0..rows_out {
        for col_idx in 0..stride {
            let mut sum = 0i64;
            let mut square = 0i64;
            for ky in 0..win {
                let idx = (row_idx + ky) * stride + col_idx;
                sum += h_sum[idx];
                square += h_sq[idx];
            }
            let variance = (round(square, 2 * d_shift) * n - round(sum, d_shift).pow(2)).max(0);
            let z = round(variance * scale, 20);
            let a2 = if z >= 255 {
                256
            } else {
                i64::from(SGR_A2_LUT[z.max(0) as usize])
            };
            let at = row_idx * stride + col_idx;
            a[at] = a2;
            b[at] = round((256 - a2) * sum * reciprocal, 12);
        }
    }
    for row in 0..h {
        for col in 0..w {
            let mut av = 0;
            let mut bv = 0;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let weight = if pass == 0 {
                        if (row as i64 + dy) & 1 != 0 {
                            if dx == 0 { 6 } else { 5 }
                        } else {
                            0
                        }
                    } else if dx == 0 || dy == 0 {
                        4
                    } else {
                        3
                    };
                    let at =
                        (row as i64 + dy + 1) as usize * stride + (col as i64 + dx + 1) as usize;
                    av += weight * a[at];
                    bv += weight * b[at];
                }
            }
            let shift = if pass == 0 && row & 1 != 0 { 4 } else { 5 };
            result[row * w + col] = round(av * s.current(x + col, y + row) + bv, 8 + shift - 4);
        }
    }
    true
}
fn coefficients(taps: &[i32; 3]) -> [i32; 4] {
    [
        taps[0],
        taps[1],
        taps[2],
        128 - 2 * (taps[0] + taps[1] + taps[2]),
    ]
}
fn wiener(
    s: &Source<'_>,
    output: &mut Plane,
    taps: &[[i32; 3]; 2],
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) {
    let vf = coefficients(&taps[0]);
    let hf = coefficients(&taps[1]);
    let r0 = if s.depth == 12 { 5 } else { 3 };
    let r1 = 14 - r0;
    let offset = 1i32 << (u32::from(s.depth) + 7 - r0 - 1);
    let limit = (1i32 << (u32::from(s.depth) + 1 + 7 - r0)) - 1;
    let min_val = -offset;
    let max_val = limit - offset;
    let mut intermediate = [0i32; 64 * 70];
    let mut padded_row = [0u16; 70];
    let max_x = s.width as i64 - 1;
    let interior_x = x >= 3 && x + w + 3 <= s.width;
    for row in 0..h + 6 {
        let src_row = s.row_slice(y as i64 + row as i64 - 3);
        let inter_row = &mut intermediate[row * w..(row + 1) * w];
        if interior_x {
            fvid_cpu::av1_wiener_h_7tap(
                &src_row[x - 3..x + w + 3],
                hf,
                r0,
                min_val,
                max_val,
                inter_row,
            );
        } else {
            for (px, dst) in padded_row[..w + 6].iter_mut().enumerate() {
                let rx = (x as i64 + px as i64 - 3).clamp(0, max_x) as usize;
                *dst = src_row[rx];
            }
            fvid_cpu::av1_wiener_h_7tap(
                &padded_row[..w + 6],
                hf,
                r0,
                min_val,
                max_val,
                inter_row,
            );
        }
    }
    let max_sample = (1i32 << s.depth) - 1;
    for row in 0..h {
        let r0_s = &intermediate[row * w..(row + 1) * w];
        let r1_s = &intermediate[(row + 1) * w..(row + 2) * w];
        let r2_s = &intermediate[(row + 2) * w..(row + 3) * w];
        let r3_s = &intermediate[(row + 3) * w..(row + 4) * w];
        let r4_s = &intermediate[(row + 4) * w..(row + 5) * w];
        let r5_s = &intermediate[(row + 5) * w..(row + 6) * w];
        let r6_s = &intermediate[(row + 6) * w..(row + 7) * w];
        let out_row = &mut output.samples[(y + row) * output.width + x..][..w];
        fvid_cpu::av1_wiener_v_7tap(
            r0_s, r1_s, r2_s, r3_s, r4_s, r5_s, r6_s, vf, r1, max_sample, out_row,
        );
    }
}
