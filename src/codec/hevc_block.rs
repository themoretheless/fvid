//! Main/Main10 4:2:0 residual syntax through inverse transform.
use super::{
    hevc_cabac::Syntax,
    hevc_residual::{self, ResidualBins, Scan},
    hevc_scaling::ScalingLists,
    hevc_transform::{self, Transform},
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub log2_size: u8,
    pub component: u8,
    pub bit_depth: u8,
    pub qp: u8,
    /// None for inter prediction; otherwise the resolved intra mode.
    pub intra_mode: Option<u8>,
    pub transform_skip_enabled: bool,
    pub transquant_bypass: bool,
    pub sign_hiding: bool,
}
impl Config {
    pub fn scan(self) -> Result<Scan> {
        if !(2..=5).contains(&self.log2_size)
            || self.component > 2
            || self.intra_mode.is_some_and(|m| m > 34)
        {
            return Err(invalid("invalid HEVC residual block configuration"));
        }
        Ok(match self.intra_mode {
            Some(mode) if self.log2_size == 2 || (self.log2_size == 3 && self.component == 0) => {
                match mode {
                    6..=14 => Scan::Vertical,
                    22..=30 => Scan::Horizontal,
                    _ => Scan::Diagonal,
                }
            }
            _ => Scan::Diagonal,
        })
    }
}
/// Call only for a coded component (CBF=1), after any CU QP-delta syntax. Returns
/// raster-order residual samples. Any error requires abandoning the slice.
pub fn decode(
    b: &mut impl ResidualBins,
    c: Config,
    scaling: &ScalingLists,
    scratch: &mut Vec<i32>,
    out: &mut Vec<i32>,
) -> Result<()> {
    read(b, c)?.reconstruct(scaling, scratch, out)
}
pub(crate) struct Coefficients {
    config: Config,
    transform: Transform,
    values: Vec<i32>,
    rotate: bool,
    rdpcm: Option<u8>,
}
pub(crate) fn read(b: &mut impl ResidualBins, c: Config) -> Result<Coefficients> {
    read_with_rotation(b, c, false)
}
pub(crate) fn read_with_rotation(
    b: &mut impl ResidualBins,
    c: Config,
    rotation_enabled: bool,
) -> Result<Coefficients> {
    read_with_tools(b, c, rotation_enabled, false, false)
}
pub(crate) fn read_with_tools(
    b: &mut impl ResidualBins,
    c: Config,
    rotation_enabled: bool,
    context_enabled: bool,
    rdpcm_enabled: bool,
) -> Result<Coefficients> {
    let scan = c.scan()?;
    if !(8..=10).contains(&c.bit_depth) || c.qp > 51 + 6 * (c.bit_depth - 8) {
        return Err(invalid("invalid HEVC block depth or QP"));
    }
    let skip = !c.transquant_bypass
        && c.transform_skip_enabled
        && c.log2_size == 2
        && b.decision(Syntax::TransformSkip, usize::from(c.component != 0))?;
    let rdpcm = c.intra_mode.filter(|&mode| {
        rdpcm_enabled && (skip || c.transquant_bypass) && matches!(mode, 10 | 26)
    });
    let coefficients = hevc_residual::read_block_with_skip_context(
        b,
        c.log2_size,
        c.component != 0,
        scan,
        c.sign_hiding && !c.transquant_bypass && rdpcm.is_none(),
        context_enabled && (skip || c.transquant_bypass),
    )?;
    let transform = if c.transquant_bypass {
        Transform::Bypass
    } else if skip {
        Transform::Skip
    } else if c.intra_mode.is_some() && c.component == 0 && c.log2_size == 2 {
        Transform::Dst4
    } else {
        Transform::Dct
    };
    Ok(Coefficients {
        config: c,
        rdpcm,
        transform,
        values: coefficients,
        rotate: rotation_enabled
            && c.log2_size == 2
            && c.intra_mode.is_some()
            && matches!(transform, Transform::Skip | Transform::Bypass),
    })
}
impl Coefficients {
    pub(crate) fn reconstruct(
        self,
        scaling: &ScalingLists,
        scratch: &mut Vec<i32>,
        out: &mut Vec<i32>,
    ) -> Result<()> {
        let c = self.config;
        let matrix = usize::from(c.component) + if c.intra_mode.is_none() { 3 } else { 0 };
        hevc_transform::reconstruct(
            &self.values,
            c.log2_size,
            c.bit_depth,
            c.qp,
            self.transform,
            scaling,
            matrix,
            scratch,
            out,
        )?;
        // H.265 8.6.2: rotate scaled skip samples, or unscaled bypass samples.
        // Ordinary inverse DCT/DST and inter blocks are unaffected.
        if self.rotate {
            out.reverse();
        }
        if let Some(mode) = self.rdpcm {
            let n = 1usize << c.log2_size;
            for y in 0..n {
                for x in 0..n {
                    let previous = if mode == 10 {
                        (x > 0).then(|| y * n + x - 1)
                    } else {
                        (y > 0).then(|| (y - 1) * n + x)
                    };
                    if let Some(previous) = previous {
                        out[y * n + x] = out[y * n + x].checked_add(out[previous])
                            .ok_or_else(|| invalid("HEVC RDPCM residual overflow"))?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Bins(VecDeque<(Option<Syntax>, usize, bool)>);
    impl ResidualBins for Bins {
        fn decision(&mut self, s: Syntax, c: usize) -> Result<bool> {
            let (expected, index, value) =
                self.0.pop_front().ok_or_else(|| invalid("missing bin"))?;
            assert_eq!(
                std::mem::discriminant(&s),
                std::mem::discriminant(&expected.unwrap())
            );
            assert_eq!(c, index);
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            let (s, _, v) = self
                .0
                .pop_front()
                .ok_or_else(|| invalid("missing bypass"))?;
            assert!(s.is_none());
            Ok(v)
        }
    }
    fn config() -> Config {
        Config {
            log2_size: 2,
            component: 0,
            bit_depth: 8,
            qp: 0,
            intra_mode: Some(0),
            transform_skip_enabled: true,
            transquant_bypass: false,
            sign_hiding: true,
        }
    }
    #[test]
    fn syntax_to_pixels_obeys_skip_and_bypass_selection() {
        for bypass in [false, true] {
            let mut c = config();
            c.transquant_bypass = bypass;
            let mut b = Bins(VecDeque::new());
            if !bypass {
                b.0.push_back((Some(Syntax::TransformSkip), 0, true));
            }
            b.0.extend([
                (Some(Syntax::LastX), 0, false),
                (Some(Syntax::LastY), 0, false),
                (Some(Syntax::Greater1), 1, true),
                (Some(Syntax::Greater2), 0, false),
                (None, 0, true),
            ]);
            let mut scratch = Vec::new();
            let mut out = Vec::new();
            decode(&mut b, c, &ScalingLists::flat(), &mut scratch, &mut out).unwrap();
            assert_eq!(out[0], if bypass { -2 } else { -1 });
            assert!(out[1..].iter().all(|&v| v == 0));
            assert!(b.0.is_empty());
        }
    }
    #[test]
    fn rotation_of_skipped_and_bypassed_intra_impulses_leaves_inter_unchanged() {
        for bypass in [false, true] {
            for intra in [false, true] {
                let mut c = config();
                c.transquant_bypass = bypass;
                c.intra_mode = intra.then_some(0);
                let mut b = Bins(VecDeque::new());
                if !bypass {
                    b.0.push_back((Some(Syntax::TransformSkip), 0, true));
                }
                b.0.extend([
                    (Some(Syntax::LastX), 0, false),
                    (Some(Syntax::LastY), 0, false),
                    (Some(Syntax::Greater1), 1, true),
                    (Some(Syntax::Greater2), 0, false),
                    (None, 0, true),
                ]);
                let mut scratch = Vec::new();
                let mut out = Vec::new();
                read_with_rotation(&mut b, c, true).unwrap()
                    .reconstruct(&ScalingLists::flat(), &mut scratch, &mut out).unwrap();
                let mut expected = vec![0; 16];
                expected[if intra { 15 } else { 0 }] = if bypass { -2 } else { -1 };
                assert_eq!(out, expected);
                assert!(b.0.is_empty());
            }
        }
    }
    #[test]
    fn implicit_rdpcm_accumulates_horizontal_and_vertical_residual_impulses() {
        for mode in [10, 26] {
            for bypass in [false, true] {
                let mut c = config();
                c.intra_mode = Some(mode);
                c.transquant_bypass = bypass;
                let mut b = Bins(VecDeque::new());
                if !bypass {
                    b.0.push_back((Some(Syntax::TransformSkip), 0, true));
                }
                b.0.extend([
                    (Some(Syntax::LastX), 0, false),
                    (Some(Syntax::LastY), 0, false),
                    (Some(Syntax::Greater1), 1, true),
                    (Some(Syntax::Greater2), 0, false),
                    (None, 0, true),
                ]);
                let mut scratch = Vec::new();
                let mut out = Vec::new();
                read_with_tools(&mut b, c, false, false, true)
                    .unwrap()
                    .reconstruct(&ScalingLists::flat(), &mut scratch, &mut out)
                    .unwrap();
                let value = if bypass { -2 } else { -1 };
                let expected = if mode == 10 {
                    vec![value, value, value, value, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
                } else {
                    vec![value, 0, 0, 0, value, 0, 0, 0, value, 0, 0, 0, value, 0, 0, 0]
                };
                assert_eq!(out, expected);
                assert!(b.0.is_empty());
            }
        }
    }
    #[test]
    fn scan_selection_distinguishes_luma_eight_and_chroma_four() {
        for component in 0..3 {
            for log in 2..=5 {
                for mode in 0..35 {
                    let mut c = config();
                    c.component = component;
                    c.log2_size = log;
                    c.intra_mode = Some(mode);
                    let scan = c.scan().unwrap();
                    let directional = log == 2 || (component == 0 && log == 3);
                    assert_eq!(
                        scan,
                        if directional && (6..=14).contains(&mode) {
                            Scan::Vertical
                        } else if directional && (22..=30).contains(&mode) {
                            Scan::Horizontal
                        } else {
                            Scan::Diagonal
                        }
                    );
                    c.intra_mode = None;
                    assert_eq!(c.scan().unwrap(), Scan::Diagonal);
                }
            }
        }
        let mut c = config();
        c.bit_depth = 0;
        assert!(
            decode(
                &mut Bins(VecDeque::new()),
                c,
                &ScalingLists::flat(),
                &mut Vec::new(),
                &mut Vec::new()
            )
            .is_err()
        );
    }
}
