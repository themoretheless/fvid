//! Progressive deblocking, ITU-T H.264 (02/2016), section 8.7.
//! Intra macroblock boundaries have bS=4; internal 4x4 edges have bS=3.
const ALPHA: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 17, 20,
    22, 25, 28, 32, 36, 40, 45, 50, 56, 63, 71, 80, 90, 101, 113, 127, 144, 162, 182, 203, 226,
    255, 255,
];
const BETA: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 6, 6, 7, 7, 8, 8,
    9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18,
];
const TC3: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
    3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 23, 25,
];

const TC1: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 6, 6, 7, 8, 9, 10, 11, 13,
];
const TC2: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2,
    2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 6, 7, 8, 8, 10, 11, 12, 13, 15, 17,
];

/// Filter one perpendicular sample line across a progressive edge.
/// p[0]/q[0] touch the edge; QP is the rounded average of neighbouring
/// component QPs without the bit-depth offset. Offsets are twice slice syntax.
pub fn filter_samples(
    p: [u16; 4],
    q: [u16; 4],
    qp: i32,
    offsets: [i32; 2],
    depth: u8,
    chroma: bool,
    strength: u8,
) -> crate::Result<([u16; 4], [u16; 4])> {
    if !(8..=14).contains(&depth)
        || strength > 4
        || !(-36..=51).contains(&qp)
        || offsets.iter().any(|v| !(-12..=12).contains(v))
    {
        return Err(crate::invalid("invalid AVC deblocking parameters"));
    }
    let max = (1u16 << depth) - 1;
    if p.iter().chain(&q).any(|&v| v > max) {
        return Err(crate::invalid("AVC deblocking sample exceeds bit depth"));
    }
    let (p, q) = filter(
        p.map(i32::from),
        q.map(i32::from),
        qp,
        offsets,
        depth,
        chroma,
        strength,
    );
    Ok((p.map(|v| v as u16), q.map(|v| v as u16)))
}

fn filter(
    p: [i32; 4],
    q: [i32; 4],
    qp: i32,
    offsets: [i32; 2],
    depth: u8,
    chroma: bool,
    strength: u8,
) -> ([i32; 4], [i32; 4]) {
    if strength == 0 {
        return (p, q);
    }
    let mut a = p;
    let mut b = q;
    let index = (qp + offsets[0]).clamp(0, 51) as usize;
    let alpha = ALPHA[index] << (depth - 8);
    let beta = BETA[(qp + offsets[1]).clamp(0, 51) as usize] << (depth - 8);
    if (p[0] - q[0]).abs() >= alpha || (p[1] - p[0]).abs() >= beta || (q[1] - q[0]).abs() >= beta {
        return (a, b);
    }
    let ap = (p[2] - p[0]).abs() < beta;
    let aq = (q[2] - q[0]).abs() < beta;
    if strength == 4 {
        let strong = !chroma && (p[0] - q[0]).abs() < (alpha >> 2) + 2;
        if strong && ap {
            a[0] = (p[2] + 2 * p[1] + 2 * p[0] + 2 * q[0] + q[1] + 4) >> 3;
            a[1] = (p[2] + p[1] + p[0] + q[0] + 2) >> 2;
            a[2] = (2 * p[3] + 3 * p[2] + p[1] + p[0] + q[0] + 4) >> 3;
        } else {
            a[0] = (2 * p[1] + p[0] + q[1] + 2) >> 2;
        }
        if strong && aq {
            b[0] = (q[2] + 2 * q[1] + 2 * q[0] + 2 * p[0] + p[1] + 4) >> 3;
            b[1] = (q[2] + q[1] + q[0] + p[0] + 2) >> 2;
            b[2] = (2 * q[3] + 3 * q[2] + q[1] + q[0] + p[0] + 4) >> 3;
        } else {
            b[0] = (2 * q[1] + q[0] + p[1] + 2) >> 2;
        }
    } else {
        let tc0 = match strength {
            1 => TC1[index],
            2 => TC2[index],
            _ => TC3[index],
        } << (depth - 8);
        let tc = tc0
            + if chroma {
                1
            } else {
                i32::from(ap) + i32::from(aq)
            };
        let delta = (((q[0] - p[0]) * 4 + p[1] - q[1] + 4) >> 3).clamp(-tc, tc);
        a[0] = (p[0] + delta).clamp(0, (1 << depth) - 1);
        b[0] = (q[0] - delta).clamp(0, (1 << depth) - 1);
        let avg = (p[0] + q[0] + 1) >> 1;
        if !chroma && ap {
            a[1] += ((p[2] + avg - 2 * p[1]) >> 1).clamp(-tc0, tc0);
        }
        if !chroma && aq {
            b[1] += ((q[2] + avg - 2 * q[1]) >> 1).clamp(-tc0, tc0);
        }
    }
    (a, b)
}

/// Called after reconstruction so intra prediction sees unfiltered neighbours.
/// QPs are component QP values without bit-depth offsets, in macroblock order.
pub(super) fn intra_plane(
    plane: &mut [u16],
    width: usize,
    height: usize,
    qps: &[i32],
    depth: u8,
    offsets: [i32; 2],
    chroma: bool,
    eight: &[u8],
) -> crate::Result<()> {
    let size = if chroma { 8 } else { 16 };
    if width == 0
        || width % size != 0
        || height % size != 0
        || qps.len() != width / size * (height / size)
        || eight.len() != qps.len()
    {
        return Err(crate::invalid("invalid intra deblocking metadata"));
    }
    let mb_width = width / size;
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(qps.len())
        .map_err(|_| crate::invalid("cannot allocate deblocking grid"))?;
    for (index, &qp) in qps.iter().enumerate() {
        let mut block = MacroblockEdges {
            strengths: [[[3; 4]; 4]; 2],
            qp: [[qp; 4]; 2],
            offsets,
            transform8: eight[index] != 0,
        };
        for direction in 0..2 {
            block.strengths[direction][0] = [4; 4];
            let neighbour = if direction == 0 {
                if index % mb_width == 0 {
                    None
                } else {
                    Some(index - 1)
                }
            } else {
                index.checked_sub(mb_width)
            };
            if let Some(n) = neighbour {
                block.qp[direction][0] = (qp + qps[n] + 1) >> 1;
            }
        }
        blocks.push(block);
    }
    inter_plane(plane, width, height, depth, chroma, &blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strong_and_normal_edges_and_threshold_equality() {
        assert_eq!(
            filter([100; 4], [104; 4], 40, [0; 2], 8, false, 4),
            ([102, 101, 101, 100], [103, 103, 104, 104])
        );
        assert_eq!(
            filter([100; 4], [104; 4], 40, [0; 2], 8, false, 3),
            ([102, 101, 100, 100], [102, 103, 104, 104])
        );
        assert_eq!(
            filter([100; 4], [104; 4], 40, [0; 2], 8, true, 4),
            ([101, 100, 100, 100], [103, 104, 104, 104])
        );
        assert_eq!(
            filter([100; 4], [104; 4], 16, [0; 2], 8, false, 4),
            ([100; 4], [104; 4])
        );
    }
}

#[cfg(test)]
mod inter_tests {
    use super::*;
    #[test]
    fn weak_edges_use_distinct_clipping_thresholds() {
        let p = [100; 4];
        let q = [120; 4];
        assert_eq!(
            filter_samples(p, q, 40, [0; 2], 8, false, 0).unwrap(),
            (p, q)
        );
        for (bs, limit) in [(1, 6), (2, 7), (3, 9)] {
            let (a, b) = filter_samples(p, q, 40, [0; 2], 8, false, bs).unwrap();
            assert_eq!(a[0], 100 + limit.min(8));
            assert_eq!(b[0], 120 - limit.min(8));
            let (a, b) = filter_samples(p, q, 40, [0; 2], 8, true, bs).unwrap();
            assert_eq!(a[0], 100 + limit - 1);
            assert_eq!(b[0], 120 - limit + 1);
            assert_eq!(a[1], 100);
            assert_eq!(b[1], 120);
        }
        assert!(filter_samples(p, q, 40, [0; 2], 7, false, 1).is_err());
        assert!(filter_samples(p, q, 40, [0; 2], 8, false, 5).is_err());
    }
}

/// Edge metadata for one progressive macroblock. Directions are vertical then
/// horizontal; edges and their four luma 4-sample segments are spatially ordered.
/// Slice boundaries disabled by the slice header must have zero strengths.
pub struct MacroblockEdges {
    pub strengths: [[[u8; 4]; 4]; 2],
    /// Rounded average component QP for each edge, without bit-depth offsets.
    pub qp: [[i32; 4]; 2],
    pub offsets: [i32; 2],
    pub transform8: bool,
}
/// Filter a packed progressive luma or 4:2:0 chroma plane in macroblock order.
/// Chroma uses luma edges 0 and 2, with two chroma samples per strength segment.
/// Metadata and input samples are checked before any in-place changes.
pub fn inter_plane(
    plane: &mut [u16],
    width: usize,
    height: usize,
    depth: u8,
    chroma: bool,
    blocks: &[MacroblockEdges],
) -> crate::Result<()> {
    let size = if chroma { 8 } else { 16 };
    if width == 0
        || height == 0
        || width % size != 0
        || height % size != 0
        || width.checked_mul(height) != Some(plane.len())
        || !(8..=14).contains(&depth)
        || blocks.len() != width / size * (height / size)
    {
        return Err(crate::invalid("invalid AVC deblocking plane geometry"));
    }
    let max = (1u16 << depth) - 1;
    if plane.iter().any(|&p| p > max)
        || blocks.iter().any(|b| {
            b.strengths.iter().flatten().flatten().any(|&s| s > 4)
                || b.qp.iter().flatten().any(|q| !(-36..=51).contains(q))
                || b.offsets.iter().any(|o| !(-12..=12).contains(o))
        })
    {
        return Err(crate::invalid("invalid AVC deblocking samples or metadata"));
    }
    let mb_width = width / size;
    for (mb, meta) in blocks.iter().enumerate() {
        let (mx, my) = (mb % mb_width, mb / mb_width);
        for direction in 0..2 {
            let vertical = direction == 0;
            for edge in 0..4 {
                if (chroma || meta.transform8) && edge % 2 != 0 {
                    continue;
                }
                if edge == 0 && if vertical { mx == 0 } else { my == 0 } {
                    continue;
                }
                let offset = edge * size / 4;
                let step = if vertical { 1 } else { width };
                for line in 0..size {
                    let strength = meta.strengths[direction][edge][line / (size / 4)];
                    if strength == 0 {
                        continue;
                    }
                    let x = mx * size + if vertical { offset } else { line };
                    let y = my * size + if vertical { line } else { offset };
                    let at = y * width + x;
                    let p = std::array::from_fn(|i| i32::from(plane[at - (i + 1) * step]));
                    let q = std::array::from_fn(|i| i32::from(plane[at + i * step]));
                    let (p, q) = filter(
                        p,
                        q,
                        meta.qp[direction][edge],
                        meta.offsets,
                        depth,
                        chroma,
                        strength,
                    );
                    for i in 0..3 {
                        plane[at - (i + 1) * step] = p[i] as u16;
                        plane[at + i * step] = q[i] as u16;
                    }
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod traversal_tests {
    use super::*;
    #[test]
    fn segment_strength_and_invalid_metadata_atomicity() {
        for chroma in [false, true] {
            let size = if chroma { 8 } else { 16 };
            let width = size * 2;
            let mut plane: Vec<_> = (0..size)
                .flat_map(|_| (0..width).map(|x| if x < size { 100 } else { 120 }))
                .collect();
            let mut blocks: Vec<_> = (0..2)
                .map(|_| MacroblockEdges {
                    strengths: [[[0; 4]; 4]; 2],
                    qp: [[40; 4]; 2],
                    offsets: [0; 2],
                    transform8: false,
                })
                .collect();
            blocks[1].strengths[0][0] = [0, 1, 2, 3];
            let original = plane.clone();
            blocks[1].qp[1][3] = 999;
            assert!(inter_plane(&mut plane, width, size, 8, chroma, &blocks).is_err());
            assert_eq!(plane, original);
            blocks[1].qp[1][3] = 40;
            inter_plane(&mut plane, width, size, 8, chroma, &blocks).unwrap();
            for segment in 0..4 {
                let delta = if chroma { [0, 5, 6, 8] } else { [0, 6, 7, 8] }[segment];
                let at = segment * (size / 4) * width + size;
                assert_eq!(plane[at - 1], 100 + delta);
                assert_eq!(plane[at], 120 - delta);
            }
        }
    }
}
