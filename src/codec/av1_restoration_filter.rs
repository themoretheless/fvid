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
fn round(v: i64, b: u32) -> i64 {
    if b == 0 {
        v
    } else {
        (v + (1 << (b - 1))) >> b
    }
}
pub(super) struct Source<'a> {
    pub before: &'a Plane,
    pub cdef: &'a Plane,
    pub width: usize,
    pub height: usize,
    pub start: i64,
    pub end: i64,
    pub depth: u8,
}
impl Source<'_> {
    fn sample(&self, x: i64, y: i64) -> i64 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let mut y = y.clamp(0, self.height as i64 - 1);
        let plane = if y < self.start {
            y = y.max(self.start - 2);
            self.before
        } else if y > self.end {
            y = y.min(self.end + 2);
            self.before
        } else {
            self.cdef
        };
        i64::from(plane.samples[y as usize * plane.width + x])
    }
    fn current(&self, x: usize, y: usize) -> i64 {
        i64::from(self.cdef.samples[y * self.cdef.width + x])
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
            let f0 = box_filter(source, x, y, w, h, parameters[0], parameters[1], 0);
            let f1 = box_filter(source, x, y, w, h, parameters[2], parameters[3], 1);
            let w0 = i64::from(weights[0]);
            let w1 = i64::from(weights[1]);
            let w2 = 128 - w0 - w1;
            for row in 0..h {
                for col in 0..w {
                    let at = row * w + col;
                    let u = source.current(x + col, y + row) << 4;
                    let v = w1 * u
                        + w0 * f0.as_ref().map_or(u, |f| f[at])
                        + w2 * f1.as_ref().map_or(u, |f| f[at]);
                    output.samples[(y + row) * output.width + x + col] =
                        round(v, 11).clamp(0, (1 << source.depth) - 1) as u16;
                }
            }
        }
    }
}
fn box_filter(
    s: &Source<'_>,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    r: i64,
    eps: i64,
    pass: usize,
) -> Option<Vec<i64>> {
    if r == 0 {
        return None;
    }
    let stride = w + 2;
    let mut a = vec![0i64; stride * (h + 2)];
    let mut b = vec![0i64; stride * (h + 2)];
    let n = (2 * r + 1) * (2 * r + 1);
    let n2e = n * n * eps;
    let scale = ((1 << 20) + n2e / 2) / n2e;
    let reciprocal = ((1 << 12) + n / 2) / n;
    for row in -1..h as i64 + 1 {
        for col in -1..w as i64 + 1 {
            let mut square = 0;
            let mut sum = 0;
            for dy in -r..=r {
                for dx in -r..=r {
                    let v = s.sample(x as i64 + col + dx, y as i64 + row + dy);
                    square += v * v;
                    sum += v;
                }
            }
            let variance = (round(square, 2 * u32::from(s.depth - 8)) * n
                - round(sum, u32::from(s.depth - 8)).pow(2))
            .max(0);
            let z = round(variance * scale, 20);
            let a2 = if z >= 255 {
                256
            } else if z == 0 {
                1
            } else {
                ((z << 8) + z / 2) / (z + 1)
            };
            let at = (row + 1) as usize * stride + (col + 1) as usize;
            a[at] = a2;
            b[at] = round((256 - a2) * sum * reciprocal, 12);
        }
    }
    let mut result = vec![0; w * h];
    for row in 0..h {
        for col in 0..w {
            let mut av = 0;
            let mut bv = 0;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let weight = if pass == 0 {
                        if (row as i64 + dy) & 1 != 0 {
                            if dx == 0 {
                                6
                            } else {
                                5
                            }
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
    Some(result)
}
fn coefficients(taps: &[i32; 3]) -> [i64; 7] {
    let mut f = [0; 7];
    f[3] = 128;
    for i in 0..3 {
        f[i] = i64::from(taps[i]);
        f[6 - i] = f[i];
        f[3] -= 2 * f[i];
    }
    f
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
    let offset = 1i64 << (u32::from(s.depth) + 7 - r0 - 1);
    let limit = (1i64 << (u32::from(s.depth) + 1 + 7 - r0)) - 1;
    let mut intermediate = vec![0i64; w * (h + 6)];
    for row in 0..h + 6 {
        for col in 0..w {
            let mut sum = 0;
            for t in 0..7 {
                sum += hf[t]
                    * s.sample(
                        x as i64 + col as i64 + t as i64 - 3,
                        y as i64 + row as i64 - 3,
                    );
            }
            intermediate[row * w + col] = round(sum, r0).clamp(-offset, limit - offset);
        }
    }
    for row in 0..h {
        for col in 0..w {
            let mut sum = 0;
            for t in 0..7 {
                sum += vf[t] * intermediate[(row + t) * w + col];
            }
            output.samples[(y + row) * output.width + x + col] =
                round(sum, r1).clamp(0, (1 << s.depth) - 1) as u16;
        }
    }
}
