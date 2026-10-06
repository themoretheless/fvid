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
    MergeFlag,
    MergeIdx,
    InterPred,
    RefIdx,
    Mvp,
    Mvd0,
    Mvd1,
    ExplicitRdpcmFlag,
    ExplicitRdpcmDirection,
    ChromaQpOffsetFlag,
    ChromaQpOffsetIndex,
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
        Syntax::MergeFlag if init != 0 => {
            if init == 1 {
                &[110]
            } else {
                &[154]
            }
        }
        Syntax::MergeIdx if init != 0 => {
            if init == 1 {
                &[122]
            } else {
                &[137]
            }
        }
        Syntax::InterPred if init != 0 => &[95, 79, 63, 31, 31],
        Syntax::RefIdx if init != 0 => &[153, 153],
        Syntax::Mvp if init != 0 => &[168],
        Syntax::Mvd0 if init != 0 => {
            if init == 1 {
                &[140]
            } else {
                &[169]
            }
        }
        Syntax::Mvd1 if init != 0 => &[198],
        // H.265 tables 9-32/33: luma/chroma entries for P and B all use 139.
        Syntax::ExplicitRdpcmFlag | Syntax::ExplicitRdpcmDirection if init != 0 => &[139, 139],
        Syntax::SaoMerge => &[153],
        Syntax::SaoType => [&[200][..], &[185][..], &[160][..]][init],
        Syntax::SplitCu => [
            &[139, 141, 157][..],
            &[107, 139, 126][..],
            &[107, 139, 126][..],
        ][init],
        Syntax::TransquantBypass | Syntax::ChromaQpOffsetFlag | Syntax::ChromaQpOffsetIndex => &[154],
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bank {
    values: [Context; 44],
    length: usize,
}
impl Bank {
    fn empty() -> Self {
        Self {
            values: [Context::hevc(154, 0); 44],
            length: 0,
        }
    }
    fn initialized(entries: &[u8], qp: i32) -> Result<Self> {
        if entries.len() > 44 {
            return Err(invalid("HEVC context bank is too large"));
        }
        let mut bank = Self::empty();
        bank.length = entries.len();
        for (target, &value) in bank.values.iter_mut().zip(entries) {
            *target = Context::hevc(value, qp);
        }
        Ok(bank)
    }
    fn len(&self) -> usize {
        self.length
    }
    fn get_mut(&mut self, index: usize) -> Option<&mut Context> {
        if index < self.len() {
            self.values.get_mut(index)
        } else {
            None
        }
    }
}
impl std::ops::Index<usize> for Bank {
    type Output = Context;
    fn index(&self, index: usize) -> &Context {
        &self.values[..self.length][index]
    }
}
/// Separate banks preserve shared contexts for syntax aliases (e.g. luma/chroma
/// SAO type) while keeping the arithmetic state in the existing CABAC engine.
pub struct HevcCabac<'a> {
    rbsp: &'a [u8],
    arithmetic: Cabac<'a>,
    contexts: [Bank; 32],
    pub(crate) rice_statistics: [super::hevc_residual::PersistentRiceStatistic; 4],
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
        Syntax::MergeFlag => 21,
        Syntax::MergeIdx => 22,
        Syntax::InterPred => 23,
        Syntax::RefIdx => 24,
        Syntax::Mvp => 25,
        Syntax::Mvd0 => 26,
        Syntax::Mvd1 => 27,
        Syntax::ExplicitRdpcmFlag => 28,
        Syntax::ExplicitRdpcmDirection => 29,
        Syntax::ChromaQpOffsetFlag => 30,
        Syntax::ChromaQpOffsetIndex => 31,
    }
}
/// Probability states transferred at the second CTU of a WPP row.
#[derive(Clone, Copy)]
pub struct Contexts(
    [Bank; 32],
    [super::hevc_residual::PersistentRiceStatistic; 4],
);
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
        let mut contexts = [Bank::empty(); 32];
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
            Syntax::MergeFlag,
            Syntax::MergeIdx,
            Syntax::InterPred,
            Syntax::RefIdx,
            Syntax::Mvp,
            Syntax::Mvd0,
            Syntax::Mvd1,
            Syntax::ExplicitRdpcmFlag,
            Syntax::ExplicitRdpcmDirection,
            Syntax::ChromaQpOffsetFlag,
            Syntax::ChromaQpOffsetIndex,
        ] {
            if let Ok(entries) = values(s, init) {
                contexts[index(s)] = Bank::initialized(&entries, qp)?;
            }
        }
        Ok(Self {
            rbsp,
            arithmetic: Cabac::new(rbsp, bit_offset)?,
            contexts,
            rice_statistics: Default::default(),
            failed: false,
        })
    }
    /// Restart a WPP arithmetic substream without rebuilding probability banks.
    pub fn from_contexts(rbsp: &'a [u8], bit_offset: usize, saved: &Contexts) -> Result<Self> {
        Ok(Self {
            rbsp,
            arithmetic: Cabac::new(rbsp, bit_offset)?,
            contexts: saved.0,
            rice_statistics: saved.1,
            failed: false,
        })
    }
    pub fn contexts(&self) -> Result<Contexts> {
        if self.failed {
            return Err(invalid("cannot save failed HEVC CABAC state"));
        }
        Ok(Contexts(self.contexts, self.rice_statistics))
    }
    pub fn restore_contexts(&mut self, saved: &Contexts) {
        self.contexts = saved.0;
        self.rice_statistics = saved.1;
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
    pub fn align_coefficient_bypass(&mut self) -> Result<()> {
        if self.failed { return Err(invalid("HEVC CABAC requires reset after error")); }
        let result = self.arithmetic.align_hevc_bypass();
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
    /// Consume byte-aligned 4:2:0 PCM and restart only the arithmetic engine.
    pub fn read_pcm(
        &mut self,
        log: u8,
        pcm_depth: [u8; 2],
        depths: [u8; 2],
    ) -> Result<[Vec<u16>; 3]> {
        self.read_pcm_with_chroma(log, pcm_depth, depths, 1)
    }
    pub fn read_pcm_with_chroma(&mut self, log: u8, pcm_depth: [u8;2], depths: [u8;2], chroma_format: u8)
        -> Result<[Vec<u16>;3]> {
        if self.failed
            || !self.arithmetic.is_terminated()
            || !(3..=5).contains(&log)
            || !matches!(chroma_format, 1 | 3)
            || depths.iter().any(|d| !(8..=12).contains(d))
            || pcm_depth.iter().zip(depths).any(|(&p, d)| p == 0 || p > d)
        {
            return Err(invalid("invalid HEVC PCM state or parameters"));
        }
        let result = (|| {
            let mut bits = super::bits::BitReader::new(self.rbsp);
            bits.skip(self.bit_position())?;
            while bits.position() % 8 != 0 {
                if bits.bit()? {
                    return Err(invalid("nonzero HEVC PCM alignment bit"));
                }
            }
            let mut samples = [Vec::new(), Vec::new(), Vec::new()];
            for (c, plane) in samples.iter_mut().enumerate() {
                let chroma = usize::from(c != 0);
                let shift = u8::from(c != 0 && chroma_format != 3);
                let count = 1usize << (2 * (log - shift));
                plane.reserve_exact(count);
                for _ in 0..count {
                    plane.push(
                        (bits.read(pcm_depth[chroma])? as u16)
                            << (depths[chroma] - pcm_depth[chroma]),
                    );
                }
            }
            self.arithmetic = Cabac::new(self.rbsp, bits.position())?;
            Ok(samples)
        })();
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
    fn aligned_bypass_preserves_banks_and_poisoning_requires_reset() {
        let data = [0u8; 8];
        let mut valid = HevcCabac::new(&data, 0, SliceType::I, false, 24).unwrap();
        valid.rice_statistics[0].observe_first_remainder(3);
        let banks = valid.contexts;
        let rice = valid.rice_statistics;
        let position = valid.arithmetic.bit_position();
        valid.align_coefficient_bypass().unwrap();
        assert_eq!(valid.contexts, banks);
        assert_eq!(valid.rice_statistics, rice);
        assert_eq!(valid.arithmetic.bit_position(), position);
        assert!(!valid.bypass().unwrap());
        let bad = [200u8, 0, 0, 0];
        let mut invalid = HevcCabac::new(&bad, 0, SliceType::I, false, 24).unwrap();
        assert!(invalid.align_coefficient_bypass().is_err());
        assert!(invalid.bypass().is_err());
        assert!(invalid.decision(Syntax::SplitCu, 0).is_err());
        assert!(invalid.terminate().is_err());
    }
    #[test]
    fn persistent_rice_statistics_follow_entropy_context_snapshots() {
        let data = [0u8; 8];
        let mut original = HevcCabac::new(&data, 0, SliceType::I, false, 33).unwrap();
        for _ in 0..4 {
            original.rice_statistics[3].observe_first_remainder(3);
        }
        let saved = original.contexts().unwrap();
        let resumed = HevcCabac::from_contexts(&data, 0, &saved).unwrap();
        assert_eq!(resumed.rice_statistics[3].parameter(), 1);
        assert_eq!(resumed.rice_statistics[0].parameter(), 0);
        let mut fresh = HevcCabac::new(&data, 0, SliceType::I, false, 33).unwrap();
        assert_eq!(fresh.rice_statistics[3].parameter(), 0);
        fresh.restore_contexts(&saved);
        assert_eq!(fresh.rice_statistics, original.rice_statistics);
    }
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

#[cfg(test)]
mod chroma_initialization_tests {
    use super::*;
    #[test]
    fn chroma_selection_has_one_context_for_every_initialization() {
        for init in 0..3 {
            for syntax in [Syntax::ChromaQpOffsetFlag, Syntax::ChromaQpOffsetIndex] {
                assert_eq!(values(syntax, init).unwrap(), [154]);
            }
        }
    }
}

#[cfg(test)]
mod pcm_tests {
    use super::*;
    fn stream() -> Vec<u8> {
        let mut data = vec![0xfe, 0x80]; // Initial offset 509: terminal bin one, zero alignment.
        data.extend(0..96); // 8x8 Y + 4x4 Cb + 4x4 Cr, each eight-bit PCM.
        data.extend([0, 0]); // New arithmetic offset zero.
        data
    }
    #[test]
    fn full_chroma_pcm_reads_three_equal_planes_and_restarts_entropy() {
        let mut data = vec![0xfe, 0x80];
        data.extend(0..192);
        data.extend([0,0]);
        for depth in [8,10,12] {
            let mut bins = HevcCabac::new(&data, 0, SliceType::I, false, 24).unwrap();
            let banks = bins.contexts;
            assert!(bins.terminate().unwrap());
            let planes = bins.read_pcm_with_chroma(3, [8;2], [depth;2], 3).unwrap();
            for (component, plane) in planes.iter().enumerate() {
                assert_eq!(plane.len(), 64);
                for (index, &sample) in plane.iter().enumerate() {
                    assert_eq!(sample, ((component * 64 + index) as u16) << (depth - 8));
                }
            }
            assert_eq!(bins.contexts, banks);
            assert_eq!(bins.bit_position(), 194 * 8 + 9);
            assert!(!bins.bypass().unwrap());
        }
        for cut in 2..data.len() {
            let mut bins = HevcCabac::new(&data[..cut], 0, SliceType::I, false, 24).unwrap();
            assert!(bins.terminate().unwrap());
            assert!(bins.read_pcm_with_chroma(3, [8;2], [12;2], 3).is_err());
            assert!(bins.bypass().is_err());
        }
    }
    #[test]
    fn pcm_reads_planes_scales_depth_and_preserves_contexts_on_restart() {
        let data = stream();
        for depth in [8, 10, 12] {
            let mut bins = HevcCabac::new(&data, 0, SliceType::P, false, 24).unwrap();
            let saved = bins.contexts().unwrap();
            assert!(bins.terminate().unwrap());
            let samples = bins.read_pcm(3, [8; 2], [depth; 2]).unwrap();
            assert_eq!(
                samples.iter().map(Vec::len).collect::<Vec<_>>(),
                [64, 16, 16]
            );
            assert_eq!(
                samples.into_iter().flatten().collect::<Vec<_>>(),
                (0u16..96).map(|v| v << (depth - 8)).collect::<Vec<_>>()
            );
            assert_eq!(bins.contexts, saved.0);
            assert_eq!(bins.bit_position(), 98 * 8 + 9);
            assert!(!bins.terminate().unwrap());
        }
    }
    #[test]
    fn truncated_pcm_and_alignment_errors_poison_entropy_state() {
        let data = stream();
        for cut in 2..data.len() {
            let mut bins = HevcCabac::new(&data[..cut], 0, SliceType::I, false, 0).unwrap();
            assert!(bins.terminate().unwrap());
            assert!(bins.read_pcm(3, [8; 2], [8; 2]).is_err());
            assert!(bins.bypass().is_err());
        }
        let mut invalid = data.clone();
        invalid[1] |= 1;
        let mut bins = HevcCabac::new(&invalid, 0, SliceType::I, false, 0).unwrap();
        assert!(bins.terminate().unwrap());
        assert!(
            bins.read_pcm(3, [8; 2], [8; 2])
                .unwrap_err()
                .to_string()
                .contains("PCM alignment")
        );
        assert!(bins.contexts().is_err());
        let mut fresh = HevcCabac::new(&[0; 32], 0, SliceType::I, false, 0).unwrap();
        assert!(fresh.read_pcm(3, [8; 2], [8; 2]).is_err());
    }
}
