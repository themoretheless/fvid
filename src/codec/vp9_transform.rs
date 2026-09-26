//! Integer inverse VP9 transforms, specification section 8.7.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    DctDct,
    AdstDct,
    DctAdst,
    AdstAdst,
    Lossless,
}
const COS: [i64; 33] = [
    16384, 16364, 16305, 16207, 16069, 15893, 15679, 15426, 15137, 14811, 14449, 14053, 13623,
    13160, 12665, 12140, 11585, 11003, 10394, 9760, 9102, 8423, 7723, 7005, 6270, 5520, 4756, 3981,
    3196, 2404, 1606, 804, 0,
];
fn cos(angle: i32) -> i64 {
    let a = (angle & 127) as usize;
    match a {
        0..=32 => COS[a],
        33..=64 => -COS[64 - a],
        65..=96 => -COS[a - 64],
        _ => COS[128 - a],
    }
}
fn round(v: i64, bits: u32) -> i64 {
    (v + (1 << (bits - 1))) >> bits
}
fn brev(n: usize, x: usize) -> usize {
    x.reverse_bits() >> (usize::BITS as usize - n)
}
fn b(t: &mut [i64], a: usize, c: usize, angle: i32, flip: bool) {
    let x = round(t[a] * cos(angle) - t[c] * cos(angle - 32), 14);
    let y = round(t[a] * cos(angle - 32) + t[c] * cos(angle), 14);
    t[a] = if flip { y } else { x };
    t[c] = if flip { x } else { y };
}
fn h(t: &mut [i64], a: usize, b: usize, flip: bool) {
    let (a, b) = if flip { (b, a) } else { (a, b) };
    let x = t[a];
    let y = t[b];
    t[a] = x + y;
    t[b] = x - y;
}
fn sb(t: &[i64], s: &mut [i64], a: usize, b: usize, angle: i32) {
    s[a] = t[a] * cos(angle - 32) + t[b] * cos(angle);
    s[b] = t[a] * cos(angle) - t[b] * cos(angle - 32);
}
fn sh(t: &mut [i64], s: &[i64], a: usize, b: usize) {
    t[a] = round(s[a] + s[b], 14);
    t[b] = round(s[a] - s[b], 14);
}
fn dct(t: &mut [i64], n: usize) {
    let n0 = 1 << n;
    let n1 = n0 / 2;
    let n2 = n0 / 4;
    let n3 = n0 / 8;
    if n == 2 {
        b(t, 0, 1, 16, true);
    } else {
        dct(t, n - 1);
    }
    for i in 0..n2 {
        b(t, n1 + i, n0 - 1 - i, 32 - brev(5, n1 + i) as i32, false);
    }
    if n >= 3 {
        for i in 0..n3 {
            for j in 0..2 {
                h(t, n1 + 4 * i + 2 * j, n1 + 1 + 4 * i + 2 * j, j != 0);
            }
        }
    }
    if n == 5 {
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    n0 - n + 3 - n2 * j - 4 * i,
                    n1 + n - 4 + n2 * j + 4 * i,
                    28 - 16 * i as i32 + 56 * j as i32,
                    true,
                );
            }
        }
        for i in 0..2 {
            for j in 0..4 {
                h(t, n1 + n3 * j + i, n1 + n2 - 5 + n3 * j - i, j & 1 != 0);
            }
        }
    }
    if n >= 4 {
        for i in 0..=usize::from(n == 5) {
            for j in 0..2 {
                b(
                    t,
                    n0 - n + 2 - i - n2 * j,
                    n1 + n - 3 + i + n2 * j,
                    24 + 48 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=(2 * n - 7) {
            for j in 0..2 {
                h(t, n1 + n2 * j + i, n1 + n2 - 1 + n2 * j - i, j & 1 != 0);
            }
        }
    }
    if n >= 3 {
        for i in 0..n3 {
            b(t, n0 - n3 - 1 - i, n1 + n3 + i, 16, true);
        }
    }
    for i in 0..n1 {
        h(t, i, n0 - 1 - i, false);
    }
}
fn adst(t: &mut [i64], n: usize) {
    if n == 2 {
        let x0 = 5283 * t[0] + 15212 * t[2] + 9929 * t[3];
        let x1 = 9929 * t[0] - 5283 * t[2] - 15212 * t[3];
        let x2 = 13377 * (t[0] - t[2] + t[3]);
        let x3 = 13377 * t[1];
        t.copy_from_slice(&[
            round(x0 + x3, 14),
            round(x1 + x3, 14),
            round(x2, 14),
            round(x0 + x1 - x3, 14),
        ]);
        return;
    }
    let size = 1 << n;
    let mut copy = [0i64; 32];
    copy[..size].copy_from_slice(&t[..size]);
    for i in 0..size / 2 {
        t[2 * i] = copy[size - 1 - 2 * i];
        t[2 * i + 1] = copy[2 * i];
    }
    let mut s = [0i64; 32];
    if n == 3 {
        for i in 0..4 {
            sb(t, &mut s, 2 * i, 1 + 2 * i, 30 - 8 * i as i32);
        }
        for i in 0..4 {
            sh(t, &s, i, 4 + i);
        }
        for i in 0..2 {
            sb(t, &mut s, 4 + 3 * i, 5 + i, 24 - 16 * i as i32);
        }
        for i in 0..2 {
            sh(t, &s, 4 + i, 6 + i);
        }
        for i in 0..2 {
            h(t, i, 2 + i, false);
        }
        for i in 0..2 {
            b(t, 2 + 4 * i, 3 + 4 * i, 16, true);
        }
    } else {
        for i in 0..8 {
            sb(t, &mut s, 2 * i, 1 + 2 * i, 31 - 4 * i as i32);
        }
        for i in 0..8 {
            sh(t, &s, i, 8 + i);
        }
        for i in 0..4 {
            sb(t, &mut s, 8 + 2 * i, 9 + 2 * i, 28 - 16 * i as i32);
        }
        for i in 0..4 {
            sh(t, &s, 8 + i, 12 + i);
            h(t, i, 4 + i, false);
        }
        for i in 0..2 {
            for j in 0..2 {
                sb(
                    t,
                    &mut s,
                    4 + 8 * i + 3 * j,
                    5 + 8 * i + j,
                    24 - 16 * j as i32,
                );
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                sh(t, &s, 4 + 8 * j + i, 6 + 8 * j + i);
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                h(t, 8 * j + i, 2 + 8 * j + i, false);
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    2 + 4 * j + 8 * i,
                    3 + 4 * j + 8 * i,
                    48 + 64 * (i ^ j) as i32,
                    false,
                );
            }
        }
    }
    let mut copy = [0i64; 32];
    copy[..size].copy_from_slice(&t[..size]);
    for i in 0..size {
        t[i] = copy[brev(n, i ^ (i >> 1))];
    }
    if n == 3 {
        for i in 0..4 {
            t[1 + 2 * i] = -t[1 + 2 * i];
        }
    } else {
        for i in 0..2 {
            for j in 0..2 {
                t[1 + 12 * j + 2 * i] = -t[1 + 12 * j + 2 * i];
            }
        }
    }
}
fn wht(t: &mut [i64], shift: u32) {
    let mut a = t[0] >> shift;
    let mut c = t[1] >> shift;
    let mut d = t[2] >> shift;
    let mut b = t[3] >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    t.copy_from_slice(&[a, b, c, d]);
}
fn one(t: &mut [i64], n: usize, sine: bool) {
    if sine {
        adst(t, n);
    } else {
        let mut copy = [0i64; 32];
        copy[..t.len()].copy_from_slice(t);
        for i in 0..t.len() {
            t[i] = copy[brev(n, i)];
        }
        dct(t, n);
    }
}
/// Return residuals in raster order. Coefficients must already be dequantized.
/// The `scratch` buffer holds the i64 intermediate and is reused across calls.
/// The `out` buffer receives the i32 residuals and is resized as needed.
pub fn inverse(
    coefficients: &[i32],
    size: usize,
    depth: u8,
    kind: Kind,
    scratch: &mut Vec<i64>,
    out: &mut Vec<i32>,
) -> Result<()> {
    if ![4, 8, 16, 32].contains(&size)
        || ![8, 10, 12].contains(&depth)
        || coefficients.len() != size * size
        || (kind == Kind::Lossless && size != 4)
        || (size == 32 && kind != Kind::DctDct)
    {
        return Err(invalid("invalid VP9 inverse transform parameters"));
    }
    let bound = 1i64 << (7 + depth);
    if coefficients
        .iter()
        .any(|&v| !(-bound..bound).contains(&i64::from(v)))
    {
        return Err(invalid(
            "VP9 dequantized coefficient exceeds bit-depth range",
        ));
    }
    let n = size.trailing_zeros() as usize;
    let len = size * size;
    scratch.resize(len, 0);
    for (d, &c) in scratch[..len].iter_mut().zip(coefficients) {
        *d = i64::from(c);
    }
    for row in scratch[..len].chunks_exact_mut(size) {
        if kind == Kind::Lossless {
            wht(row, 2);
        } else {
            one(row, n, matches!(kind, Kind::DctAdst | Kind::AdstAdst));
        }
    }
    let mut col = [0i64; 32];
    for x in 0..size {
        for y in 0..size {
            col[y] = scratch[y * size + x];
        }
        if kind == Kind::Lossless {
            wht(&mut col[..size], 0);
        } else {
            one(
                &mut col[..size],
                n,
                matches!(kind, Kind::AdstDct | Kind::AdstAdst),
            );
        }
        for y in 0..size {
            scratch[y * size + x] = if kind == Kind::Lossless {
                col[y]
            } else {
                round(col[y], (n + 2).min(6) as u32)
            };
        }
    }
    out.resize(len, 0);
    for (d, &s) in out[..len].iter_mut().zip(&scratch[..len]) {
        *d = i32::try_from(s).map_err(|_| invalid("VP9 inverse transform overflow"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_transform_sizes_and_kinds_match_independent_libvpx_pixels() {
        let expected = include_bytes!("../../tests/fixtures/vp9/transforms.bin");
        let mut at = 0;
        let mut rng = 0x73ab9215u32;
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        for size in [4, 8, 16, 32] {
            let kinds = if size == 4 {
                5
            } else if size == 32 {
                1
            } else {
                4
            };
            for kind in &[
                Kind::DctDct,
                Kind::AdstDct,
                Kind::DctAdst,
                Kind::AdstAdst,
                Kind::Lossless,
            ][..kinds]
            {
                for test in 0..32 {
                    let coefficients: Vec<_> = (0..size * size)
                        .map(|_| {
                            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
                            ((rng >> 16) % 65) as i32 - 32
                        })
                        .collect();
                    inverse(&coefficients, size, 8, *kind, &mut scratch, &mut out).unwrap();
                    let pixels: Vec<_> =
                        out.iter().map(|v| (128 + v).clamp(0, 255) as u8).collect();
                    assert_eq!(
                        pixels,
                        &expected[at..at + size * size],
                        "size={size} kind={kind:?} test={test}"
                    );
                    at += size * size;
                }
            }
        }
        assert_eq!(at, expected.len());
    }
}
