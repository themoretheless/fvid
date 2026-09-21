//! Main/Main10 4:2:0 intra sample prediction, H.265 8.4.4.2.
use crate::{Result, invalid};

/// Neighbours unavailable, inter-coded, PCM-coded, or (for top) in a different
/// CTU row must be supplied as None. This differs from sample availability.
pub fn luma_candidates(left: Option<u8>, top: Option<u8>) -> Result<[u8; 3]> {
    let a = left.unwrap_or(1);
    let b = top.unwrap_or(1);
    if a > 34 || b > 34 {
        return Err(invalid("invalid HEVC neighbouring intra mode"));
    }
    Ok(if a == b {
        if a < 2 {
            [0, 1, 26]
        } else {
            [a, 2 + (a + 29) % 32, 2 + (a - 1) % 32]
        }
    } else {
        [
            a,
            b,
            if a != 0 && b != 0 {
                0
            } else if a != 1 && b != 1 {
                1
            } else {
                26
            },
        ]
    })
}

/// `value` is either mpm_idx (0..2) or rem_intra_luma_pred_mode (0..31).
pub fn luma_mode(left: Option<u8>, top: Option<u8>, mpm: bool, value: u8) -> Result<u8> {
    let mut candidates = luma_candidates(left, top)?;
    if mpm {
        return candidates
            .get(usize::from(value))
            .copied()
            .ok_or_else(|| invalid("invalid HEVC MPM index"));
    }
    if value > 31 {
        return Err(invalid("invalid HEVC remaining intra mode"));
    }
    candidates.sort_unstable();
    let mut mode = value;
    for candidate in candidates {
        if mode >= candidate {
            mode += 1;
        }
    }
    Ok(mode)
}

/// Chroma mode derivation for Main/Main10 4:2:0. Value 4 derives from luma.
pub fn chroma_mode(luma: u8, value: u8) -> Result<u8> {
    if luma > 34 || value > 4 {
        return Err(invalid("invalid HEVC chroma intra mode"));
    }
    if value == 4 {
        return Ok(luma);
    }
    let selected = [0, 26, 10, 1][usize::from(value)];
    Ok(if selected == luma { 34 } else { selected })
}

/// Availability must already account for picture/slice/tile boundaries, z-scan
/// reconstruction order and constrained intra prediction. Top and left each
/// contain 2*side samples, starting immediately beside the block.
pub struct References {
    side: usize,
    depth: u8,
    corner: i32,
    top: Vec<i32>,
    left: Vec<i32>,
}
impl References {
    pub fn new(
        log2_size: u8,
        bit_depth: u8,
        corner: Option<u16>,
        top: &[Option<u16>],
        left: &[Option<u16>],
    ) -> Result<Self> {
        if !(2..=5).contains(&log2_size) || !(8..=10).contains(&bit_depth) {
            return Err(invalid("unsupported HEVC intra size or bit depth"));
        }
        let side = 1usize << log2_size;
        if top.len() != 2 * side || left.len() != 2 * side {
            return Err(invalid("invalid HEVC intra reference lengths"));
        }
        let chain: Vec<_> = left
            .iter()
            .rev()
            .copied()
            .chain([corner])
            .chain(top.iter().copied())
            .collect();
        if chain
            .iter()
            .flatten()
            .any(|&v| u32::from(v) >= 1u32 << bit_depth)
        {
            return Err(invalid("HEVC reference sample exceeds bit depth"));
        }
        let mut value = chain
            .iter()
            .flatten()
            .next()
            .copied()
            .unwrap_or(1 << (bit_depth - 1));
        let filled: Vec<_> = chain
            .into_iter()
            .map(|sample| {
                if let Some(v) = sample {
                    value = v;
                }
                i32::from(value)
            })
            .collect();
        Ok(Self {
            side,
            depth: bit_depth,
            corner: filled[2 * side],
            top: filled[2 * side + 1..].to_vec(),
            left: filled[..2 * side].iter().rev().copied().collect(),
        })
    }

    fn filtered(&self, mode: u8, chroma: bool, strong: bool) -> (i32, Vec<i32>, Vec<i32>) {
        let n = self.side;
        let distance = (i32::from(mode) - 26)
            .abs()
            .min((i32::from(mode) - 10).abs());
        let threshold = match n {
            8 => 7,
            16 => 1,
            _ => 0,
        };
        if chroma || mode == 1 || n == 4 || distance <= threshold {
            return (self.corner, self.top.clone(), self.left.clone());
        }
        let linear = strong
            && n == 32
            && [&self.top, &self.left]
                .into_iter()
                .all(|a| (self.corner + a[63] - 2 * a[31]).abs() < 1 << (self.depth - 5));
        let filter = |a: &[i32]| {
            (0..2 * n)
                .map(|i| {
                    if i == 2 * n - 1 {
                        a[i]
                    } else if linear {
                        (((63 - i) as i32 * self.corner) + ((i + 1) as i32 * a[63]) + 32) >> 6
                    } else {
                        ((if i == 0 { self.corner } else { a[i - 1] }) + 2 * a[i] + a[i + 1] + 2)
                            >> 2
                    }
                })
                .collect()
        };
        let corner = if linear {
            self.corner
        } else {
            (self.left[0] + 2 * self.corner + self.top[0] + 2) >> 2
        };
        (corner, filter(&self.top), filter(&self.left))
    }

    /// Modes 0=planar, 1=DC, 2..34=angular. Chroma means 4:2:0 Cb/Cr:
    /// reference smoothing and luma boundary correction do not apply there.
    pub fn predict(&self, mode: u8, chroma: bool, strong_smoothing: bool) -> Result<Vec<u16>> {
        if mode > 34 {
            return Err(invalid("invalid HEVC intra prediction mode"));
        }
        let n = self.side;
        let (corner, top, left) = self.filtered(mode, chroma, strong_smoothing);
        let mut output = vec![0u16; n * n];
        if mode == 0 {
            for y in 0..n {
                for x in 0..n {
                    output[y * n + x] = (((n - 1 - x) as i32 * left[y]
                        + (x + 1) as i32 * top[n]
                        + (n - 1 - y) as i32 * top[x]
                        + (y + 1) as i32 * left[n]
                        + n as i32)
                        / (2 * n) as i32) as u16;
                }
            }
        } else if mode == 1 {
            let dc = (top[..n].iter().sum::<i32>() + left[..n].iter().sum::<i32>() + n as i32)
                / (2 * n) as i32;
            output.fill(dc as u16);
            if !chroma && n < 32 {
                output[0] = ((left[0] + 2 * dc + top[0] + 2) >> 2) as u16;
                for i in 1..n {
                    output[i] = ((top[i] + 3 * dc + 2) >> 2) as u16;
                    output[i * n] = ((left[i] + 3 * dc + 2) >> 2) as u16;
                }
            }
        } else {
            const ANGLES: [i32; 33] = [
                32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17,
                -13, -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32,
            ];
            let angle = ANGLES[usize::from(mode - 2)];
            let vertical = mode >= 18;
            let (main, secondary) = if vertical {
                (&top, &left)
            } else {
                (&left, &top)
            };
            let inverse = match angle {
                -2 => 4096,
                -5 => 1638,
                -9 => 910,
                -13 => 630,
                -17 => 482,
                -21 => 390,
                -26 => 315,
                -32 => 256,
                _ => 0,
            };
            let reference = |index: i32| {
                if index == 0 {
                    corner
                } else if index > 0 {
                    main[index as usize - 1]
                } else {
                    secondary[((-index * inverse + 128) >> 8) as usize - 1]
                }
            };
            for y in 0..n {
                for x in 0..n {
                    let (along, across) = if vertical { (x, y) } else { (y, x) };
                    let delta = (across as i32 + 1) * angle;
                    let index = along as i32 + (delta >> 5) + 1;
                    let fraction = delta & 31;
                    let value = if fraction == 0 {
                        reference(index)
                    } else {
                        ((32 - fraction) * reference(index) + fraction * reference(index + 1) + 16)
                            >> 5
                    };
                    output[y * n + x] = value as u16;
                }
            }
            if !chroma && n < 32 && matches!(mode, 10 | 26) {
                for i in 0..n {
                    let value =
                        (main[0] + ((secondary[i] - corner) >> 1)).clamp(0, (1 << self.depth) - 1);
                    output[if vertical { i * n } else { i }] = value as u16;
                }
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_candidates_wrap_and_remainder_covers_exact_complement() {
        assert_eq!(luma_candidates(None, None).unwrap(), [0, 1, 26]);
        assert_eq!(luma_candidates(Some(2), Some(2)).unwrap(), [2, 33, 3]);
        assert_eq!(luma_candidates(Some(34), Some(34)).unwrap(), [34, 33, 3]);
        assert_eq!(luma_candidates(Some(0), Some(1)).unwrap(), [0, 1, 26]);
        for a in 0..35 {
            for b in 0..35 {
                let candidates = luma_candidates(Some(a), Some(b)).unwrap();
                let mut modes = Vec::new();
                for i in 0..3 {
                    assert_eq!(
                        luma_mode(Some(a), Some(b), true, i).unwrap(),
                        candidates[usize::from(i)]
                    );
                }
                for i in 0..32 {
                    modes.push(luma_mode(Some(a), Some(b), false, i).unwrap());
                }
                assert_eq!(
                    modes,
                    (0..35)
                        .filter(|m| !candidates.contains(m))
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(chroma_mode(a, 4).unwrap(), a);
            for (i, base) in [0, 26, 10, 1].into_iter().enumerate() {
                assert_eq!(
                    chroma_mode(a, i as u8).unwrap(),
                    if a == base { 34 } else { base }
                );
            }
        }
        assert!(luma_candidates(Some(35), None).is_err());
        assert!(luma_mode(None, None, true, 3).is_err());
        assert!(luma_mode(None, None, false, 32).is_err());
        assert!(chroma_mode(35, 0).is_err());
        assert!(chroma_mode(0, 5).is_err());
    }
    #[test]
    fn substitution_follows_bottom_left_to_top_right_order() {
        let r = References::new(
            2,
            8,
            None,
            &[Some(70), None, Some(90), None, None, None, None, None],
            &[None, None, Some(30), None, None, Some(60), None, None],
        )
        .unwrap();
        assert_eq!(r.left, [30, 30, 30, 60, 60, 60, 60, 60]);
        assert_eq!(r.corner, 30);
        assert_eq!(r.top, [70, 70, 90, 90, 90, 90, 90, 90]);
    }
    #[test]
    fn all_modes_sizes_components_preserve_constant_and_missing_edges() {
        for log in 2..=5 {
            for depth in [8, 10] {
                let edges = vec![None; 2 << log];
                let r = References::new(log, depth, None, &edges, &edges).unwrap();
                for mode in 0..=34 {
                    for chroma in [false, true] {
                        assert_eq!(
                            r.predict(mode, chroma, true).unwrap(),
                            vec![1 << (depth - 1); 1 << (2 * log)]
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn explicit_planar_dc_and_negative_diagonal_vectors() {
        let r = References::new(
            2,
            8,
            Some(10),
            &[
                Some(20),
                Some(30),
                Some(40),
                Some(50),
                Some(60),
                Some(70),
                Some(80),
                Some(90),
            ],
            &[
                Some(100),
                Some(110),
                Some(120),
                Some(130),
                Some(140),
                Some(150),
                Some(160),
                Some(170),
            ],
        )
        .unwrap();
        assert_eq!(
            r.predict(0, true, false).unwrap(),
            [
                70, 69, 68, 66, 89, 85, 81, 78, 108, 101, 95, 89, 126, 118, 109, 100
            ]
        );
        assert_eq!(r.predict(1, true, false).unwrap(), [75; 16]);
        assert_eq!(
            r.predict(1, false, false).unwrap(),
            [
                68, 64, 66, 69, 84, 75, 75, 75, 86, 75, 75, 75, 89, 75, 75, 75
            ]
        );
        assert_eq!(
            r.predict(18, true, false).unwrap(),
            [
                10, 20, 30, 40, 100, 10, 20, 30, 110, 100, 10, 20, 120, 110, 100, 10
            ]
        );
    }
    #[test]
    fn fractional_angles_round_and_extend_across_the_corner() {
        let top: Vec<_> = (0..8).map(|i| Some(20 + i * 10)).collect();
        let left: Vec<_> = (0..8).map(|i| Some(100 + i * 10)).collect();
        let r = References::new(2, 8, Some(10), &top, &left).unwrap();
        assert_eq!(
            r.predict(25, true, false).unwrap(),
            [
                19, 29, 39, 49, 19, 29, 39, 49, 18, 28, 38, 48, 18, 28, 38, 48
            ]
        );
        assert_eq!(
            r.predict(27, true, false).unwrap(),
            [
                21, 31, 41, 51, 21, 31, 41, 51, 22, 32, 42, 52, 23, 33, 43, 53
            ]
        );
        let a = r.predict(22, true, false).unwrap();
        assert_eq!([a[0], a[4], a[8], a[12]], [16, 12, 32, 73]);
    }
    #[test]
    fn strong_smoothing_uses_endpoints_and_strict_midpoint_threshold() {
        let mut top: Vec<_> = (0..64).map(|i| Some(102 + i * 2)).collect();
        let left: Vec<_> = (0..64).map(|i| Some(101 + i)).collect();
        top[0] = Some(255);
        let r = References::new(5, 8, Some(100), &top, &left).unwrap();
        let (corner, a, b) = r.filtered(0, false, true);
        assert_eq!(corner, 100);
        assert_eq!(a, (0..64).map(|i| 102 + i * 2).collect::<Vec<_>>());
        assert_eq!(b, (0..64).map(|i| 101 + i).collect::<Vec<_>>());
        let weak = r.filtered(0, false, false);
        assert_eq!((weak.0, weak.1[0]), (139, 179));
        top[31] = Some(160); // abs(100+228-2*160)=8, exactly the threshold
        let r = References::new(5, 8, Some(100), &top, &left).unwrap();
        assert_eq!(r.filtered(0, false, true), r.filtered(0, false, false));
        assert_eq!(r.filtered(1, false, true).1[0], 255); // DC never smooths
        assert_eq!(r.filtered(0, true, true).1[0], 255); // 4:2:0 chroma never smooths
    }
    #[test]
    fn invalid_references_and_modes_are_rejected() {
        assert!(References::new(1, 8, None, &[], &[]).is_err());
        assert!(References::new(2, 11, None, &[None; 8], &[None; 8]).is_err());
        assert!(References::new(2, 8, None, &[None; 7], &[None; 8]).is_err());
        assert!(References::new(2, 8, Some(256), &[None; 8], &[None; 8]).is_err());
        let r = References::new(2, 8, None, &[None; 8], &[None; 8]).unwrap();
        assert!(r.predict(35, false, false).is_err());
    }
    #[test]
    fn angular_transpose_symmetry_and_boundary_clipping() {
        for log in 2..=5 {
            let n = 1 << log;
            let top: Vec<_> = (0..2 * n)
                .map(|i| Some(((i * 71 + 43) % 1024) as u16))
                .collect();
            let left: Vec<_> = (0..2 * n)
                .map(|i| Some(((i * 137 + 3) % 1024) as u16))
                .collect();
            let a = References::new(log, 10, Some(1000), &top, &left).unwrap();
            let b = References::new(log, 10, Some(1000), &left, &top).unwrap();
            for mode in 2..=34 {
                let aa = a.predict(mode, false, true).unwrap();
                let bb = b.predict(36 - mode, false, true).unwrap();
                for y in 0..n {
                    for x in 0..n {
                        assert_eq!(aa[y * n + x], bb[x * n + y]);
                    }
                }
            }
            if n < 32 {
                assert_eq!(a.predict(26, false, false).unwrap()[0], 0);
            }
        }
    }
}
