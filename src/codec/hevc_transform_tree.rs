//! Transform-tree syntax and chroma ownership for 4:2:0, 4:2:2 and 4:4:4.
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
    /// Second vertically adjacent Cb/Cr block for 4:2:2; false otherwise.
    pub coded_lower: [bool; 2],
    /// Chroma residuals are read only at the last 4x4 child of an 8x8 parent.
    pub owns_chroma: bool,
    pub chroma_origin: [u32; 2],
    pub log2_chroma_size: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentBlock {
    pub component: usize,
    pub origin: [u32; 2],
    pub log2_size: u8,
    pub coded: bool,
}
impl Unit {
    /// Prediction/residual blocks in entropy order: Y, Cb upper/lower, Cr upper/lower.
    pub fn component_blocks(self, chroma_format: u8)
        -> impl Iterator<Item = ComponentBlock> {
        let mut blocks = [None; 5];
        blocks[0] = Some(ComponentBlock { component: 0, origin: self.origin,
            log2_size: self.log2_size, coded: self.coded[0] });
        if self.owns_chroma {
            for component in 1..3 {
                let slot = 1 + (component - 1) * 2;
                blocks[slot] = Some(ComponentBlock { component, origin: self.chroma_origin,
                    log2_size: self.log2_chroma_size, coded: self.coded[component] });
                if chroma_format == 2 {
                    blocks[slot + 1] = Some(ComponentBlock { component,
                        origin: [self.chroma_origin[0], self.chroma_origin[1] + (1 << self.log2_chroma_size)],
                        log2_size: self.log2_chroma_size, coded: self.coded_lower[component - 1] });
                }
            }
        }
        blocks.into_iter().flatten()
    }
    pub fn coded_components(self) -> [bool; 3] {
        [self.coded[0], self.coded[1] || self.coded_lower[0], self.coded[2] || self.coded_lower[1]]
    }
}

/// Decode leaves synchronously in Z order using the same entropy stream.
/// The callback must consume QP syntax and residual data before traversal resumes.
/// Discard all partial state on error; this function does not rewind CABAC.
pub fn read_intra<B: ResidualBins>(
    bins: &mut B,
    origin: [u32; 2],
    config: Config,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    read(bins, origin, config, true, false, 1, leaf)
}
pub fn read_inter<B: ResidualBins>(
    bins: &mut B,
    origin: [u32; 2],
    config: Config,
    partitioned: bool,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    read(bins, origin, config, false, partitioned, 1, leaf)
}
/// 4:4:4 components own a residual block at every luma leaf, including 4x4.
pub fn read_intra_444<B: ResidualBins>(
    bins: &mut B, origin: [u32; 2], config: Config,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    read(bins, origin, config, true, false, 3, leaf)
}
pub fn read_inter_444<B: ResidualBins>(
    bins: &mut B, origin: [u32; 2], config: Config, partitioned: bool,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    read(bins, origin, config, false, partitioned, 3, leaf)
}
pub fn read_intra_with_chroma<B: ResidualBins>(
    bins: &mut B, origin: [u32;2], config: Config, chroma_format: u8,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    if chroma_format > 3 { return Err(crate::unsupported("unsupported HEVC transform-tree chroma format")); }
    read(bins, origin, config, true, false, chroma_format, leaf)
}
pub fn read_inter_with_chroma<B: ResidualBins>(
    bins: &mut B, origin: [u32;2], config: Config, partitioned: bool, chroma_format: u8,
    leaf: impl FnMut(&mut B, Unit) -> Result<()>,
) -> Result<()> {
    if chroma_format > 3 { return Err(crate::unsupported("unsupported HEVC transform-tree chroma format")); }
    read(bins, origin, config, false, partitioned, chroma_format, leaf)
}
fn read<B: ResidualBins>(
    bins: &mut B,
    origin: [u32; 2],
    config: Config,
    intra: bool,
    partitioned: bool,
    chroma_format: u8,
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
        intra: bool,
        partitioned: bool,
        chroma_format: u8,
        p: [u32; 2],
        base: [u32; 2],
        log: u8,
        depth: u8,
        index: u8,
        parent: [bool; 2],
        parent_lower: [bool; 2],
        leaf: &mut impl FnMut(&mut B, Unit) -> Result<()>,
    ) -> Result<()> {
        let full_chroma = chroma_format == 3;
        let force = log > c.log2_max_transform
            || (c.intra_split && depth == 0)
            || (!intra && partitioned && c.max_depth == 0 && depth == 0);
        let split = if force {
            true
        } else if log > c.log2_min_transform && depth < c.max_depth {
            b.decision(Syntax::SplitTransform, usize::from(5 - log))?
        } else {
            false
        };
        let mut chroma = parent;
        let mut lower = if log == 2 { parent_lower } else { [false; 2] };
        if (log > 2 && chroma_format != 0) || full_chroma {
            for (component, flag) in chroma.iter_mut().enumerate() {
                *flag = if depth == 0 || *flag {
                    let upper = b.decision(Syntax::CbfChroma, usize::from(depth))?;
                    if chroma_format == 2 && (!split || log == 3) {
                        lower[component] = b.decision(Syntax::CbfChroma, usize::from(depth))?;
                    }
                    upper
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
                    intra,
                    partitioned,
                    chroma_format,
                    [p[0] + dx * half, p[1] + dy * half],
                    p,
                    log - 1,
                    depth + 1,
                    i as u8,
                    chroma,
                    lower,
                    leaf,
                )?;
            }
            Ok(())
        } else {
            let y = if intra || depth != 0 || chroma.iter().chain(lower.iter()).any(|&v| v) {
                b.decision(Syntax::CbfLuma, usize::from(depth == 0))?
            } else {
                true
            };
            let cp = if log == 2 && !full_chroma { base } else { p };
            leaf(
                b,
                Unit {
                    origin: p,
                    log2_size: log,
                    depth,
                    coded: [y, chroma[0], chroma[1]],
                    coded_lower: lower,
                    owns_chroma: chroma_format != 0 && (full_chroma || log > 2 || index == 3),
                    chroma_origin: if full_chroma { cp } else { [cp[0] / 2, cp[1] >> u32::from(chroma_format == 1)] },
                    log2_chroma_size: if full_chroma { log } else { (log - 1).max(2) },
                },
            )
        }
    }
    walk(
        bins,
        c,
        intra,
        partitioned,
        chroma_format,
        origin,
        origin,
        c.log2_cu,
        0,
        0,
        [false; 2],
        [false; 2],
        &mut leaf,
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
    fn monochrome_tree_consumes_only_luma_syntax() {
        let c = Config { log2_cu: 3, log2_min_transform: 2,
            log2_max_transform: 5, max_depth: 1, intra_split: true };
        let mut b = Bins((0..4).map(|i| (Syntax::CbfLuma,0,i==3)).collect());
        let mut units = Vec::new();
        read_intra_with_chroma(&mut b,[0,0],c,0,|_,u| {units.push(u);Ok(())}).unwrap();
        assert!(b.0.is_empty());
        assert_eq!(units.len(),4);
        for (index,u) in units.iter().enumerate() {
            assert_eq!(u.coded,[index==3,false,false]);
            assert_eq!(u.coded_lower,[false;2]);
            assert!(!u.owns_chroma);
            assert_eq!(u.component_blocks(0).count(),1);
        }
        let mut b = Bins(VecDeque::new());
        read_inter_with_chroma(&mut b,[0,0],Config {max_depth:0,intra_split:false,..c},false,0,
            |_,u| { assert_eq!(u.coded,[true,false,false]);Ok(()) }).unwrap();
    }
    #[test]
    fn chroma422_unsplit_reads_cb_pair_before_cr_pair() {
        let c = Config { log2_cu: 3, log2_min_transform: 2,
            log2_max_transform: 5, max_depth: 0, intra_split: false };
        let decisions = VecDeque::from([
            (Syntax::CbfChroma, 0, false), (Syntax::CbfChroma, 0, true),
            (Syntax::CbfChroma, 0, true), (Syntax::CbfChroma, 0, false),
            (Syntax::CbfLuma, 1, false),
        ]);
        let mut b = Bins(decisions.clone());
        let mut units = Vec::new();
        read_inter_with_chroma(&mut b, [8,16], c, false, 2,
            |_, u| { units.push(u); Ok(()) }).unwrap();
        assert!(b.0.is_empty());
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].coded, [false, false, true]);
        assert_eq!(units[0].coded_lower, [true, false]);
        assert_eq!(units[0].coded_components(), [false, true, true]);
        let blocks: Vec<_> = units[0].component_blocks(2).collect();
        assert_eq!(blocks.iter().map(|b| b.component).collect::<Vec<_>>(), [0,1,1,2,2]);
        assert_eq!(blocks.iter().map(|b| b.origin).collect::<Vec<_>>(),
            [[8,16],[4,16],[4,20],[4,16],[4,20]]);
        assert_eq!(blocks.iter().map(|b| b.coded).collect::<Vec<_>>(),
            [false,false,true,true,false]);
        assert_eq!(units[0].chroma_origin, [4,16]);
        assert_eq!(units[0].log2_chroma_size, 2);
        assert!(units[0].owns_chroma);
        for cut in 0..decisions.len() {
            let mut b = Bins(decisions.iter().take(cut).copied().collect());
            assert!(read_inter_with_chroma(&mut b, [8,16], c, false, 2, |_, _| Ok(())).is_err());
        }
    }
    #[test]
    fn chroma422_split_minimum_parent_retains_both_blocks_at_last_child() {
        let c = Config { log2_cu: 3, log2_min_transform: 2,
            log2_max_transform: 5, max_depth: 1, intra_split: true };
        let mut decisions = VecDeque::from([
            (Syntax::CbfChroma, 0, true), (Syntax::CbfChroma, 0, false),
            (Syntax::CbfChroma, 0, false), (Syntax::CbfChroma, 0, true),
        ]);
        decisions.extend((0..4).map(|_| (Syntax::CbfLuma, 0, false)));
        let mut b = Bins(decisions);
        let mut units = Vec::new();
        read_intra_with_chroma(&mut b, [8,16], c, 2,
            |_, u| { units.push(u); Ok(()) }).unwrap();
        assert!(b.0.is_empty());
        assert_eq!(units.len(), 4);
        for (index,u) in units.iter().enumerate() {
            assert_eq!(u.coded, [false, true, false]);
            assert_eq!(u.coded_lower, [false, true]);
            assert_eq!(u.chroma_origin, [4,16]);
            assert_eq!(u.owns_chroma, index == 3);
            assert_eq!(u.log2_chroma_size, 2);
        }
    }
    #[test]
    fn chroma422_false_parent_suppresses_both_descendant_flags() {
        let c = Config { log2_cu: 4, log2_min_transform: 2,
            log2_max_transform: 5, max_depth: 1, intra_split: true };
        let mut decisions = VecDeque::from([
            (Syntax::CbfChroma, 0, false), (Syntax::CbfChroma, 0, true),
        ]);
        for _ in 0..4 {
            decisions.extend([(Syntax::CbfChroma, 1, false),
                (Syntax::CbfChroma, 1, true), (Syntax::CbfLuma, 0, false)]);
        }
        let mut b = Bins(decisions);
        let mut units = Vec::new();
        read_intra_with_chroma(&mut b, [16,32], c, 2,
            |_, u| { units.push(u); Ok(()) }).unwrap();
        assert!(b.0.is_empty());
        assert_eq!(units.len(), 4);
        for (index,u) in units.iter().enumerate() {
            assert_eq!(u.coded, [false; 3]);
            assert_eq!(u.coded_lower, [false, true]);
            assert_eq!(u.chroma_origin, [[8,32],[12,32],[8,40],[12,40]][index]);
            assert!(u.owns_chroma);
        }
    }
    #[test]
    fn full_chroma_four_by_four_leaves_read_own_flags_and_coordinates() {
        let c = Config { log2_cu: 3, log2_min_transform: 2,
            log2_max_transform: 5, max_depth: 1, intra_split: true };
        let mut decisions = VecDeque::from([
            (Syntax::CbfChroma, 0, true), (Syntax::CbfChroma, 0, false),
        ]);
        for index in 0..4 {
            decisions.push_back((Syntax::CbfChroma, 1, index % 2 == 0));
            // Cr's false parent suppresses all descendant Cr syntax.
            decisions.push_back((Syntax::CbfLuma, 0, index == 3));
        }
        let mut bins = Bins(decisions.clone());
        let mut units = Vec::new();
        read_intra_444(&mut bins, [8, 16], c, |_, unit| { units.push(unit); Ok(()) }).unwrap();
        assert_eq!(units.len(), 4);
        for (index, unit) in units.iter().enumerate() {
            assert_eq!(unit.origin, [[8,16], [12,16], [8,20], [12,20]][index]);
            assert_eq!(unit.chroma_origin, unit.origin);
            assert_eq!(unit.log2_chroma_size, 2);
            assert!(unit.owns_chroma);
            assert_eq!(unit.coded, [index == 3, index % 2 == 0, false]);
        }
        assert!(bins.0.is_empty());
        let mut inter = Bins(decisions.clone());
        let mut inter_units = Vec::new();
        read_inter_444(&mut inter, [8, 16], Config {
            intra_split: false, max_depth: 0, ..c
        }, true, |_, unit| { inter_units.push(unit); Ok(()) }).unwrap();
        assert_eq!(inter_units, units);
        assert!(inter.0.is_empty());
        for cut in 0..decisions.len() {
            let mut short = Bins(decisions.iter().take(cut).copied().collect());
            assert!(read_intra_444(&mut short, [8,16], c, |_, _| Ok(())).is_err());
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
