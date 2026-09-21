//! HEVC CABAC context initialization for coding-tree, intra and residual syntax.
//! Arithmetic decoding uses the shared FVid engine; context tables are HEVC-only.
use super::cabac::{Cabac, Context};
use crate::{Result, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceType {
    I,
    P,
    B,
}
#[derive(Clone, Copy, Debug)]
pub enum Syntax {
    SaoMerge,
    SaoType,
    SplitCu,
    TransquantBypass,
    Skip,
    PredMode,
    PartMode,
    PreviousIntraLuma,
    IntraChroma,
    SplitTransform,
    CbfLuma,
    CbfChroma,
    QpDelta,
    TransformSkip,
    LastX,
    LastY,
    CodedSubBlock,
    SignificantCoefficient,
    Greater1,
    Greater2,
    RootCbf,
}
fn values(syntax: Syntax, init: usize) -> Result<Vec<u8>> {
    use super::hevc_cabac_tables::*;
    let table: Option<(&[u8], usize)> = match syntax {
        Syntax::SplitTransform => Some((&TABLE_20, 3)),
        Syntax::CbfLuma => Some((&TABLE_21, 2)),
        Syntax::QpDelta => Some((&TABLE_24, 2)),
        Syntax::TransformSkip => Some((&TABLE_25, 2)),
        Syntax::LastX => Some((&TABLE_26, 18)),
        Syntax::LastY => Some((&TABLE_27, 18)),
        Syntax::CodedSubBlock => Some((&TABLE_28, 4)),
        Syntax::Greater1 => Some((&TABLE_30, 24)),
        Syntax::Greater2 => Some((&TABLE_31, 6)),
        _ => None,
    };
    if let Some((table, stride)) = table {
        return Ok(table[init * stride..(init + 1) * stride].to_vec());
    }
    if matches!(syntax, Syntax::CbfChroma) {
        let mut values = TABLE_22[init * 4..init * 4 + 4].to_vec();
        values.push(TABLE_22[12 + init]);
        return Ok(values);
    }
    if matches!(syntax, Syntax::SignificantCoefficient) {
        let mut values = TABLE_29[init * 42..init * 42 + 42].to_vec();
        values.extend_from_slice(&TABLE_29[126 + init * 2..128 + init * 2]);
        return Ok(values);
    }
    if matches!(syntax, Syntax::RootCbf) && init != 0 {
        return Ok(vec![TABLE_14[init - 1]]);
    }

    let base: &[u8] = match syntax {
        Syntax::SaoMerge => &[153],
        Syntax::SaoType => [&[200][..], &[185][..], &[160][..]][init],
        Syntax::SplitCu => [
            &[139, 141, 157][..],
            &[107, 139, 126][..],
            &[107, 139, 126][..],
        ][init],
        Syntax::TransquantBypass => &[154],
        Syntax::Skip if init != 0 => &[197, 185, 201],
        Syntax::PredMode if init != 0 => {
            if init == 1 {
                &[149]
            } else {
                &[134]
            }
        }
        Syntax::PartMode => {
            if init == 0 {
                &[184]
            } else {
                &[154, 139, 154, 154]
            }
        }
        Syntax::PreviousIntraLuma => [&[184][..], &[154][..], &[183][..]][init],
        Syntax::IntraChroma => {
            if init == 0 {
                &[63]
            } else {
                &[152]
            }
        }
        _ => return Err(invalid("HEVC context is unavailable for this slice type")),
    };
    Ok(base.to_vec())
}
/// Separate banks preserve shared contexts for syntax aliases (e.g. luma/chroma
/// SAO type) while keeping the arithmetic state in the existing CABAC engine.
pub struct HevcCabac<'a> {
    arithmetic: Cabac<'a>,
    contexts: [Vec<Context>; 21],
    failed: bool,
}
fn index(s: Syntax) -> usize {
    match s {
        Syntax::SaoMerge => 0,
        Syntax::SaoType => 1,
        Syntax::SplitCu => 2,
        Syntax::TransquantBypass => 3,
        Syntax::Skip => 4,
        Syntax::PredMode => 5,
        Syntax::PartMode => 6,
        Syntax::PreviousIntraLuma => 7,
        Syntax::IntraChroma => 8,
        Syntax::SplitTransform => 9,
        Syntax::CbfLuma => 10,
        Syntax::CbfChroma => 11,
        Syntax::QpDelta => 12,
        Syntax::TransformSkip => 13,
        Syntax::LastX => 14,
        Syntax::LastY => 15,
        Syntax::CodedSubBlock => 16,
        Syntax::SignificantCoefficient => 17,
        Syntax::Greater1 => 18,
        Syntax::Greater2 => 19,
        Syntax::RootCbf => 20,
    }
}
impl<'a> HevcCabac<'a> {
    pub fn new(
        rbsp: &'a [u8],
        bit_offset: usize,
        slice: SliceType,
        cabac_init: bool,
        qp: i32,
    ) -> Result<Self> {
        if slice == SliceType::I && cabac_init {
            return Err(invalid("HEVC I slice cannot swap CABAC initialization"));
        }
        let init = match (slice, cabac_init) {
            (SliceType::I, _) => 0,
            (SliceType::P, false) | (SliceType::B, true) => 1,
            _ => 2,
        };
        let mut contexts: [Vec<Context>; 21] = std::array::from_fn(|_| Vec::new());
        for s in [
            Syntax::SaoMerge,
            Syntax::SaoType,
            Syntax::SplitCu,
            Syntax::TransquantBypass,
            Syntax::Skip,
            Syntax::PredMode,
            Syntax::PartMode,
            Syntax::PreviousIntraLuma,
            Syntax::IntraChroma,
            Syntax::SplitTransform,
            Syntax::CbfLuma,
            Syntax::CbfChroma,
            Syntax::QpDelta,
            Syntax::TransformSkip,
            Syntax::LastX,
            Syntax::LastY,
            Syntax::CodedSubBlock,
            Syntax::SignificantCoefficient,
            Syntax::Greater1,
            Syntax::Greater2,
            Syntax::RootCbf,
        ] {
            if let Ok(entries) = values(s, init) {
                contexts[index(s)] = entries.iter().map(|&v| Context::hevc(v, qp)).collect();
            }
        }
        Ok(Self {
            arithmetic: Cabac::new(rbsp, bit_offset)?,
            contexts,
            failed: false,
        })
    }
    pub fn decision(&mut self, syntax: Syntax, increment: usize) -> Result<bool> {
        if self.failed {
            return Err(invalid("HEVC CABAC requires reset after error"));
        }
        let result = match self.contexts[index(syntax)].get_mut(increment) {
            Some(c) => self.arithmetic.decision(c),
            None => Err(invalid("HEVC context increment out of range")),
        };
        self.failed = result.is_err();
        result
    }
    pub fn bypass(&mut self) -> Result<bool> {
        if self.failed {
            return Err(invalid("HEVC CABAC requires reset after error"));
        }
        let result = self.arithmetic.bypass();
        self.failed = result.is_err();
        result
    }
    pub fn terminate(&mut self) -> Result<bool> {
        if self.failed {
            return Err(invalid("HEVC CABAC requires reset after error"));
        }
        let result = self.arithmetic.terminate();
        self.failed = result.is_err();
        result
    }
    pub fn bit_position(&self) -> usize {
        self.arithmetic.bit_position()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn residual_bank_layout_and_xy_adaptation_are_independent() {
        let data = [0; 32];
        for (slice, init) in [(SliceType::I, 0), (SliceType::P, 1), (SliceType::B, 2)] {
            let h = HevcCabac::new(&data, 0, slice, false, 33).unwrap();
            for (syntax, count) in [
                (Syntax::SplitTransform, 3),
                (Syntax::CbfLuma, 2),
                (Syntax::CbfChroma, 5),
                (Syntax::LastX, 18),
                (Syntax::LastY, 18),
                (Syntax::SignificantCoefficient, 44),
                (Syntax::Greater1, 24),
                (Syntax::Greater2, 6),
            ] {
                assert_eq!(h.contexts[index(syntax)].len(), count);
            }
            assert_eq!(
                h.contexts[index(Syntax::CbfChroma)][4],
                Context::hevc(154, 33)
            );
            assert_eq!(
                h.contexts[index(Syntax::SignificantCoefficient)][42],
                Context::hevc(if init == 0 { 141 } else { 140 }, 33)
            );
            assert_eq!(
                h.contexts[index(Syntax::RootCbf)].len(),
                usize::from(init != 0)
            );
        }
        let mut h = HevcCabac::new(&data, 0, SliceType::I, false, 33).unwrap();
        let original = h.contexts[index(Syntax::LastY)][0];
        h.decision(Syntax::LastX, 0).unwrap();
        assert_ne!(h.contexts[index(Syntax::LastX)][0], original);
        assert_eq!(h.contexts[index(Syntax::LastY)][0], original);
    }
    #[test]
    fn all_initialization_values_clip_qp_and_select_mps() {
        for value in 0..=255u8 {
            for qp in -48..=63 {
                let slope = (i32::from(value) / 16) * 5 - 45;
                let offset = (i32::from(value) % 16) * 8 - 16;
                let pre = (slope * qp.clamp(0, 51)).div_euclid(16) + offset;
                let pre = pre.clamp(1, 126);
                assert_eq!(
                    Context::hevc(value, qp).state(),
                    (
                        if pre <= 63 {
                            (63 - pre) as u8
                        } else {
                            (pre - 64) as u8
                        },
                        pre > 63
                    )
                );
            }
        }
    }
    #[test]
    fn slice_table_swap_and_invalid_context_poisoning() {
        let data = [0; 16];
        let p = HevcCabac::new(&data, 0, SliceType::P, false, 33).unwrap();
        let b = HevcCabac::new(&data, 0, SliceType::B, true, 33).unwrap();
        assert_eq!(p.contexts, b.contexts);
        let mut i = HevcCabac::new(&data, 0, SliceType::I, false, 33).unwrap();
        assert_eq!(
            i.contexts[index(Syntax::SaoType)][0],
            Context::hevc(200, 33)
        );
        assert_eq!(i.contexts[index(Syntax::SplitCu)].len(), 3);
        assert!(i.decision(Syntax::Skip, 0).is_err());
        assert!(i.bypass().is_err());
        assert!(HevcCabac::new(&data, 0, SliceType::I, true, 33).is_err());
    }
}
