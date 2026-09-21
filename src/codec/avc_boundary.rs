//! Progressive-frame boundary strength, H.264 8.7.2.1.
//! Field/MBAFF edges need separate rules and are not represented by this API.
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionReference {
    /// Decoded picture identity, not the index in a slice's reference list.
    pub picture: u64,
    /// Quarter-luma-frame sample units.
    pub vector: [i16; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct BlockEdge {
    pub intra: bool,
    pub switching_slice: bool,
    /// For 8x8 transforms this is the flag of the containing 8x8 block.
    pub nonzero_luma: bool,
    pub motion: [Option<MotionReference>; 2],
}
/// Returns bS=0..4. The caller handles slice-edge suppression and absent edges.
pub fn strength(p: BlockEdge, q: BlockEdge, macroblock_edge: bool) -> Result<u8> {
    if p.intra || q.intra || p.switching_slice || q.switching_slice {
        return Ok(if macroblock_edge { 4 } else { 3 });
    }
    if p.motion.iter().all(Option::is_none) || q.motion.iter().all(Option::is_none) {
        return Err(invalid("AVC inter edge has no motion reference"));
    }
    if p.nonzero_luma || q.nonzero_luma {
        return Ok(2);
    }
    let equal = |a: Option<MotionReference>, b: Option<MotionReference>| match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.picture == b.picture
                && (0..2).all(|i| (i32::from(a.vector[i]) - i32::from(b.vector[i])).abs() < 4)
        }
        _ => false,
    };
    // Either pairing is valid, including two vectors that reference one picture.
    let direct = equal(p.motion[0], q.motion[0]) && equal(p.motion[1], q.motion[1]);
    let swapped = equal(p.motion[0], q.motion[1]) && equal(p.motion[1], q.motion[0]);
    Ok(u8::from(!direct && !swapped))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn mv(picture: u64, x: i16, y: i16) -> Option<MotionReference> {
        Some(MotionReference {
            picture,
            vector: [x, y],
        })
    }
    fn edge(motion: [Option<MotionReference>; 2]) -> BlockEdge {
        BlockEdge {
            intra: false,
            switching_slice: false,
            nonzero_luma: false,
            motion,
        }
    }
    #[test]
    fn strength_priority_and_quarter_sample_threshold() {
        let p = edge([mv(1, 0, 0), None]);
        assert_eq!(strength(p, edge([None, mv(1, 3, -3)]), true).unwrap(), 0);
        assert_eq!(strength(p, edge([None, mv(1, 4, 0)]), false).unwrap(), 1);
        assert_eq!(strength(p, edge([mv(2, 0, 0), None]), true).unwrap(), 1);
        assert_eq!(
            strength(
                p,
                BlockEdge {
                    nonzero_luma: true,
                    ..p
                },
                true
            )
            .unwrap(),
            2
        );
        assert_eq!(
            strength(p, BlockEdge { intra: true, ..p }, false).unwrap(),
            3
        );
        assert_eq!(
            strength(
                p,
                BlockEdge {
                    switching_slice: true,
                    ..p
                },
                true
            )
            .unwrap(),
            4
        );
        assert!(strength(p, edge([None, None]), false).is_err());
        assert_eq!(
            strength(
                edge([mv(1, i16::MIN, 0), None]),
                edge([mv(1, i16::MAX, 0), None]),
                false
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn bipred_pairing_and_identical_reference_ambiguity() {
        let p = edge([mv(1, 0, 0), mv(2, 8, 0)]);
        assert_eq!(
            strength(p, edge([mv(2, 9, 0), mv(1, 0, 0)]), true).unwrap(),
            0
        );
        assert_eq!(
            strength(p, edge([mv(2, 12, 0), mv(1, 0, 0)]), true).unwrap(),
            1
        );
        let same = edge([mv(1, 0, 0), mv(1, 20, 0)]);
        assert_eq!(
            strength(same, edge([mv(1, 21, 0), mv(1, 1, 0)]), false).unwrap(),
            0
        );
        assert_eq!(
            strength(same, edge([mv(1, 24, 0), mv(1, 1, 0)]), false).unwrap(),
            1
        );
        assert_eq!(strength(same, edge([mv(1, 0, 0), None]), false).unwrap(), 1);
    }
}

/// Decoded progressive macroblock state needed by the loop filter.
pub struct DecodedBlockEdges {
    /// Sixteen luma 4x4 positions in raster order. For 8x8 transforms, callers
    /// repeat each containing transform's nonzero flag at its four positions.
    pub blocks: [BlockEdge; 16],
    /// Component QPs Y/Cb/Cr, after chroma mapping, without bit-depth offsets.
    pub qp: [i32; 3],
    pub slice_id: u32,
    pub disable_filter: u8,
    pub offsets: [i32; 2],
    pub transform8: bool,
}
/// Construct metadata for one component plane. Input geometry is in macroblocks.
/// Results can be passed directly to avc_deblock::inter_plane.
pub fn picture_edges(
    input: &[DecodedBlockEdges],
    width_mbs: usize,
    component: usize,
) -> Result<Vec<super::avc_deblock::MacroblockEdges>> {
    if width_mbs == 0 || input.is_empty() || input.len() % width_mbs != 0 || component > 2 {
        return Err(invalid("invalid AVC edge-grid geometry"));
    }
    if input.iter().any(|m| {
        m.disable_filter > 2
            || m.qp.iter().any(|q| !(-36..=51).contains(q))
            || m.offsets.iter().any(|o| !(-12..=12).contains(o))
    }) {
        return Err(invalid("invalid AVC edge-grid parameters"));
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(input.len())
        .map_err(|_| invalid("cannot allocate AVC edge grid"))?;
    for (index, current) in input.iter().enumerate() {
        let mut out = super::avc_deblock::MacroblockEdges {
            strengths: [[[0; 4]; 4]; 2],
            qp: [[current.qp[component]; 4]; 2],
            offsets: current.offsets,
            transform8: current.transform8,
        };
        if current.disable_filter != 1 {
            for direction in 0..2 {
                for edge in 0..4 {
                    if (component != 0 || current.transform8) && edge % 2 != 0 {
                        continue;
                    }
                    let neighbour = if edge != 0 {
                        Some(index)
                    } else if direction == 0 {
                        if index % width_mbs == 0 {
                            None
                        } else {
                            Some(index - 1)
                        }
                    } else {
                        index.checked_sub(width_mbs)
                    };
                    let Some(neighbour) = neighbour else { continue };
                    let previous = &input[neighbour];
                    if current.disable_filter == 2 && current.slice_id != previous.slice_id {
                        continue;
                    }
                    out.qp[direction][edge] =
                        (current.qp[component] + previous.qp[component] + 1) >> 1;
                    for segment in 0..4 {
                        let q = if direction == 0 {
                            segment * 4 + edge
                        } else {
                            edge * 4 + segment
                        };
                        let p = if direction == 0 {
                            segment * 4 + if edge == 0 { 3 } else { edge - 1 }
                        } else {
                            (if edge == 0 { 3 } else { edge - 1 }) * 4 + segment
                        };
                        out.strengths[direction][edge][segment] =
                            strength(previous.blocks[p], current.blocks[q], edge == 0)?;
                    }
                }
            }
        }
        result.push(out);
    }
    Ok(result)
}
#[cfg(test)]
mod grid_tests {
    use super::*;
    fn mb(slice_id: u32) -> DecodedBlockEdges {
        DecodedBlockEdges {
            blocks: [BlockEdge {
                intra: false,
                switching_slice: false,
                nonzero_luma: false,
                motion: [
                    Some(MotionReference {
                        picture: 1,
                        vector: [0, 0],
                    }),
                    None,
                ],
            }; 16],
            qp: [20, 30, 40],
            slice_id,
            disable_filter: 0,
            offsets: [0; 2],
            transform8: false,
        }
    }
    #[test]
    fn neighbours_slice_suppression_and_component_qp() {
        let mut blocks = [mb(0), mb(1)];
        blocks[0].qp = [31, 33, 35];
        blocks[0].blocks[3].nonzero_luma = true;
        blocks[1].blocks[12].intra = true;
        let grid = picture_edges(&blocks, 2, 1).unwrap();
        assert_eq!(grid[1].qp[0][0], 32);
        assert_eq!(grid[1].strengths[0][0], [2, 0, 0, 4]);
        assert_eq!(grid[0].strengths[0][0], [0; 4]);
        blocks[1].disable_filter = 2;
        assert_eq!(
            picture_edges(&blocks, 2, 0).unwrap()[1].strengths[0][0],
            [0; 4]
        );
        blocks[1].disable_filter = 1;
        assert_eq!(
            picture_edges(&blocks, 2, 0).unwrap()[1].strengths,
            [[[0; 4]; 4]; 2]
        );
        blocks[1].disable_filter = 0;
        blocks[1].transform8 = true;
        assert_eq!(
            picture_edges(&blocks, 2, 0).unwrap()[1].strengths[0][1],
            [0; 4]
        );
    }
}
