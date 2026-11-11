//! VP9 intra prediction, bitstream specification section 8.5.1.
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    Dc,
    Vertical,
    Horizontal,
    D45,
    D135,
    D117,
    D153,
    D207,
    D63,
    TrueMotion,
}
impl TryFrom<u8> for Mode {
    type Error = crate::Error;
    fn try_from(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::Dc,
            1 => Self::Vertical,
            2 => Self::Horizontal,
            3 => Self::D45,
            4 => Self::D135,
            5 => Self::D117,
            6 => Self::D153,
            7 => Self::D207,
            8 => Self::D63,
            9 => Self::TrueMotion,
            _ => return Err(invalid("invalid VP9 intra mode")),
        })
    }
}
/// Prepared references include edge extension: above has 2*size samples,
/// left has size samples. Availability controls DC prediction independently of
/// the synthesized reference values used by directional prediction.
pub struct References<'a> {
    pub above: &'a [u16],
    pub left: &'a [u16],
    pub corner: u16,
    pub have_above: bool,
    pub have_left: bool,
}
pub fn predict(
    out: &mut [u16],
    size: usize,
    depth: u8,
    mode: Mode,
    refs: &References<'_>,
) -> Result<()> {
    if ![4, 8, 16, 32].contains(&size) || ![8, 10, 12].contains(&depth) {
        return Err(invalid("invalid VP9 intra geometry or depth"));
    }
    if out.len() < size * size {
        return Err(invalid("VP9 intra output buffer too small"));
    }
    let max = (1u16 << depth) - 1;
    if refs.above.len() != 2 * size
        || refs.left.len() != size
        || refs.corner > max
        || refs.above.iter().chain(refs.left).any(|&v| v > max)
    {
        return Err(invalid("invalid VP9 intra references"));
    }
    let a = |i: isize| {
        i32::from(if i < 0 {
            refs.corner
        } else {
            refs.above[i as usize]
        })
    };
    let l = |i: usize| i32::from(refs.left[i.min(size - 1)]);
    let avg2 = |a: i32, b: i32| ((a + b + 1) >> 1) as u16;
    let avg3 = |a: i32, b: i32, c: i32| ((a + 2 * b + c + 2) >> 2) as u16;
    let out = &mut out[..size * size];
    out.fill(0);
    match mode {
        Mode::Dc => {
            let mut sum = 0u32;
            let mut count = 0;
            if refs.have_above {
                sum += refs.above[..size]
                    .iter()
                    .map(|&v| u32::from(v))
                    .sum::<u32>();
                count += size as u32;
            }
            if refs.have_left {
                sum += refs.left.iter().map(|&v| u32::from(v)).sum::<u32>();
                count += size as u32;
            }
            let dc = if count == 0 {
                1 << (depth - 1)
            } else {
                ((sum + count / 2) / count) as u16
            };
            out.fill(dc);
        }
        Mode::Vertical | Mode::Horizontal | Mode::D45 | Mode::D63 | Mode::TrueMotion => {
            for y in 0..size {
                for x in 0..size {
                    out[y * size + x] = match mode {
                        Mode::Vertical => a(x as isize) as u16,
                        Mode::Horizontal => l(y) as u16,
                        Mode::D45 => {
                            let i = (x + y) as isize;
                            if x + y + 2 < 2 * size {
                                avg3(a(i), a(i + 1), a(i + 2))
                            } else {
                                a((2 * size - 1) as isize) as u16
                            }
                        }
                        Mode::D63 => {
                            let i = (y / 2 + x) as isize;
                            if y & 1 == 0 {
                                avg2(a(i), a(i + 1))
                            } else {
                                avg3(a(i), a(i + 1), a(i + 2))
                            }
                        }
                        Mode::TrueMotion => {
                            (a(x as isize) + l(y) - a(-1)).clamp(0, i32::from(max)) as u16
                        }
                        _ => unreachable!(),
                    };
                }
            }
        }
        Mode::D207 => {
            for y in (0..size).rev() {
                for x in 0..size {
                    out[y * size + x] = if y == size - 1 {
                        l(y) as u16
                    } else if x == 0 {
                        avg2(l(y), l(y + 1))
                    } else if x == 1 {
                        avg3(l(y), l(y + 1), l(y + 2))
                    } else {
                        out[(y + 1) * size + x - 2]
                    };
                }
            }
        }
        Mode::D135 => {
            out[0] = avg3(l(0), a(-1), a(0));
            for x in 1..size {
                let j = x as isize;
                out[x] = avg3(a(j - 2), a(j - 1), a(j));
            }
            out[size] = avg3(a(-1), l(0), l(1));
            for y in 2..size {
                out[y * size] = avg3(l(y - 2), l(y - 1), l(y));
            }
            for y in 1..size {
                for x in 1..size {
                    out[y * size + x] = out[(y - 1) * size + x - 1];
                }
            }
        }
        Mode::D117 => {
            for x in 0..size {
                let j = x as isize;
                out[x] = avg2(a(j - 1), a(j));
            }
            out[size] = avg3(l(0), a(-1), a(0));
            for x in 1..size {
                let j = x as isize;
                out[size + x] = avg3(a(j - 2), a(j - 1), a(j));
            }
            out[2 * size] = avg3(a(-1), l(0), l(1));
            for y in 3..size {
                out[y * size] = avg3(l(y - 3), l(y - 2), l(y - 1));
            }
            for y in 2..size {
                for x in 1..size {
                    out[y * size + x] = out[(y - 2) * size + x - 1];
                }
            }
        }
        Mode::D153 => {
            out[0] = avg2(l(0), a(-1));
            for y in 1..size {
                out[y * size] = avg2(l(y - 1), l(y));
            }
            out[1] = avg3(l(0), a(-1), a(0));
            out[size + 1] = avg3(a(-1), l(0), l(1));
            for y in 2..size {
                out[y * size + 1] = avg3(l(y - 2), l(y - 1), l(y));
            }
            for x in 2..size {
                let j = x as isize;
                out[x] = avg3(a(j - 3), a(j - 2), a(j - 1));
            }
            for y in 1..size {
                for x in 2..size {
                    out[y * size + x] = out[(y - 1) * size + x - 2];
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directional_ramps_and_true_motion_clipping() {
        let refs = References {
            above: &[10, 20, 30, 40, 50, 60, 70, 80],
            left: &[90, 100, 110, 120],
            corner: 0,
            have_above: true,
            have_left: true,
        };
        let mut out = vec![0u16; 16];
        predict(&mut out, 4, 8, Mode::D45, &refs).unwrap();
        assert_eq!(
            out,
            [
                20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80
            ]
        );
        predict(&mut out, 4, 8, Mode::D207, &refs).unwrap();
        assert_eq!(
            out,
            [
                95, 100, 105, 110, 105, 110, 115, 118, 115, 118, 120, 120, 120, 120, 120, 120
            ]
        );
        predict(&mut out, 4, 8, Mode::D135, &refs).unwrap();
        assert_eq!(
            out,
            [
                25, 10, 20, 30, 70, 25, 10, 20, 100, 70, 25, 10, 110, 100, 70, 25
            ]
        );
        let extreme = References {
            above: &[255; 8],
            left: &[255; 4],
            corner: 0,
            have_above: true,
            have_left: true,
        };
        predict(&mut out, 4, 8, Mode::TrueMotion, &extreme).unwrap();
        assert_eq!(out, [255; 16]);
        let extreme = References {
            above: &[0; 8],
            left: &[0; 4],
            corner: 255,
            ..extreme
        };
        predict(&mut out, 4, 8, Mode::TrueMotion, &extreme).unwrap();
        assert_eq!(out, [0; 16]);
    }
    #[test]
    fn every_mode_size_and_depth_preserves_constant_references() {
        for size in [4, 8, 16, 32] {
            for depth in [8, 10, 12] {
                let value = (1 << depth) - 1;
                let top = vec![value; 2 * size];
                let left = vec![value; size];
                let refs = References {
                    above: &top,
                    left: &left,
                    corner: value,
                    have_above: true,
                    have_left: true,
                };
                let mut out = vec![0u16; size * size];
                for mode in 0..10 {
                    predict(&mut out, size, depth, Mode::try_from(mode).unwrap(), &refs).unwrap();
                    assert!(out.iter().all(|&v| v == value));
                }
                predict(
                    &mut out,
                    size,
                    depth,
                    Mode::Dc,
                    &References {
                        have_above: false,
                        have_left: false,
                        ..refs
                    },
                )
                .unwrap();
                assert!(out.iter().all(|&v| v == 1 << (depth - 1)));
            }
        }
        assert!(Mode::try_from(10).is_err());
    }
}
