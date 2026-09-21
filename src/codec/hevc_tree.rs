//! Coding-quadtree traversal with CABAC split contexts and picture-edge inference.
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub x: u32,
    pub y: u32,
    pub log2_size: u8,
    pub depth: u8,
}

pub trait Visitor<B: ResidualBins> {
    /// Left/top depths at (x-1,y)/(x,y-1), excluding unavailable slice/tile
    /// neighbours. Publish completed CU depth before the next sibling is visited.
    fn neighbouring_depths(&self, node: Node) -> [Option<u8>; 2];
    /// Called after reading/inferencing the split flag, before child traversal.
    /// Reset QP-delta/chroma-offset coding state here when node size requires it.
    fn enter(&mut self, node: Node, split: bool) -> Result<()>;
    /// Decode a CU using the same CABAC stream. Failure aborts the CTU; caller
    /// must discard partial picture/metadata and must not resume that slice.
    fn leaf(&mut self, bins: &mut B, node: Node) -> Result<()>;
}

/// Traverse one aligned CTU. Picture dimensions must be multiples of minimum CU
/// size; out-of-picture quadrants consume no syntax and produce no leaves.
pub fn read_ctu<B: ResidualBins>(
    bins: &mut B,
    visitor: &mut impl Visitor<B>,
    picture: [u32; 2],
    origin: [u32; 2],
    log2_ctu: u8,
    log2_min_cu: u8,
) -> Result<()> {
    if !(4..=6).contains(&log2_ctu) || !(3..=log2_ctu).contains(&log2_min_cu) {
        return Err(invalid("invalid HEVC coding-tree sizes"));
    }
    let min = 1u32 << log2_min_cu;
    let ctu = 1u32 << log2_ctu;
    if (0..2).any(|i| {
        picture[i] == 0 || picture[i] % min != 0 || origin[i] >= picture[i] || origin[i] % ctu != 0
    }) {
        return Err(invalid("invalid HEVC coding-tree geometry"));
    }
    fn walk<B: ResidualBins>(
        b: &mut B,
        v: &mut impl Visitor<B>,
        picture: [u32; 2],
        min: u8,
        node: Node,
    ) -> Result<()> {
        let side = 1u64 << node.log2_size;
        let inside = u64::from(node.x) + side <= u64::from(picture[0])
            && u64::from(node.y) + side <= u64::from(picture[1]);
        let split = if node.log2_size == min {
            false
        } else if !inside {
            true
        } else {
            let context = v
                .neighbouring_depths(node)
                .into_iter()
                .filter(|d| d.is_some_and(|depth| depth > node.depth))
                .count();
            b.decision(Syntax::SplitCu, context)?
        };
        v.enter(node, split)?;
        if split {
            let log = node.log2_size - 1;
            let half = 1u64 << log;
            for [dx, dy] in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                let x = u64::from(node.x) + dx * half;
                let y = u64::from(node.y) + dy * half;
                if x < u64::from(picture[0]) && y < u64::from(picture[1]) {
                    walk(
                        b,
                        v,
                        picture,
                        min,
                        Node {
                            x: x as u32,
                            y: y as u32,
                            log2_size: log,
                            depth: node.depth + 1,
                        },
                    )?;
                }
            }
            Ok(())
        } else {
            v.leaf(b, node)
        }
    }
    walk(
        bins,
        visitor,
        picture,
        log2_min_cu,
        Node {
            x: origin[0],
            y: origin[1],
            log2_size: log2_ctu,
            depth: 0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Bins(VecDeque<(usize, bool)>);
    impl ResidualBins for Bins {
        fn decision(&mut self, s: Syntax, context: usize) -> Result<bool> {
            assert!(matches!(s, Syntax::SplitCu));
            let (expected, value) = self
                .0
                .pop_front()
                .ok_or_else(|| invalid("truncated split"))?;
            assert_eq!(context, expected);
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("unexpected bypass")
        }
    }
    #[derive(Default)]
    struct Field {
        leaves: Vec<Node>,
        entered: Vec<(Node, bool)>,
    }
    impl Visitor<Bins> for Field {
        fn neighbouring_depths(&self, n: Node) -> [Option<u8>; 2] {
            let find = |x: u32, y: u32| {
                self.leaves
                    .iter()
                    .find(|l| {
                        x >= l.x
                            && y >= l.y
                            && x - l.x < 1 << l.log2_size
                            && y - l.y < 1 << l.log2_size
                    })
                    .map(|l| l.depth)
            };
            [
                n.x.checked_sub(1).and_then(|x| find(x, n.y)),
                n.y.checked_sub(1).and_then(|y| find(n.x, y)),
            ]
        }
        fn enter(&mut self, n: Node, s: bool) -> Result<()> {
            self.entered.push((n, s));
            Ok(())
        }
        fn leaf(&mut self, _: &mut Bins, n: Node) -> Result<()> {
            self.leaves.push(n);
            Ok(())
        }
    }
    #[test]
    fn depth_contexts_follow_completed_siblings_in_z_order() {
        let mut bins = Bins(VecDeque::from([
            (0, true),
            (0, true),
            (1, false),
            (1, false),
            (0, false),
        ]));
        let mut field = Field::default();
        read_ctu(&mut bins, &mut field, [64, 64], [0, 0], 6, 4).unwrap();
        let actual: Vec<_> = field
            .leaves
            .iter()
            .map(|n| (n.x, n.y, n.log2_size))
            .collect();
        assert_eq!(
            actual,
            [
                (0, 0, 4),
                (16, 0, 4),
                (0, 16, 4),
                (16, 16, 4),
                (32, 0, 5),
                (0, 32, 5),
                (32, 32, 5)
            ]
        );
        assert_eq!(field.entered.len(), 9);
        assert!(bins.0.is_empty());
    }
    #[test]
    fn picture_edges_force_splits_without_consuming_bins() {
        // A bottom-right 8x8 remainder of a 72x72 picture: every split is inferred.
        let mut bins = Bins(VecDeque::new());
        let mut field = Field::default();
        read_ctu(&mut bins, &mut field, [72, 72], [64, 64], 6, 3).unwrap();
        assert_eq!(
            field.leaves,
            [Node {
                x: 64,
                y: 64,
                log2_size: 3,
                depth: 3
            }]
        );
        assert_eq!(field.entered.len(), 4);
        assert!(read_ctu(&mut bins, &mut field, [71, 72], [64, 64], 6, 3).is_err());
        assert!(read_ctu(&mut bins, &mut field, [72, 72], [8, 0], 6, 3).is_err());
        assert!(read_ctu(&mut bins, &mut field, [64, 64], [0, 0], 6, 3).is_err());
    }
}
