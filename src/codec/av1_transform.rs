//! AV1 normative integer inverse transforms (section 7.13).
use crate::{Result, invalid};
const COS: [i64; 65] = [
    4096, 4095, 4091, 4085, 4076, 4065, 4052, 4036, 4017, 3996, 3973, 3948, 3920, 3889, 3857, 3822,
    3784, 3745, 3703, 3659, 3612, 3564, 3513, 3461, 3406, 3349, 3290, 3229, 3166, 3102, 3035, 2967,
    2896, 2824, 2751, 2675, 2598, 2520, 2440, 2359, 2276, 2191, 2106, 2019, 1931, 1842, 1751, 1660,
    1567, 1474, 1380, 1285, 1189, 1092, 995, 897, 799, 700, 601, 501, 401, 301, 201, 101, 0,
];
fn cos(a: i32) -> i64 {
    let a = (a & 255) as usize;
    match a {
        0..=64 => COS[a],
        65..=128 => -COS[128 - a],
        129..=192 => -COS[a - 128],
        _ => COS[256 - a],
    }
}
fn round(x: i64, n: u32) -> i64 {
    if n == 0 { x } else { (x + (1 << (n - 1))) >> n }
}
fn clip(x: i64, r: u8) -> i64 {
    x.clamp(-(1 << (r - 1)), (1 << (r - 1)) - 1)
}
fn brev(n: u32, x: usize) -> usize {
    x.reverse_bits() >> (usize::BITS - n)
}
fn b(t: &mut [i64], a: usize, c: usize, angle: i32, flip: bool) {
    let x = round(t[a] * cos(angle) - t[c] * cos(angle - 64), 12);
    let y = round(t[a] * cos(angle - 64) + t[c] * cos(angle), 12);
    t[a] = if flip { y } else { x };
    t[c] = if flip { x } else { y };
}
fn h(t: &mut [i64], a: usize, b: usize, flip: bool, r: u8) {
    let (a, b) = if flip { (b, a) } else { (a, b) };
    let (x, y) = (t[a], t[b]);
    t[a] = clip(x + y, r);
    t[b] = clip(x - y, r);
}
fn dct(t: &mut [i64], r: u8) {
    let n = t.len().ilog2();
    let mut copy = [0i64; 64];
    copy[..t.len()].copy_from_slice(t);
    for i in 0..t.len() {
        t[i] = copy[brev(n, i)];
    }
    if n == 6 {
        for i in 0..16 {
            b(t, 32 + i, 63 - i, 63 - 4 * brev(4, i) as i32, false);
        }
    }
    if n >= 5 {
        for i in 0..8 {
            b(t, 16 + i, 31 - i, 6 + ((brev(3, 7 - i) as i32) << 3), false);
        }
    }
    if n == 6 {
        for i in 0..16 {
            h(t, 32 + i * 2, 33 + i * 2, i & 1 != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..4 {
            b(t, 8 + i, 15 - i, 12 + ((brev(2, 3 - i) as i32) << 4), false);
        }
    }
    if n >= 5 {
        for i in 0..8 {
            h(t, 16 + 2 * i, 17 + 2 * i, i & 1 != 0, r);
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..2 {
                b(
                    t,
                    62 - i * 4 - j,
                    33 + i * 4 + j,
                    60 - 16 * brev(2, i) as i32 + 64 * j as i32,
                    true,
                );
            }
        }
    }
    if n >= 3 {
        for i in 0..2 {
            b(t, 4 + i, 7 - i, 56 - 32 * i as i32, false);
        }
    }
    if n >= 4 {
        for i in 0..4 {
            h(t, 8 + 2 * i, 9 + 2 * i, i & 1 != 0, r);
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    30 - 4 * i - j,
                    17 + 4 * i + j,
                    24 + 64 * j as i32 + 32 * (1 - i) as i32,
                    true,
                );
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            for j in 0..2 {
                h(t, 32 + i * 4 + j, 35 + i * 4 - j, i & 1 != 0, r);
            }
        }
    }
    for i in 0..2 {
        b(t, 2 * i, 2 * i + 1, 32 + 16 * i as i32, i == 0);
    }
    if n >= 3 {
        for i in 0..2 {
            h(t, 4 + 2 * i, 5 + 2 * i, i != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 14 - i, 9 + i, 48 + 64 * i as i32, true);
        }
    }
    if n >= 5 {
        for i in 0..4 {
            for j in 0..2 {
                h(t, 16 + 4 * i + j, 19 + 4 * i - j, i & 1 != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..2 {
            for j in 0..4 {
                b(
                    t,
                    61 - i * 8 - j,
                    34 + i * 8 + j,
                    56 - i as i32 * 32 + (j as i32 >> 1) * 64,
                    true,
                );
            }
        }
    }
    for i in 0..2 {
        h(t, i, 3 - i, false, r);
    }
    if n >= 3 {
        b(t, 6, 5, 32, true);
    }
    if n >= 4 {
        for i in 0..2 {
            for j in 0..2 {
                h(t, 8 + 4 * i + j, 11 + 4 * i - j, i != 0, r);
            }
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 29 - i, 18 + i, 48 + (i as i32 >> 1) * 64, true);
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..4 {
                h(t, 32 + 8 * i + j, 39 + 8 * i - j, i & 1 != 0, r);
            }
        }
    }
    if n >= 3 {
        for i in 0..4 {
            h(t, i, 7 - i, false, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 13 - i, 10 + i, 32, true);
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..4 {
                h(t, 16 + i * 8 + j, 23 + i * 8 - j, i != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 59 - i, 36 + i, if i < 4 { 48 } else { 112 }, true);
        }
    }
    if n >= 4 {
        for i in 0..8 {
            h(t, i, 15 - i, false, r);
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 27 - i, 20 + i, 32, true);
        }
    }
    if n == 6 {
        for i in 0..8 {
            h(t, 32 + i, 47 - i, false, r);
            h(t, 48 + i, 63 - i, true, r);
        }
    }
    if n >= 5 {
        for i in 0..16 {
            h(t, i, 31 - i, false, r);
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 55 - i, 40 + i, 32, true);
        }
        for i in 0..32 {
            h(t, i, 63 - i, false, r);
        }
    }
}
fn adst(t: &mut [i64], r: u8) {
    if t.len() == 4 {
        let a = 1321 * t[0] + 3803 * t[2] + 2482 * t[3];
        let b = 2482 * t[0] - 1321 * t[2] - 3803 * t[3];
        let c = 3344 * t[1];
        let d = 3344 * (t[0] - t[2] + t[3]);
        t[0] = round(a + c, 12);
        t[1] = round(b + c, 12);
        t[2] = round(d, 12);
        t[3] = round(a + b - c, 12);
        return;
    }
    let len = t.len();
    let mut copy = [0i64; 16];
    copy[..len].copy_from_slice(t);
    for i in 0..len {
        t[i] = copy[if i & 1 != 0 { i - 1 } else { len - i - 1 }];
    }
    if len == 8 {
        for i in 0..4 {
            b(t, 2 * i, 2 * i + 1, 60 - 16 * i as i32, true);
        }
        for i in 0..4 {
            h(t, i, 4 + i, false, r);
        }
        for i in 0..2 {
            b(t, 4 + 3 * i, 5 + i, 48 - 32 * i as i32, true);
        }
    } else {
        for i in 0..8 {
            b(t, 2 * i, 2 * i + 1, 62 - 8 * i as i32, true);
        }
        for i in 0..8 {
            h(t, i, 8 + i, false, r);
        }
        for i in 0..2 {
            b(t, 8 + 2 * i, 9 + 2 * i, 56 - 32 * i as i32, true);
            b(t, 13 + 2 * i, 12 + 2 * i, 8 + 32 * i as i32, true);
        }
        for j in 0..2 {
            for i in 0..4 {
                h(t, 8 * j + i, 4 + 8 * j + i, false, r);
            }
        }
        for j in 0..2 {
            for i in 0..2 {
                b(
                    t,
                    4 + 8 * j + 3 * i,
                    5 + 8 * j + i,
                    48 - 32 * i as i32,
                    true,
                );
            }
        }
    }
    for j in 0..len / 4 {
        for i in 0..2 {
            h(t, 4 * j + i, 2 + 4 * j + i, false, r);
        }
    }
    for i in 0..len / 4 {
        b(t, 2 + 4 * i, 3 + 4 * i, 32, true);
    }
    let mut copy = [0i64; 16];
    copy[..len].copy_from_slice(t);
    let n = len.ilog2();
    for (i, out) in t.iter_mut().enumerate() {
        let a = (i >> 3) & 1;
        let b = ((i >> 2) ^ (i >> 3)) & 1;
        let c = ((i >> 1) ^ (i >> 2)) & 1;
        let d = (i ^ (i >> 1)) & 1;
        let index = ((d << 3) | (c << 2) | (b << 1) | a) >> (4 - n);
        *out = if i & 1 != 0 {
            -copy[index]
        } else {
            copy[index]
        };
    }
}
fn transform(t: &mut [i64], kind: u8, r: u8) -> Result<()> {
    let length = t.len();
    match kind {
        0 => dct(t, r),
        1 | 2 => {
            if t.len() > 16 {
                return Err(invalid("AV1 ADST exceeds 16 samples"));
            }
            adst(t, r);
        }
        3 => {
            for v in t.iter_mut() {
                *v = match length {
                    4 => round(*v * 5793, 12),
                    8 => *v * 2,
                    16 => round(*v * 11586, 12),
                    32 => *v * 4,
                    _ => return Err(invalid("invalid AV1 identity transform")),
                };
            }
        }
        _ => return Err(invalid("invalid AV1 transform kind")),
    }
    Ok(())
}
/// AV1 transform type numbers 0..15, raster-order dequantized coefficients.
pub fn inverse(
    coefficients: &[i32],
    size: [usize; 2],
    depth: u8,
    kind: u8,
    scratch: &mut Vec<i64>,
    out: &mut Vec<i32>,
) -> Result<()> {
    let [w, h] = size;
    if ![4, 8, 16, 32, 64].contains(&w)
        || ![4, 8, 16, 32, 64].contains(&h)
        || w > h * 4
        || h > w * 4
        || coefficients.len() != w * h
        || ![8, 10, 12].contains(&depth)
        || kind > 15
    {
        return Err(invalid("invalid AV1 inverse transform configuration"));
    }
    let (vertical, horizontal) = [
        (0, 0),
        (1, 0),
        (0, 1),
        (1, 1),
        (2, 0),
        (0, 2),
        (2, 2),
        (1, 2),
        (2, 1),
        (3, 3),
        (0, 3),
        (3, 0),
        (1, 3),
        (3, 1),
        (2, 3),
        (3, 2),
    ][kind as usize];
    let row_shift = match (w, h) {
        (4, 4) | (4, 8) | (8, 4) => 0,
        (8, 8)
        | (8, 16)
        | (16, 8)
        | (16, 32)
        | (32, 16)
        | (4, 16)
        | (16, 4)
        | (32, 64)
        | (64, 32) => 1,
        _ => 2,
    };
    scratch.resize(w * h + h, 0);
    let (data, col) = scratch.split_at_mut(w * h);
    for (d, c) in data.iter_mut().zip(coefficients) {
        *d = i64::from(*c);
    }
    let col_range = (depth + 6).max(16);
    for row in data.chunks_exact_mut(w) {
        if w.ilog2().abs_diff(h.ilog2()) == 1 {
            for v in row.iter_mut() {
                *v = round(*v * 2896, 12);
            }
        }
        transform(row, horizontal, depth + 8)?;
        for v in row {
            *v = clip(round(*v, row_shift), col_range);
        }
    }
    out.resize(w * h, 0);
    out.fill(0);
    let col = &mut *col;
    for x in 0..w {
        for y in 0..h {
            col[y] = data[y * w + x];
        }
        transform(col, vertical, col_range)?;
        for y in 0..h {
            let xx = if horizontal == 2 { w - 1 - x } else { x };
            let yy = if vertical == 2 { h - 1 - y } else { y };
            out[yy * w + xx] = i32::try_from(round(col[y], 4))
                .map_err(|_| invalid("AV1 inverse transform overflow"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_sizes_types_depths_match_libaom() {
        let shapes = [
            [4, 4],
            [8, 8],
            [16, 16],
            [32, 32],
            [64, 64],
            [4, 8],
            [8, 4],
            [8, 16],
            [16, 8],
            [16, 32],
            [32, 16],
            [32, 64],
            [64, 32],
            [4, 16],
            [16, 4],
            [8, 32],
            [32, 8],
            [16, 64],
            [64, 16],
        ];
        let mut expected = &include_bytes!("../../tests/fixtures/av1/transforms.bin")[..];
        for [w, h] in shapes {
            for kind in 0..16 {
                if (w == 64 || h == 64) && kind != 0 {
                    continue;
                }
                if (w == 32 || h == 32) && kind != 0 && kind != 9 {
                    continue;
                }
                for depth in [8, 10, 12] {
                    for trial in 0..4 {
                        let mut rng = 0x73915926u32 + trial;
                        let mut coeff = vec![0; w * h];
                        for y in 0..h {
                            for x in 0..w {
                                rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                                if x < 32 && y < 32 {
                                    coeff[y * w + x] = ((rng >> 16) % 1025) as i32 - 512;
                                }
                            }
                        }
                        let mut scratch = Vec::new();
                        let mut out = Vec::new();
                        inverse(&coeff, [w, h], depth, kind, &mut scratch, &mut out).unwrap();
                        for (i, v) in out.iter().enumerate() {
                            let pixel = (v + (1 << (depth - 1))).clamp(0, (1 << depth) - 1) as u16;
                            let oracle = u16::from_le_bytes([expected[0], expected[1]]);
                            expected = &expected[2..];
                            assert_eq!(
                                pixel, oracle,
                                "{w}x{h} kind={kind} depth={depth} trial={trial} pixel={i}"
                            );
                        }
                    }
                }
            }
        }
        assert!(expected.is_empty());
    }
}
