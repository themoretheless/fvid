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

/// H.264 8.7.2.1: boundary strength when both sides are intra coded.
/// Horizontal macroblock edges involving a field use bS=3; vertical
/// macroblock edges and horizontal frame/frame edges use bS=4.
pub fn intra_strength(macroblock_edge: bool, vertical: bool, field: [bool; 2]) -> u8 {
    if macroblock_edge && (vertical || !field[0] && !field[1]) {
        4
    } else {
        3
    }
}

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

/// Filter one edge line in frame storage. `step` advances perpendicular to
/// the edge; a horizontal field edge uses twice the frame plane stride.
/// Caller derives edge topology and strengths; the complete eight-sample
/// footprint and filter parameters are validated before any mutation.
pub fn filter_line(
    plane: &mut [u16],
    at: usize,
    step: usize,
    qp: i32,
    offsets: [i32; 2],
    depth: u8,
    chroma: bool,
    strength: u8,
) -> crate::Result<()> {
    if step == 0 {
        return Err(crate::invalid("AVC deblocking sample step is zero"));
    }
    let mut pi = [0usize; 4];
    let mut qi = [0usize; 4];
    for i in 0..4 {
        pi[i] = (i + 1)
            .checked_mul(step)
            .and_then(|n| at.checked_sub(n))
            .ok_or_else(|| crate::invalid("AVC deblocking p footprint outside plane"))?;
        qi[i] = i
            .checked_mul(step)
            .and_then(|n| at.checked_add(n))
            .filter(|n| *n < plane.len())
            .ok_or_else(|| crate::invalid("AVC deblocking q footprint outside plane"))?;
    }
    let (p, q) = filter_samples(
        pi.map(|index| plane[index]),
        qi.map(|index| plane[index]),
        qp,
        offsets,
        depth,
        chroma,
        strength,
    )?;
    for i in 0..3 {
        plane[pi[i]] = p[i];
        plane[qi[i]] = q[i];
    }
    Ok(())
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
    headers: &[&super::avc_slice::SliceHeader],
    chroma: bool,
    eight: &[u8],
) -> crate::Result<()> {
    intra_plane_owned(
        plane, width, height, qps, depth, headers, chroma, eight, None,
    )
}
pub(super) fn intra_plane_owned(
    plane: &mut [u16],
    width: usize,
    height: usize,
    qps: &[i32],
    depth: u8,
    headers: &[&super::avc_slice::SliceHeader],
    chroma: bool,
    eight: &[u8],
    owners: Option<&[usize]>,
) -> crate::Result<()> {
    let size = if chroma { 8 } else { 16 };
    if width == 0
        || width % size != 0
        || height % size != 0
        || qps.len() != width / size * (height / size)
        || eight.len() != qps.len()
        || headers.is_empty()
        || headers[0].first_mb != 0
        || headers
            .windows(2)
            .any(|pair| pair[0].first_mb >= pair[1].first_mb)
        || headers
            .iter()
            .any(|h| h.first_mb as usize >= qps.len() || h.disable_deblocking_filter_idc > 2)
    {
        return Err(crate::invalid("invalid intra deblocking metadata"));
    }
    let mb_width = width / size;
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(qps.len())
        .map_err(|_| crate::invalid("cannot allocate deblocking grid"))?;
    for (index, &qp) in qps.iter().enumerate() {
        let slice = if let Some(owners) = owners {
            *owners
                .get(index)
                .filter(|id| **id < headers.len())
                .ok_or_else(|| crate::invalid("invalid intra slice owner"))?
        } else {
            headers.partition_point(|h| h.first_mb as usize <= index) - 1
        };
        let header = headers[slice];
        let mut block = MacroblockEdges {
            strengths: [[[3; 4]; 4]; 2],
            qp: [[qp; 4]; 2],
            offsets: [header.alpha_offset, header.beta_offset],
            transform8: eight[index] != 0,
        };
        for direction in 0..2 {
            block.strengths[direction][0] =
                [intra_strength(true, direction == 0, [header.field_pic; 2]); 4];
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
                if header.disable_deblocking_filter_idc == 2
                    && if let Some(owners) = owners {
                        owners[n] != slice
                    } else {
                        n < header.first_mb as usize
                    }
                {
                    block.strengths[direction][0] = [0; 4];
                }
            }
        }
        if header.disable_deblocking_filter_idc == 1 {
            block.strengths = [[[0; 4]; 4]; 2];
        }
        blocks.push(block);
    }
    inter_plane(plane, width, height, depth, chroma, &blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_line_filters_only_its_parity_and_rejects_invalid_footprints_atomically() {
        let mut plane = [200u16; 24];
        for i in (0..24).step_by(2) {
            plane[i] = if i < 12 { 100 } else { 104 };
        }
        filter_line(&mut plane, 12, 2, 40, [0; 2], 8, false, 4).unwrap();
        assert_eq!(
            [plane[10], plane[8], plane[6], plane[4]],
            [102, 101, 101, 100]
        );
        assert_eq!(
            [plane[12], plane[14], plane[16], plane[18]],
            [103, 103, 104, 104]
        );
        assert!((1..24).step_by(2).all(|i| plane[i] == 200));
        let saved = plane;
        for (at, step) in [(1, 2), (22, 2), (12, 0), (12, usize::MAX)] {
            assert!(filter_line(&mut plane, at, step, 40, [0; 2], 8, false, 4).is_err());
            assert_eq!(plane, saved);
        }
    }
    #[test]
    fn intra_field_horizontal_boundaries_use_normal_filter_strength() {
        for fields in [[false, false], [false, true], [true, false], [true, true]] {
            assert_eq!(intra_strength(true, true, fields), 4);
            assert_eq!(intra_strength(false, false, fields), 3);
            let strength = intra_strength(true, false, fields);
            let (p, q) =
                filter_samples([100; 4], [104; 4], 40, [0; 2], 8, false, strength).unwrap();
            if fields == [false, false] {
                assert_eq!((p, q), ([102, 101, 101, 100], [103, 103, 104, 104]));
            } else {
                assert_eq!((p, q), ([102, 101, 100, 100], [102, 103, 104, 104]));
            }
        }
    }
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
#[derive(Clone)]
pub struct MacroblockEdges {
    pub strengths: [[[u8; 4]; 4]; 2],
    /// Rounded average component QP for each edge, without bit-depth offsets.
    pub qp: [[i32; 4]; 2],
    pub offsets: [i32; 2],
    pub transform8: bool,
}
/// Intra-only MBAFF metadata in pair address order. Component QP values exclude
/// the bit-depth offset; PCM callers supply zero component QPs.
#[derive(Clone, Copy)]
pub struct MbaffIntraBlock {
    pub qp: [i32; 3],
    pub field: bool,
    pub transform8: bool,
    pub slice: usize,
    pub disable: u8,
    pub offsets: [i32; 2],
}
/// Filter one 4:2:0 component after all intra reconstruction has finished.
/// Pair address order and vertical-before-horizontal order are significant.
pub fn mbaff_intra_plane(
    plane: &mut [u16],
    width: usize,
    height: usize,
    depth: u8,
    component: usize,
    blocks: &[MbaffIntraBlock],
) -> crate::Result<()> {
    mbaff_plane(
        plane,
        width,
        height,
        depth,
        component,
        blocks.len(),
        |address| blocks[address],
        |p, _, q, _, external, vertical| {
            Ok(intra_strength(
                external,
                vertical,
                [blocks[p].field, blocks[q].field],
            ))
        },
    )
}
/// Complete address-local inter/intra state for MBAFF loop filtering.
pub struct MbaffBlockEdges {
    pub edges: super::avc_boundary::DecodedBlockEdges,
    pub field: bool,
}
/// Filter one complete packed 4:2:0 component using pair-aware boundary ownership.
pub fn mbaff_inter_plane(
    plane: &mut [u16],
    width: usize,
    height: usize,
    depth: u8,
    component: usize,
    blocks: &[MbaffBlockEdges],
) -> crate::Result<()> {
    if blocks.iter().any(|b| {
        b.edges.blocks.iter().any(|cell| {
            !cell.intra && !cell.switching_slice && cell.motion.iter().all(Option::is_none)
        })
    }) {
        return Err(crate::invalid(
            "MBAFF inter deblocking block has no reference",
        ));
    }
    mbaff_plane(
        plane,
        width,
        height,
        depth,
        component,
        blocks.len(),
        |address| {
            let b = &blocks[address];
            MbaffIntraBlock {
                qp: b.edges.qp,
                field: b.field,
                transform8: b.edges.transform8,
                slice: b.edges.slice_id as usize,
                disable: b.edges.disable_filter,
                offsets: b.edges.offsets,
            }
        },
        |p, pl, q, ql, external, vertical| {
            super::avc_boundary::strength_mbaff(
                blocks[p].edges.blocks[pl[1] / 4 * 4 + pl[0] / 4],
                blocks[q].edges.blocks[ql[1] / 4 * 4 + ql[0] / 4],
                external,
                vertical,
                [blocks[p].field, blocks[q].field],
            )
        },
    )
}
fn mbaff_plane(
    plane: &mut [u16],
    width: usize,
    height: usize,
    depth: u8,
    component: usize,
    count: usize,
    metadata: impl Fn(usize) -> MbaffIntraBlock,
    strength: impl Fn(usize, [usize; 2], usize, [usize; 2], bool, bool) -> crate::Result<u8>,
) -> crate::Result<()> {
    let chroma = component != 0;
    let size = if chroma { 8 } else { 16 };
    if component > 2
        || width == 0
        || height == 0
        || width % size != 0
        || height % (2 * size) != 0
        || width.checked_mul(height) != Some(plane.len())
        || !(8..=14).contains(&depth)
        || count != width / size * (height / size)
        || (0..count).any(|address| {
            let b = metadata(address);
            b.disable > 2
                || b.qp.iter().any(|q| !(-36..=51).contains(q))
                || b.offsets.iter().any(|v| !(-12..=12).contains(v))
        })
        || (0..count / 2).any(|pair| metadata(pair * 2).field != metadata(pair * 2 + 1).field)
    {
        return Err(crate::invalid("invalid MBAFF intra deblocking metadata"));
    }
    let sub = if chroma { [2, 2] } else { [1, 1] };
    let w = width / size;
    let h = height / size;
    let pair_field = |pair: usize| (pair * 2 < count).then(|| metadata(pair * 2).field);
    for address in 0..count {
        let block = metadata(address);
        let layout = super::avc_mbaff::layout(address, w, h, true, block.field, sub)?;
        if block.disable == 1 {
            continue;
        }
        for vertical in [true, false] {
            for edge in 0..4 {
                if (chroma || block.transform8) && edge % 2 != 0 {
                    continue;
                }
                let offset = edge * size / 4;
                // A frame top below a field pair is filtered on both parities.
                let mixed_top = !vertical
                    && edge == 0
                    && !block.field
                    && super::avc_mbaff::deblock_line(
                        address, w, h, true, sub, false, 0, 0, false, pair_field,
                    )?
                    .is_some_and(|line| metadata(line.p_owner.0).field);
                for extra in 0..if mixed_top { 2 } else { 1 } {
                    for line in 0..size {
                        let Some(geometry) = super::avc_mbaff::deblock_line(
                            address,
                            w,
                            h,
                            true,
                            sub,
                            vertical,
                            offset + extra,
                            line,
                            mixed_top,
                            pair_field,
                        )?
                        else {
                            continue;
                        };
                        let neighbour = metadata(geometry.p_owner.0);
                        if block.disable == 2 && neighbour.slice != block.slice {
                            continue;
                        }
                        let qp = (block.qp[component] + neighbour.qp[component] + 1) >> 1;
                        let pl = geometry.p_owner.1;
                        let ql = [
                            geometry.q[0] - layout.origin[0],
                            (geometry.q[1] - layout.origin[1]) / layout.row_step,
                        ];
                        let strength = strength(
                            geometry.p_owner.0,
                            [pl[0] * sub[0], pl[1] * sub[1]],
                            address,
                            [ql[0] * sub[0], ql[1] * sub[1]],
                            edge == 0,
                            vertical,
                        )?;
                        filter_line(
                            plane,
                            geometry.at,
                            geometry.step,
                            qp,
                            block.offsets,
                            depth,
                            chroma,
                            strength,
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
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
    // Reconstruction clips every sample to the bit depth; scanning the whole
    // plane again here cost a visible share of decode time.
    debug_assert!(plane.iter().all(|&p| p <= (1u16 << depth) - 1));
    if blocks.iter().any(|b| {
        b.strengths.iter().flatten().flatten().any(|&s| s > 4)
            || b.qp.iter().flatten().any(|q| !(-36..=51).contains(q))
            || b.offsets.iter().any(|o| !(-12..=12).contains(o))
    }) {
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
                    filter_line(
                        plane,
                        at,
                        step,
                        meta.qp[direction][edge],
                        meta.offsets,
                        depth,
                        chroma,
                        strength,
                    )?;
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
    fn mixed_inter_walker_preserves_intra_filtering_and_rejects_missing_motion_before_mutation() {
        use super::super::avc_boundary::{BlockEdge, DecodedBlockEdges, MotionReference};
        for component in 0..3 {
            let side = if component == 0 { 16 } else { 8 };
            for depth in [8, 10, 12, 14] {
                let scale = 1u16 << (depth - 8);
                for modes in [[false, false], [true, true], [false, true], [true, false]] {
                    let intra: Vec<_> = (0..4)
                        .map(|address| MbaffIntraBlock {
                            qp: [40; 3],
                            field: modes[address / 2],
                            transform8: false,
                            slice: 0,
                            disable: 0,
                            offsets: [0; 2],
                        })
                        .collect();
                    let mut blocks: Vec<_> = intra
                        .iter()
                        .map(|b| MbaffBlockEdges {
                            field: b.field,
                            edges: DecodedBlockEdges {
                                blocks: [BlockEdge {
                                    intra: true,
                                    switching_slice: false,
                                    nonzero_luma: false,
                                    motion: [None; 2],
                                }; 16],
                                qp: b.qp,
                                slice_id: 0,
                                disable_filter: b.disable,
                                offsets: b.offsets,
                                transform8: b.transform8,
                            },
                        })
                        .collect();
                    let input: Vec<_> = (0..side * 2)
                        .flat_map(|y| {
                            (0..side * 2).map(move |x| {
                                (100 + if x >= side { 4 } else { 0 }
                                    + if y >= side { 4 } else { 0 })
                                    * scale
                            })
                        })
                        .collect();
                    let mut expected = input.clone();
                    let mut actual = input.clone();
                    mbaff_intra_plane(&mut expected, side * 2, side * 2, depth, component, &intra)
                        .unwrap();
                    mbaff_inter_plane(&mut actual, side * 2, side * 2, depth, component, &blocks)
                        .unwrap();
                    assert_eq!(actual, expected);
                    for b in &mut blocks {
                        for cell in &mut b.edges.blocks {
                            cell.intra = false;
                            cell.nonzero_luma = true;
                            cell.motion = [
                                Some(MotionReference {
                                    picture: 1,
                                    vector: [0; 2],
                                }),
                                None,
                            ];
                        }
                    }
                    blocks[3].edges.blocks[15].motion = [None; 2];
                    let mut rejected = input.clone();
                    assert!(
                        mbaff_inter_plane(
                            &mut rejected,
                            side * 2,
                            side * 2,
                            depth,
                            component,
                            &blocks
                        )
                        .is_err()
                    );
                    assert_eq!(rejected, input);
                }
            }
        }
    }
    #[test]
    fn mixed_horizontal_top_filters_both_parities_with_normal_strength() {
        let input: Vec<u16> = (0..64)
            .flat_map(|y| (0..16).map(move |_| if y < 32 { 100 } else { 104 }))
            .collect();
        let mut blocks = vec![
            MbaffIntraBlock {
                qp: [40; 3],
                field: false,
                transform8: true,
                slice: 0,
                disable: 0,
                offsets: [0; 2]
            };
            4
        ];
        blocks[0].field = true;
        blocks[1].field = true;
        blocks[0].qp = [16; 3];
        let mut expected = input.clone();
        for parity in 0..2 {
            for x in 0..16 {
                filter_line(
                    &mut expected,
                    (32 + parity) * 16 + x,
                    32,
                    if parity == 0 { 28 } else { 40 },
                    [0; 2],
                    8,
                    false,
                    3,
                )
                .unwrap();
            }
        }
        let mut actual = input.clone();
        mbaff_intra_plane(&mut actual, 16, 64, 8, 0, &blocks).unwrap();
        assert_eq!(actual, expected);
        blocks[2].slice = 1;
        blocks[3].slice = 1;
        blocks[2].disable = 2;
        blocks[3].disable = 2;
        let mut actual = input.clone();
        mbaff_intra_plane(&mut actual, 16, 64, 8, 0, &blocks).unwrap();
        assert_eq!(actual, input);
    }
    #[test]
    fn mixed_vertical_boundary_uses_each_frame_owner_qp_and_slice() {
        let input: Vec<u16> = (0..32)
            .flat_map(|_| (0..32).map(|x| if x < 16 { 100 } else { 104 }))
            .collect();
        let mut blocks = vec![
            MbaffIntraBlock {
                qp: [40; 3],
                field: false,
                transform8: true,
                slice: 0,
                disable: 0,
                offsets: [0; 2]
            };
            4
        ];
        blocks[0].qp = [16; 3];
        blocks[2].field = true;
        blocks[3].field = true;
        let mut expected = input.clone();
        for row in 0..32 {
            filter_line(
                &mut expected,
                row * 32 + 16,
                1,
                if row < 16 { 28 } else { 40 },
                [0; 2],
                8,
                false,
                4,
            )
            .unwrap();
        }
        let mut actual = input.clone();
        mbaff_intra_plane(&mut actual, 32, 32, 8, 0, &blocks).unwrap();
        assert_eq!(actual, expected);
        blocks[0].slice = 1;
        blocks[2].disable = 2;
        blocks[3].disable = 2;
        let mut expected = input.clone();
        for row in 16..32 {
            filter_line(&mut expected, row * 32 + 16, 1, 40, [0; 2], 8, false, 4).unwrap();
        }
        let mut actual = input;
        mbaff_intra_plane(&mut actual, 32, 32, 8, 0, &blocks).unwrap();
        assert_eq!(actual, expected);
    }
    #[test]
    fn mbaff_intra_traversal_matches_separate_field_planes() {
        for component in 0..3 {
            let size = if component == 0 { 16 } else { 8 };
            let width = 2 * size;
            let height = 2 * size;
            let input: Vec<u16> = (0..height)
                .flat_map(|y| {
                    (0..width)
                        .map(move |x| 80 + (x / 4 * 3 + y / 8 * 2) as u16 + (y % 2 * 24) as u16)
                })
                .collect();
            let blocks = vec![
                MbaffIntraBlock {
                    qp: [40; 3],
                    field: true,
                    transform8: false,
                    slice: 0,
                    disable: 0,
                    offsets: [0; 2]
                };
                4
            ];
            let mut actual = input.clone();
            mbaff_intra_plane(&mut actual, width, height, 8, component, &blocks).unwrap();
            let mut expected = input.clone();
            for parity in 0..2 {
                let mut field: Vec<_> = input
                    .chunks_exact(width)
                    .skip(parity)
                    .step_by(2)
                    .flatten()
                    .copied()
                    .collect();
                let mut edges = vec![
                    MacroblockEdges {
                        strengths: [[[3; 4]; 4]; 2],
                        qp: [[40; 4]; 2],
                        offsets: [0; 2],
                        transform8: false
                    };
                    2
                ];
                for edge in &mut edges {
                    edge.strengths[0][0] = [4; 4];
                }
                inter_plane(&mut field, width, size, 8, component != 0, &edges).unwrap();
                for (y, row) in field.chunks_exact(width).enumerate() {
                    expected[(y * 2 + parity) * width..(y * 2 + parity + 1) * width]
                        .copy_from_slice(row);
                }
            }
            assert_eq!(actual, expected);
            assert_ne!(actual, input);
            let mut invalid = blocks.clone();
            invalid[3].field = false;
            let mut unchanged = input.clone();
            assert!(
                mbaff_intra_plane(&mut unchanged, width, height, 8, component, &invalid).is_err()
            );
            assert_eq!(unchanged, input);
            for block in &mut invalid {
                block.field = true;
                block.disable = 1;
            }
            mbaff_intra_plane(&mut unchanged, width, height, 8, component, &invalid).unwrap();
            assert_eq!(unchanged, input);
        }
    }
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

#[cfg(test)]
mod slice_boundary_tests {
    use super::*;
    #[test]
    fn intra_filter_respects_slice_boundary_and_disabled_slice() {
        use crate::{
            codec::{
                avc::{Pps, Sps},
                avc_slice::SliceHeader,
                config::{AvcConfig, NalUnits},
            },
            container::mp4::Mp4Reader,
        };
        let mut reader = Mp4Reader::open(
            std::io::Cursor::new(include_bytes!(
                "../../tests/fixtures/playback-errors/avc-multislice-ipb.mp4"
            )),
            Default::default(),
        )
        .unwrap();
        let config = reader.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut packet = Vec::new();
        reader.read_packet(0, 0, &mut packet).unwrap();
        let mut headers: Vec<_> = NalUnits::new(&packet, avc.length_size)
            .unwrap()
            .map(|n| n.unwrap())
            .filter(|n| matches!(n[0] & 31, 1 | 5))
            .map(|n| SliceHeader::parse(n, &sps, &pps).unwrap())
            .collect();
        headers[1].first_mb = 1;
        for h in &mut headers {
            h.disable_deblocking_filter_idc = 0;
            h.alpha_offset = 0;
            h.beta_offset = 0;
        }
        let input: Vec<_> = (0..512)
            .map(|i| if i % 32 < 16 { 100 } else { 104 })
            .collect();
        let mut enabled = input.clone();
        intra_plane(
            &mut enabled,
            32,
            16,
            &[40, 40],
            8,
            &[&headers[0], &headers[1]],
            false,
            &[0, 0],
        )
        .unwrap();
        assert_ne!(enabled, input);
        headers[1].disable_deblocking_filter_idc = 2;
        let mut boundary_disabled = input.clone();
        intra_plane(
            &mut boundary_disabled,
            32,
            16,
            &[40, 40],
            8,
            &[&headers[0], &headers[1]],
            false,
            &[0, 0],
        )
        .unwrap();
        assert_eq!(boundary_disabled, input);
        headers[1].disable_deblocking_filter_idc = 1;
        let mut slice_disabled = input.clone();
        intra_plane(
            &mut slice_disabled,
            32,
            16,
            &[40, 40],
            8,
            &[&headers[0], &headers[1]],
            false,
            &[0, 0],
        )
        .unwrap();
        assert_eq!(slice_disabled, input);
    }
}
