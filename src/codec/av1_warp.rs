//! Integer local affine estimation and separable AV1 warp filtering.
use super::{
    av1_picture::Picture,
    av1_tables::{DIV_LUT, WARPED_FILTERS},
};
use crate::{Result, invalid};
fn round(v: i64, n: u32) -> i64 {
    if n == 0 { v } else { (v + (1 << (n - 1))) >> n }
}
fn signed(v: i64, n: u32) -> i64 {
    v.signum() * round(v.abs(), n)
}
fn divisor(d: i64) -> (u32, i64) {
    let n = d.unsigned_abs().ilog2();
    let e = d.abs() - (1 << n);
    let f = if n > 8 { round(e, n - 8) } else { e << (8 - n) };
    (n + 14, i64::from(DIV_LUT[f as usize]) * d.signum())
}
pub(super) fn estimate(
    samples: &[[i64; 4]],
    origin: [usize; 2],
    size: [usize; 2],
    mv: [i32; 2],
) -> Option<[i64; 6]> {
    let midx = (origin[0] * 4 + size[0] * 2 - 1) as i64;
    let midy = (origin[1] * 4 + size[1] * 2 - 1) as i64;
    let mut a = [0i64; 3];
    let mut bx = [0; 2];
    let mut by = [0; 2];
    let product = |a: i64, b: i64| (a * b >> 2) + a + b;
    for cand in samples {
        let sy = cand[0] - midy * 8;
        let sx = cand[1] - midx * 8;
        let dy = cand[2] - midy * 8 - i64::from(mv[0]);
        let dx = cand[3] - midx * 8 - i64::from(mv[1]);
        if (sx - dx).abs() < 256 && (sy - dy).abs() < 256 {
            a[0] += product(sx, sx) + 8;
            a[1] += product(sx, sy) + 4;
            a[2] += product(sy, sy) + 8;
            bx[0] += product(sx, dx) + 8;
            bx[1] += product(sy, dx) + 4;
            by[0] += product(sx, dy) + 4;
            by[1] += product(sy, dy) + 8;
        }
    }
    let det = a[0] * a[2] - a[1] * a[1];
    if det == 0 {
        return None;
    }
    let (mut shift, mut factor) = divisor(det);
    if shift < 16 {
        factor <<= 16 - shift;
        shift = 0;
    } else {
        shift -= 16;
    }
    let diag = |v| signed(v * factor, shift).clamp(65536 - 8191, 65536 + 8191);
    let non = |v| signed(v * factor, shift).clamp(-8191, 8191);
    let mut params = [
        0,
        0,
        diag(a[2] * bx[0] - a[1] * bx[1]),
        non(-a[1] * bx[0] + a[0] * bx[1]),
        non(a[2] * by[0] - a[1] * by[1]),
        diag(-a[1] * by[0] + a[0] * by[1]),
    ];
    params[0] = (i64::from(mv[1]) * 8192 - midx * (params[2] - 65536) - midy * params[3])
        .clamp(-(1 << 23), (1 << 23) - 1);
    params[1] = (i64::from(mv[0]) * 8192 - midx * params[4] - midy * (params[5] - 65536))
        .clamp(-(1 << 23), (1 << 23) - 1);
    shear(params).map(|_| params)
}
fn shear(p: [i64; 6]) -> Option<[i64; 4]> {
    if p[2] == 0 {
        return None;
    }
    let (shift, factor) = divisor(p[2]);
    let a = (p[2] - 65536).clamp(-32768, 32767);
    let b = p[3].clamp(-32768, 32767);
    let g = signed((p[4] << 16) * factor, shift).clamp(-32768, 32767);
    let d = (p[5] - signed(p[3] * p[4] * factor, shift) - 65536).clamp(-32768, 32767);
    let [a, b, g, d] = [a, b, g, d].map(|v| signed(v, 6) << 6);
    (4 * a.abs() + 7 * b.abs() < 65536 && 4 * g.abs() + 4 * d.abs() < 65536).then_some([a, b, g, d])
}
pub(super) fn valid(params: [i64; 6]) -> bool {
    shear(params).is_some()
}
pub(super) fn predict(
    reference: &Picture,
    plane: usize,
    origin: [usize; 2],
    size: [usize; 2],
    params: [i64; 6],
    compound: bool,
) -> Result<Vec<i32>> {
    let [a, b, g, d] = shear(params).ok_or_else(|| invalid("invalid AV1 warp shear"))?;
    let [x, y] = origin;
    let [w, h] = size;
    let [sub_x, sub_y] = if plane == 0 {
        [0; 2]
    } else {
        reference.subsampling.map(usize::from)
    };
    let src = &reference.planes[plane];
    let maxx = (reference.size[0] as usize).div_ceil(1 << sub_x) as i64 - 1;
    let maxy = (reference.size[1] as usize).div_ceil(1 << sub_y) as i64 - 1;
    let r0 = if reference.depth == 12 { 5 } else { 3 };
    let r1 = if compound { 7 } else { 14 - r0 };
    let mut out = vec![0; w * h];
    for yy in (0..h).step_by(8) {
        for xx in (0..w).step_by(8) {
            let sx = ((x + xx + 4) << sub_x) as i64;
            let sy = ((y + yy + 4) << sub_y) as i64;
            let dx = (params[2] * sx + params[3] * sy + params[0]) >> sub_x;
            let dy = (params[4] * sx + params[5] * sy + params[1]) >> sub_y;
            let ix = dx >> 16;
            let iy = dy >> 16;
            let fx = dx & 65535;
            let fy = dy & 65535;
            let mut temp = [[0i64; 8]; 15];
            for r in -7i64..8 {
                for col in -4i64..4 {
                    let offset = round(fx + a * col + b * r, 10) + 64;
                    let filter = WARPED_FILTERS
                        .get(offset as usize)
                        .ok_or_else(|| invalid("AV1 warp filter outside table"))?;
                    let sy = (iy + r).clamp(0, maxy) as usize;
                    let mut sum = 0;
                    for (t, k) in filter.iter().enumerate() {
                        let sx = (ix + col - 3 + t as i64).clamp(0, maxx) as usize;
                        sum += i64::from(*k) * i64::from(src.samples[sy * src.width + sx]);
                    }
                    temp[(r + 7) as usize][(col + 4) as usize] = round(sum, r0);
                }
            }
            for r in -4i64..4.min((h - yy) as i64 - 4) {
                for col in -4i64..4.min((w - xx) as i64 - 4) {
                    let offset = round(fy + g * col + d * r, 10) + 64;
                    let filter = WARPED_FILTERS
                        .get(offset as usize)
                        .ok_or_else(|| invalid("AV1 warp filter outside table"))?;
                    let sum = filter
                        .iter()
                        .enumerate()
                        .map(|(t, k)| {
                            i64::from(*k) * temp[(r + t as i64 + 4) as usize][(col + 4) as usize]
                        })
                        .sum::<i64>();
                    out[(yy + (r + 4) as usize) * w + xx + (col + 4) as usize] =
                        round(sum, r1) as i32;
                }
            }
        }
    }
    Ok(out)
}
