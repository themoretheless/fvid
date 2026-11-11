//! AVC Intra4x4 luma and 4:2:0 chroma prediction (8.3.1 and 8.3.4).
//! Neighbours must be reconstructed, pre-deblocking pixels. The caller applies slice availability rules.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Intra4Mode {
    Vertical = 0,
    Horizontal = 1,
    Dc = 2,
    DiagonalDownLeft = 3,
    DiagonalDownRight = 4,
    VerticalRight = 5,
    HorizontalDown = 6,
    VerticalLeft = 7,
    HorizontalUp = 8,
}
impl TryFrom<u8> for Intra4Mode {
    type Error = crate::Error;
    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            0 => Self::Vertical,
            1 => Self::Horizontal,
            2 => Self::Dc,
            3 => Self::DiagonalDownLeft,
            4 => Self::DiagonalDownRight,
            5 => Self::VerticalRight,
            6 => Self::HorizontalDown,
            7 => Self::VerticalLeft,
            8 => Self::HorizontalUp,
            _ => return Err(invalid("invalid Intra4 mode")),
        })
    }
}
pub fn derive_intra4_mode(
    left: Option<Intra4Mode>,
    top: Option<Intra4Mode>,
    use_predicted: bool,
    remainder: u8,
) -> Result<Intra4Mode> {
    let predicted = match (left, top) {
        (Some(l), Some(t)) => (l as u8).min(t as u8),
        _ => 2,
    };
    if use_predicted {
        return Intra4Mode::try_from(predicted);
    }
    if remainder > 7 {
        return Err(invalid("invalid Intra4 remainder"));
    }
    Intra4Mode::try_from(if remainder < predicted {
        remainder
    } else {
        remainder + 1
    })
}
fn max_sample(depth: u8) -> Result<u16> {
    if !(8..=14).contains(&depth) {
        return Err(invalid("invalid AVC bit depth"));
    }
    Ok((1u16 << depth) - 1)
}
fn avg(a: u16, b: u16) -> u16 {
    ((u32::from(a) + u32::from(b) + 1) >> 1) as u16
}
fn filt(a: u16, b: u16, c: u16) -> u16 {
    ((u32::from(a) + 2 * u32::from(b) + u32::from(c) + 2) >> 2) as u16
}
fn vr(x: i32, y: i32, top: &[u16], left: &[u16], corner: u16) -> u16 {
    let t = |i: i32| if i < 0 { corner } else { top[i as usize] };
    let l = |i: i32| if i < 0 { corner } else { left[i as usize] };
    let z = 2 * x - y;
    if z >= 0 {
        let at = x - (y >> 1);
        if z % 2 == 0 {
            avg(t(at - 1), t(at))
        } else {
            filt(t(at - 2), t(at - 1), t(at))
        }
    } else if z == -1 {
        filt(left[0], corner, top[0])
    } else {
        filt(l(y - 2 * x - 1), l(y - 2 * x - 2), l(y - 2 * x - 3))
    }
}
pub fn intra4(
    mode: Intra4Mode,
    top: Option<&[u16; 4]>,
    top_right: Option<&[u16; 4]>,
    left: Option<&[u16; 4]>,
    corner: Option<u16>,
    depth: u8,
) -> Result<[u16; 16]> {
    intra_n::<4, 16>(mode, top, top_right, left, corner, depth, false)
}
/// Intra8 uses the same mode numbering, with filtered neighbouring samples.
pub fn intra8(
    mode: Intra4Mode,
    top: Option<&[u16; 8]>,
    top_right: Option<&[u16; 8]>,
    left: Option<&[u16; 8]>,
    corner: Option<u16>,
    depth: u8,
) -> Result<[u16; 64]> {
    intra_n::<8, 64>(mode, top, top_right, left, corner, depth, true)
}

#[cfg(test)]
mod intra8_tests {
    use super::*;
    #[test]
    fn filtered_edges_directional_negative_branch_and_missing_neighbours() {
        let top = std::array::from_fn(|i| 20 + 4 * i as u16);
        let right = std::array::from_fn(|i| 52 + 4 * i as u16);
        let left = std::array::from_fn(|i| 40 + 4 * i as u16);
        let predict =
            |mode| intra8(mode, Some(&top), Some(&right), Some(&left), Some(16), 8).unwrap();
        assert_eq!(&predict(Intra4Mode::Vertical)[..8], &top);
        let horizontal = predict(Intra4Mode::Horizontal);
        assert_eq!(&horizontal[..8], &[35; 8]);
        assert_eq!(&horizontal[56..], &[67; 8]);
        assert_eq!(predict(Intra4Mode::Dc), [44; 64]);
        assert_eq!(predict(Intra4Mode::VerticalRight)[7 * 8 + 2], 43);
        assert_eq!(predict(Intra4Mode::HorizontalDown)[2 * 8 + 7], 24);
        assert_eq!(
            intra8(Intra4Mode::Dc, None, None, None, None, 10).unwrap(),
            [512; 64]
        );
        assert!(
            intra8(
                Intra4Mode::DiagonalDownRight,
                Some(&top),
                None,
                Some(&left),
                None,
                8
            )
            .is_err()
        );
        for mode in 0..9 {
            assert_eq!(
                intra8(
                    Intra4Mode::try_from(mode).unwrap(),
                    Some(&[777; 8]),
                    None,
                    Some(&[777; 8]),
                    Some(777),
                    10
                )
                .unwrap(),
                [777; 64]
            );
        }
    }
}
fn intra_n<const N: usize, const M: usize>(
    mode: Intra4Mode,
    top: Option<&[u16; N]>,
    top_right: Option<&[u16; N]>,
    left: Option<&[u16; N]>,
    corner: Option<u16>,
    depth: u8,
    filter_references: bool,
) -> Result<[u16; M]> {
    let max = max_sample(depth)?;
    if top
        .into_iter()
        .chain(top_right)
        .chain(left)
        .flatten()
        .any(|&v| v > max)
        || corner.is_some_and(|v| v > max)
    {
        return Err(invalid("Intra4 neighbour exceeds bit depth"));
    }
    let require_top = matches!(
        mode,
        Intra4Mode::Vertical
            | Intra4Mode::DiagonalDownLeft
            | Intra4Mode::DiagonalDownRight
            | Intra4Mode::VerticalRight
            | Intra4Mode::HorizontalDown
            | Intra4Mode::VerticalLeft
    );
    let require_left = matches!(
        mode,
        Intra4Mode::Horizontal
            | Intra4Mode::DiagonalDownRight
            | Intra4Mode::VerticalRight
            | Intra4Mode::HorizontalDown
            | Intra4Mode::HorizontalUp
    );
    let require_corner = matches!(
        mode,
        Intra4Mode::DiagonalDownRight | Intra4Mode::VerticalRight | Intra4Mode::HorizontalDown
    );
    if (require_top && top.is_none())
        || (require_left && left.is_none())
        || (require_corner && corner.is_none())
    {
        return Err(invalid("required Intra4 neighbour unavailable"));
    }
    let mut t = [0; 16];
    if let Some(top) = top {
        t[..N].copy_from_slice(top);
        t[N..2 * N].copy_from_slice(top_right.unwrap_or(&[top[N - 1]; N]));
    }
    let mut l = left.copied().unwrap_or([0; N]);
    let mut c = corner.unwrap_or(0);
    if filter_references {
        let original_t = t;
        let original_l = l;
        if top.is_some() {
            for i in 0..2 * N {
                t[i] = filt(
                    if i == 0 {
                        corner.unwrap_or(original_t[0])
                    } else {
                        original_t[i - 1]
                    },
                    original_t[i],
                    original_t[(i + 1).min(2 * N - 1)],
                );
            }
        }
        if left.is_some() {
            for i in 0..N {
                l[i] = filt(
                    if i == 0 {
                        corner.unwrap_or(original_l[0])
                    } else {
                        original_l[i - 1]
                    },
                    original_l[i],
                    original_l[(i + 1).min(N - 1)],
                );
            }
        }
        if corner.is_some() {
            c = filt(
                if top.is_some() { original_t[0] } else { c },
                c,
                if left.is_some() { original_l[0] } else { c },
            );
        }
    }
    let sum = |v: &[u16]| v.iter().map(|&n| u32::from(n)).sum::<u32>();
    let dc = match (top.is_some(), left.is_some()) {
        (true, true) => (sum(&t[..N]) + sum(&l) + N as u32) / (2 * N) as u32,
        (true, false) => (sum(&t[..N]) + (N / 2) as u32) / N as u32,
        (false, true) => (sum(&l) + (N / 2) as u32) / N as u32,
        _ => 1 << (depth - 1),
    } as u16;
    let mut out = [0; M];
    for y in 0..N {
        for x in 0..N {
            out[y * N + x] = match mode {
                Intra4Mode::Vertical => t[x],
                Intra4Mode::Horizontal => l[y],
                Intra4Mode::Dc => dc,
                Intra4Mode::DiagonalDownLeft => {
                    let at = x + y;
                    filt(t[at], t[at + 1], t[(at + 2).min(2 * N - 1)])
                }
                Intra4Mode::DiagonalDownRight => {
                    if x == y {
                        filt(t[0], c, l[0])
                    } else {
                        let (edge, d) = if x > y {
                            (&t[..N], x - y)
                        } else {
                            (&l[..], y - x)
                        };
                        filt(if d == 1 { c } else { edge[d - 2] }, edge[d - 1], edge[d])
                    }
                }
                Intra4Mode::VerticalRight => vr(x as i32, y as i32, &t[..N], &l, c),
                Intra4Mode::HorizontalDown => vr(y as i32, x as i32, &l, &t[..N], c),
                Intra4Mode::VerticalLeft => {
                    let at = x + y / 2;
                    if y % 2 == 0 {
                        avg(t[at], t[at + 1])
                    } else {
                        filt(t[at], t[at + 1], t[at + 2])
                    }
                }
                Intra4Mode::HorizontalUp => {
                    let at = y + x / 2;
                    if x % 2 == 0 {
                        avg(l[at.min(N - 1)], l[(at + 1).min(N - 1)])
                    } else {
                        filt(
                            l[at.min(N - 1)],
                            l[(at + 1).min(N - 1)],
                            l[(at + 2).min(N - 1)],
                        )
                    }
                }
            };
        }
    }
    Ok(out)
}
#[derive(Clone, Copy, Debug)]
pub enum ChromaMode {
    Dc,
    Horizontal,
    Vertical,
    Plane,
}
impl TryFrom<u8> for ChromaMode {
    type Error = crate::Error;
    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            0 => Self::Dc,
            1 => Self::Horizontal,
            2 => Self::Vertical,
            3 => Self::Plane,
            _ => return Err(invalid("invalid chroma prediction mode")),
        })
    }
}
pub fn chroma8(
    mode: ChromaMode,
    top: Option<&[u16; 8]>,
    left: Option<&[u16; 8]>,
    corner: Option<u16>,
    depth: u8,
) -> Result<[u16; 64]> {
    let max = max_sample(depth)?;
    if top.into_iter().chain(left).flatten().any(|&v| v > max) || corner.is_some_and(|v| v > max) {
        return Err(invalid("chroma neighbour exceeds bit depth"));
    }
    let mut out = [0; 64];
    match mode {
        ChromaMode::Vertical => {
            let t = top.ok_or_else(|| invalid("chroma vertical requires top"))?;
            for row in out.chunks_exact_mut(8) {
                row.copy_from_slice(t);
            }
        }
        ChromaMode::Horizontal => {
            let l = left.ok_or_else(|| invalid("chroma horizontal requires left"))?;
            for (y, row) in out.chunks_exact_mut(8).enumerate() {
                row.fill(l[y]);
            }
        }
        ChromaMode::Dc => {
            return chroma8_dc(
                std::array::from_fn(|half| top.map(|v| std::array::from_fn(|i| v[half * 4 + i]))),
                std::array::from_fn(|half| left.map(|v| std::array::from_fn(|i| v[half * 4 + i]))),
                depth,
            );
        }
        ChromaMode::Plane => {
            let t = top.ok_or_else(|| invalid("chroma plane requires top"))?;
            let l = left.ok_or_else(|| invalid("chroma plane requires left"))?;
            let corner = i32::from(corner.ok_or_else(|| invalid("chroma plane requires corner"))?);
            let gradient = |edge: &[u16; 8]| {
                (1..=4)
                    .map(|i| {
                        i as i32
                            * (i32::from(edge[3 + i])
                                - if i == 4 {
                                    corner
                                } else {
                                    i32::from(edge[3 - i])
                                })
                    })
                    .sum::<i32>()
            };
            let a = 16 * (i32::from(t[7]) + i32::from(l[7]));
            let b = (17 * gradient(t) + 16) >> 5;
            let c = (17 * gradient(l) + 16) >> 5;
            for y in 0..8 {
                for x in 0..8 {
                    out[y * 8 + x] = ((a + b * (x as i32 - 3) + c * (y as i32 - 3) + 16) >> 5)
                        .clamp(0, i32::from(max)) as u16;
                }
            }
        }
    }
    Ok(out)
}


/// Chroma DC uses separate availability for each four-sample boundary group
/// (8.3.4.1), including mixed frame/field neighbors under constrained intra.
pub fn chroma8_dc(
    top: [Option<[u16; 4]>; 2],
    left: [Option<[u16; 4]>; 2],
    depth: u8,
) -> Result<[u16; 64]> {
    let max = max_sample(depth)?;
    if top.iter().chain(&left).flatten().flatten().any(|&v| v > max) {
        return Err(invalid("chroma neighbour exceeds bit depth"));
    }
    let sums = |edges: [Option<[u16; 4]>; 2]| {
        edges.map(|edge| edge.map(|v| v.iter().map(|&n| u32::from(n)).sum::<u32>()))
    };
    let (top, left) = (sums(top), sums(left));
    let mut out = [0; 64];
    for by in 0..2 {
        for bx in 0..2 {
            let (t, l) = (top[bx], left[by]);
            let value = if bx == 1 && by == 0 {
                t.or(l).map(|n| (n + 2) >> 2)
            } else if bx == 0 && by == 1 {
                l.or(t).map(|n| (n + 2) >> 2)
            } else {
                match (t, l) {
                    (Some(t), Some(l)) => Some((t + l + 4) >> 3),
                    (Some(n), None) | (None, Some(n)) => Some((n + 2) >> 2),
                    _ => None,
                }
            }.unwrap_or(1 << (depth - 1)) as u16;
            for row in 0..4 {
                out[(by * 4 + row) * 8 + bx * 4..(by * 4 + row) * 8 + bx * 4 + 4].fill(value);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_nine_modes_match_explicit_vectors() {
        let top = [10, 20, 30, 40];
        let right = [50, 60, 70, 80];
        let left = [90, 100, 110, 120];
        let expected = [
            [
                10, 20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40,
            ],
            [
                90, 90, 90, 90, 100, 100, 100, 100, 110, 110, 110, 110, 120, 120, 120, 120,
            ],
            [65; 16],
            [
                20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 78,
            ],
            [
                65, 30, 20, 30, 90, 65, 30, 20, 100, 90, 65, 30, 110, 100, 90, 65,
            ],
            [
                45, 15, 25, 35, 65, 30, 20, 30, 90, 45, 15, 25, 100, 65, 30, 20,
            ],
            [
                85, 65, 30, 20, 95, 90, 85, 65, 105, 100, 95, 90, 115, 110, 105, 100,
            ],
            [
                15, 25, 35, 45, 20, 30, 40, 50, 25, 35, 45, 55, 30, 40, 50, 60,
            ],
            [
                95, 100, 105, 110, 105, 110, 115, 118, 115, 118, 120, 120, 120, 120, 120, 120,
            ],
        ];
        for (mode, expected) in expected.iter().enumerate() {
            assert_eq!(
                &intra4(
                    Intra4Mode::try_from(mode as u8).unwrap(),
                    Some(&top),
                    Some(&right),
                    Some(&left),
                    Some(80),
                    8
                )
                .unwrap(),
                expected,
                "mode {mode}"
            );
        }
    }
    #[test]
    fn unavailable_neighbours_and_top_right_replication() {
        assert_eq!(
            intra4(Intra4Mode::Dc, None, None, None, None, 10).unwrap(),
            [512; 16]
        );
        assert_eq!(
            intra4(Intra4Mode::Dc, Some(&[10, 20, 30, 40]), None, None, None, 8).unwrap(),
            [25; 16]
        );
        assert!(intra4(Intra4Mode::Vertical, None, None, Some(&[20; 4]), None, 8).is_err());
        assert!(
            intra4(
                Intra4Mode::DiagonalDownRight,
                Some(&[20; 4]),
                None,
                Some(&[20; 4]),
                None,
                8
            )
            .is_err()
        );
        let top = [10, 20, 30, 40];
        for mode in [Intra4Mode::VerticalLeft, Intra4Mode::DiagonalDownLeft] {
            assert_eq!(
                intra4(mode, Some(&top), None, None, None, 8).unwrap(),
                intra4(mode, Some(&top), Some(&[40; 4]), None, None, 8).unwrap()
            );
        }
        assert_eq!(
            derive_intra4_mode(Some(Intra4Mode::Vertical), None, true, 0).unwrap(),
            Intra4Mode::Dc
        );
        assert_eq!(
            derive_intra4_mode(Some(Intra4Mode::Horizontal), Some(Intra4Mode::Dc), false, 1)
                .unwrap(),
            Intra4Mode::Dc
        );
        assert!(derive_intra4_mode(None, None, false, 8).is_err());
    }
    #[test]
    fn chroma_dc_retains_partial_four_sample_boundary_availability() {
        for (top, left, expected) in [
            ([None, None], [Some([64; 4]), None], [64, 64, 128, 128]),
            ([Some([10; 4]), None], [None, Some([30; 4])], [10, 128, 30, 30]),
            ([None, Some([20; 4])], [Some([60; 4]), None], [60, 20, 128, 20]),
        ] {
            let pixels = chroma8_dc(top, left, 8).unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(pixels[y * 8 + x], expected[y / 4 * 2 + x / 4]);
                }
            }
        }
        assert_eq!(chroma8_dc([None; 2], [None; 2], 10).unwrap(), [512; 64]);
        assert!(chroma8_dc([Some([256; 4]), None], [None; 2], 8).is_err());
    }
    #[test]
    fn chroma_dc_quadrants_and_affine_plane() {
        let top = [20, 20, 20, 20, 40, 40, 40, 40];
        let left = [60, 60, 60, 60, 80, 80, 80, 80];
        let dc = chroma8(ChromaMode::Dc, Some(&top), Some(&left), None, 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(dc[y * 8 + x], [[40, 40], [80, 60]][y / 4][x / 4]);
            }
        }
        let edge = std::array::from_fn(|i| 78 + 2 * i as u16);
        let plane = chroma8(ChromaMode::Plane, Some(&edge), Some(&edge), Some(76), 8).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(plane[y * 8 + x], 80 + 2 * x as u16 + 2 * y as u16);
            }
        }
    }
    #[test]
    fn constant_fields_and_component_bounds() {
        for depth in 8..=14 {
            let max = (1u16 << depth) - 1;
            for mode in 0..9 {
                assert_eq!(
                    intra4(
                        Intra4Mode::try_from(mode).unwrap(),
                        Some(&[max; 4]),
                        Some(&[max; 4]),
                        Some(&[max; 4]),
                        Some(max),
                        depth
                    )
                    .unwrap(),
                    [max; 16]
                );
            }
            for mode in 0..4 {
                assert_eq!(
                    chroma8(
                        ChromaMode::try_from(mode).unwrap(),
                        Some(&[max; 8]),
                        Some(&[max; 8]),
                        Some(max),
                        depth
                    )
                    .unwrap(),
                    [max; 64]
                );
            }
        }
        assert!(intra4(Intra4Mode::Dc, Some(&[256; 4]), None, None, None, 8).is_err());
        assert!(chroma8(ChromaMode::Plane, Some(&[1; 8]), Some(&[1; 8]), None, 8).is_err());
        assert!(chroma8(ChromaMode::Dc, None, None, None, 0).is_err());
    }
}
