//! Intra 4:2:0 transform-tree syntax and chroma ownership.
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub log2_cu: u8,
    pub log2_min_transform: u8,
    pub log2_max_transform: u8,
    /// MaxTrafoDepth, including the IntraSplitFlag increment.
    pub max_depth: u8,
    pub intra_split: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    pub origin: [u32; 2],
    pub log2_size: u8,
    pub depth: u8,
    /// Y/Cb/Cr CBFs. In a 4x4 luma leaf, chroma flags belong to its 8x8 parent.
    pub coded: [bool; 3],
    /// Chroma residuals are read only at the last 4x4 child of an 8x8 parent.
    pub owns_chroma: bool,
    pub chroma_origin: [u32; 2],
    pub log2_chroma_size: u8,
}

/// Decode leaves synchronously in Z order using the same entropy stream.
/// The callback must consume QP syntax and residual data before traversal resumes.
/// Discard all partial state on error; this function does not rewind CABAC.
pub fn read_intra<B: ResidualBins>(
    bins: &mut B,
    origin: [u32; 2],
    config: Config,
    mut leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    let c = config;
    if !(3..=6).contains(&c.log2_cu)
        || !(2..=5).contains(&c.log2_min_transform)
        || !(c.log2_min_transform..=5).contains(&c.log2_max_transform)
        || c.log2_min_transform > c.log2_cu
        || c.max_depth > 5
        || (c.intra_split && (c.max_depth == 0 || c.log2_cu <= c.log2_min_transform))
    {
        return Err(invalid("invalid HEVC intra transform-tree configuration"));
    }
    let side = 1u32 << c.log2_cu;
    if origin
        .iter()
        .any(|&v| v % side != 0 || v.checked_add(side).is_none())
    {
        return Err(invalid("invalid HEVC transform-tree origin"));
    }
    fn walk<B: ResidualBins>(
        b: &mut B,
        c: Config,
        p: [u32; 2],
        base: [u32; 2],
        log: u8,
        depth: u8,
        index: u8,
        parent: [bool; 2],
        leaf: &mut impl FnMut(&mut B, Unit) -> Result<()>,
    ) -> Result<()> {
        let force = log > c.log2_max_transform || (c.intra_split && depth == 0);
        let split = if force {
            true
        } else if log > c.log2_min_transform && depth < c.max_depth {
            b.decision(Syntax::SplitTransform, usize::from(5 - log))?
        } else {
            false
        };
        let mut chroma = parent;
        if log > 2 {
            for flag in &mut chroma {
                *flag = if depth == 0 || *flag {
                    b.decision(Syntax::CbfChroma, usize::from(depth))?
                } else {
                    false
                };
            }
        }
        if split {
            let half = 1u32 << (log - 1);
            for (i, [dx, dy]) in [[0, 0], [1, 0], [0, 1], [1, 1]].into_iter().enumerate() {
                walk(
                    b,
                    c,
                    [p[0] + dx * half, p[1] + dy * half],
                    p,
                    log - 1,
                    depth + 1,
                    i as u8,
                    chroma,
                    leaf,
                )?;
            }
            Ok(())
        } else {
            let y = b.decision(Syntax::CbfLuma, usize::from(depth == 0))?;
            let cp = if log == 2 { base } else { p };
            leaf(
                b,
                Unit {
                    origin: p,
                    log2_size: log,
                    depth,
                    coded: [y, chroma[0], chroma[1]],
                    owns_chroma: log > 2 || index == 3,
                    chroma_origin: [cp[0] / 2, cp[1] / 2],
                    log2_chroma_size: (log - 1).max(2),
                },
            )
        }
    }
    walk(
        bins, c, origin, origin, c.log2_cu, 0, 0, [false; 2], &mut leaf,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Bins(VecDeque<(Syntax, usize, bool)>);
    impl ResidualBins for Bins {
        fn decision(&mut self, s: Syntax, c: usize) -> Result<bool> {
            let (expected, context, value) = self
                .0
                .pop_front()
                .ok_or_else(|| invalid("truncated transform tree"))?;
            assert_eq!(
                std::mem::discriminant(&s),
                std::mem::discriminant(&expected)
            );
            assert_eq!(c, context);
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("unexpected bypass")
        }
    }
    #[test]
    fn split_four_by_four_defers_parent_chroma_to_last_child() {
        let mut b = Bins(VecDeque::from([
            (Syntax::CbfChroma, 0, true),
            (Syntax::CbfChroma, 0, false),
            (Syntax::CbfLuma, 0, true),
            (Syntax::CbfLuma, 0, false),
            (Syntax::CbfLuma, 0, false),
            (Syntax::CbfLuma, 0, true),
        ]));
        let c = Config {
            log2_cu: 3,
            log2_min_transform: 2,
            log2_max_transform: 5,
            max_depth: 1,
            intra_split: true,
        };
        let mut units = Vec::new();
        read_intra(&mut b, [8, 16], c, |_, u| {
            units.push(u);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            units.iter().map(|u| u.origin).collect::<Vec<_>>(),
            [[8, 16], [12, 16], [8, 20], [12, 20]]
        );
        for (i, u) in units.iter().enumerate() {
            assert_eq!(u.chroma_origin, [4, 8]);
            assert_eq!(u.log2_chroma_size, 2);
            assert_eq!(u.owns_chroma, i == 3);
            assert!(u.coded[1]);
            assert!(!u.coded[2]);
        }
        assert!(b.0.is_empty());
    }
    #[test]
    fn unsplit_root_contexts_and_truncation() {
        let c = Config {
            log2_cu: 4,
            log2_min_transform: 2,
            log2_max_transform: 5,
            max_depth: 2,
            intra_split: false,
        };
        let mut b = Bins(VecDeque::from([
            (Syntax::SplitTransform, 1, false),
            (Syntax::CbfChroma, 0, false),
            (Syntax::CbfChroma, 0, true),
            (Syntax::CbfLuma, 1, false),
        ]));
        read_intra(&mut b, [0, 0], c, |_, u| {
            assert_eq!(u.coded, [false, false, true]);
            assert!(u.owns_chroma);
            assert_eq!(u.log2_chroma_size, 3);
            Ok(())
        })
        .unwrap();
        assert!(b.0.is_empty());
        assert!(read_intra(&mut b, [0, 0], c, |_, _| Ok(())).is_err());
        assert!(read_intra(&mut b, [1, 0], c, |_, _| Ok(())).is_err());
    }
}
