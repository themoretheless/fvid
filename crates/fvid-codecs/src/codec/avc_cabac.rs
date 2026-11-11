//! H.264 CABAC context bank and residual syntax for 4x4 and chroma DC blocks.
use super::{
    avc_cabac_init::INIT,
    avc_slice::SliceType,
    cabac::{Cabac, Context},
};
use crate::{Result, invalid};
#[path = "avc_cabac_444_init.rs"]
mod colour444_init;

#[derive(Clone)]
pub struct AvcCabac<'a> {
    rbsp: &'a [u8],
    engine: Cabac<'a>,
    contexts: [Option<Context>; 1024],
}
impl<'a> AvcCabac<'a> {
    pub fn new(
        rbsp: &'a [u8],
        bit_offset: usize,
        slice_type: SliceType,
        init_idc: u8,
        qp: i32,
    ) -> Result<Self> {
        if init_idc > 2 {
            return Err(invalid("CABAC init idc exceeds 2"));
        }
        let group = if matches!(slice_type, SliceType::I | SliceType::Si) {
            0
        } else {
            usize::from(init_idc) + 1
        };
        Ok(Self {
            rbsp,
            engine: Cabac::new(rbsp, bit_offset)?,
            contexts: std::array::from_fn(|i| {
                INIT[group]
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|(m, n)| Context::avc(m, n, qp))
            }),
        })
    }
    /// Full nonseparate 4:4:4 residual bank. Ordinary AVC keeps 460..1023 unused.
    pub(crate) fn new_444(
        rbsp: &'a [u8],
        bit_offset: usize,
        slice_type: SliceType,
        init_idc: u8,
        qp: i32,
    ) -> Result<Self> {
        let mut decoder = Self::new(rbsp, bit_offset, slice_type, init_idc, qp)?;
        let group = if matches!(slice_type, SliceType::I | SliceType::Si) {
            0
        } else {
            usize::from(init_idc) + 1
        };
        for (at, &(m, n)) in colour444_init::INIT_444[group].iter().enumerate() {
            decoder.contexts[460 + at] = Some(Context::avc(m, n, qp));
        }
        Ok(decoder)
    }
    pub(crate) fn residual_444(
        &mut self,
        category: ResidualCategory,
        inc: u8,
        field: bool,
    ) -> Result<Residual<64>> {
        read_residual_impl::<64>(self, category, inc, field, false, true)
    }
    pub fn decision(&mut self, index: usize) -> Result<bool> {
        let context = self
            .contexts
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or_else(|| invalid("unused or unsupported AVC CABAC context"))?;
        self.engine.decision(context)
    }
    pub fn bypass(&mut self) -> Result<bool> {
        self.engine.bypass()
    }
    pub fn terminate(&mut self) -> Result<bool> {
        self.engine.terminate()
    }
    pub fn bit_position(&self) -> usize {
        self.engine.bit_position()
    }
    pub fn pcm(
        &mut self,
        luma_depth: u8,
        chroma_depth: u8,
    ) -> Result<super::avc_macroblock::IntraLuma> {
        self.pcm_components(luma_depth, chroma_depth, true)
    }
    pub(super) fn pcm_components(
        &mut self,
        luma_depth: u8,
        chroma_depth: u8,
        has_chroma: bool,
    ) -> Result<super::avc_macroblock::IntraLuma> {
        let (y, [cb, cr]) = self.pcm_raw::<64>(luma_depth, chroma_depth, has_chroma)?;
        Ok(super::avc_macroblock::IntraLuma::Pcm { y, cb, cr })
    }
    pub(super) fn pcm422(
        &mut self,
        luma_depth: u8,
        chroma_depth: u8,
    ) -> Result<(super::avc_macroblock::IntraLuma, [[u16; 128]; 2])> {
        let (y, chroma) = self.pcm_raw::<128>(luma_depth, chroma_depth, true)?;
        Ok((
            super::avc_macroblock::IntraLuma::Pcm {
                y,
                cb: [0; 64],
                cr: [0; 64],
            },
            chroma,
        ))
    }
    pub(super) fn pcm_raw<const C: usize>(
        &mut self,
        luma_depth: u8,
        chroma_depth: u8,
        has_chroma: bool,
    ) -> Result<([u16; 256], [[u16; C]; 2])> {
        if !self.engine.is_terminated()
            || !(8..=14).contains(&luma_depth)
            || !(8..=14).contains(&chroma_depth)
        {
            return Err(invalid(
                "PCM requires terminated CABAC and supported sample depths",
            ));
        }
        let mut bits = super::bits::BitReader::new(self.rbsp);
        bits.skip(self.bit_position())?;
        // CABAC has already selected its terminal interval. The remaining
        // bits of that arithmetic flush byte are not raw PCM alignment syntax
        // (unlike CAVLC); PCM starts at the next byte, as in the JM decoder.
        bits.skip((8 - bits.position() % 8) % 8)?;
        let mut y = [0; 256];
        let mut chroma = [[0; C]; 2];
        for sample in &mut y {
            *sample = bits.read(luma_depth)? as u16;
        }
        for sample in chroma
            .iter_mut()
            .flatten()
            .take(if has_chroma { 2 * C } else { 0 })
        {
            *sample = bits.read(chroma_depth)? as u16;
        }
        self.engine = Cabac::new(self.rbsp, bits.position())?;
        Ok((y, chroma))
    }
    pub fn finish_slice(&self) -> Result<()> {
        if !self.engine.is_terminated() {
            return Err(invalid("CABAC slice has not terminated"));
        }
        let mut bits = super::bits::BitReader::new(self.rbsp);
        bits.skip(self.bit_position() - 1)?;
        if !bits.bit()? {
            return Err(invalid("missing CABAC RBSP stop bit"));
        }
        // The arithmetic terminator has already selected the final interval.
        // Accept unused bits in its last byte (including x264 flush padding).
        bits.skip((8 - bits.position() % 8) % 8)?;
        if bits.remaining() % 16 != 0 {
            return Err(invalid("incomplete CABAC zero word"));
        }
        while bits.remaining() != 0 {
            if bits.read(16)? != 0 {
                return Err(invalid("nonzero CABAC zero word"));
            }
        }
        Ok(())
    }
    /// 4:2:0/4:2:2 luma 8x8 residual, with coded_block_flag inferred as one.
    /// Call only when the corresponding coded_block_pattern bit is set.
    pub fn residual8(&mut self, field: bool) -> Result<Residual<64>> {
        read_residual::<64>(self, ResidualCategory::Luma8, 0, field, false)
    }
    /// coded_context is condTermFlagA + 2*condTermFlagB from neighbouring blocks.
    /// Output is scan order; AC blocks exclude the separately coded DC coefficient.
    /// On syntax errors discard this reader; contexts of preceding bins have advanced.
    pub fn residual(
        &mut self,
        category: ResidualCategory,
        coded_context: u8,
        field: bool,
        chroma422: bool,
    ) -> Result<Residual> {
        read_residual::<16>(self, category, coded_context, field, chroma422)
    }
}
#[derive(Clone, Copy, Debug)]
pub enum ResidualCategory {
    LumaDc,
    LumaAc,
    Luma4,
    ChromaDc,
    ChromaAc,
    Luma8,
    CbDc,
    CbAc,
    Cb4,
    Cb8,
    CrDc,
    CrAc,
    Cr4,
    Cr8,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Residual<const N: usize = 16> {
    pub coefficients: [i32; N],
    pub total_coefficients: u8,
}
trait Bins {
    fn context(&mut self, index: usize) -> Result<bool>;
    fn bypass(&mut self) -> Result<bool>;
}
impl Bins for AvcCabac<'_> {
    fn context(&mut self, index: usize) -> Result<bool> {
        self.decision(index)
    }
    fn bypass(&mut self) -> Result<bool> {
        self.bypass()
    }
}
fn read_residual<const N: usize>(
    bins: &mut impl Bins,
    category: ResidualCategory,
    coded_context: u8,
    field: bool,
    chroma422: bool,
) -> Result<Residual<N>> {
    read_residual_impl(bins, category, coded_context, field, chroma422, false)
}
fn read_residual_impl<const N: usize>(
    bins: &mut impl Bins,
    category: ResidualCategory,
    coded_context: u8,
    field: bool,
    chroma422: bool,
    explicit8: bool,
) -> Result<Residual<N>> {
    if coded_context > 3 {
        return Err(invalid("invalid coded-block neighbour context"));
    }
    let cat = category as usize;
    let count = match category {
        ResidualCategory::Luma8 | ResidualCategory::Cb8 | ResidualCategory::Cr8 => 64,
        ResidualCategory::LumaDc
        | ResidualCategory::Luma4
        | ResidualCategory::CbDc
        | ResidualCategory::Cb4
        | ResidualCategory::CrDc
        | ResidualCategory::Cr4 => 16,
        ResidualCategory::LumaAc
        | ResidualCategory::ChromaAc
        | ResidualCategory::CbAc
        | ResidualCategory::CrAc => 15,
        ResidualCategory::ChromaDc => {
            if chroma422 {
                8
            } else {
                4
            }
        }
    };
    if count > N {
        return Err(invalid("residual block exceeds output size"));
    }
    let mut out = Residual {
        coefficients: [0; N],
        total_coefficients: 0,
    };
    let cbf = [
        85, 89, 93, 97, 101, 1012, 460, 464, 468, 1016, 472, 476, 480, 1020,
    ][cat];
    if (cat != 5 || explicit8) && !bins.context(cbf + usize::from(coded_context))? {
        return Ok(out);
    }
    // H.264 tables 9-34/9-40: component-specific full-resolution categories.
    let significant = if field {
        [
            277, 292, 306, 321, 324, 436, 776, 791, 805, 675, 820, 835, 849, 733,
        ][cat]
    } else {
        [
            105, 120, 134, 149, 152, 402, 484, 499, 513, 660, 528, 543, 557, 718,
        ][cat]
    };
    let last = if field {
        [
            338, 353, 367, 382, 385, 451, 864, 879, 893, 699, 908, 923, 937, 757,
        ][cat]
    } else {
        [
            166, 181, 195, 210, 213, 417, 572, 587, 601, 690, 616, 631, 645, 748,
        ][cat]
    };
    let eight = matches!(
        category,
        ResidualCategory::Luma8 | ResidualCategory::Cb8 | ResidualCategory::Cr8
    );
    let mut positions = [0; N];
    for pos in 0..count {
        let inc = if cat == 3 {
            (pos / if chroma422 { 2 } else { 1 }).min(2)
        } else {
            pos
        };
        let (sig_inc, last_inc) = if eight && pos < 63 {
            let row = super::avc_8x8_tables::SIGNIFICANCE[pos];
            (row[usize::from(field)], row[2])
        } else {
            (inc, inc)
        };
        if pos == count - 1 || bins.context(significant + sig_inc)? {
            positions[usize::from(out.total_coefficients)] = pos;
            out.total_coefficients += 1;
            if pos == count - 1 || bins.context(last + last_inc)? {
                break;
            }
        }
    }
    let base = [
        227, 237, 247, 257, 266, 426, 952, 962, 972, 708, 982, 992, 1002, 766,
    ][cat];
    let (mut ones, mut greater) = (0usize, 0usize);
    for &pos in positions[..usize::from(out.total_coefficients)]
        .iter()
        .rev()
    {
        let inc = if greater != 0 { 0 } else { (1 + ones).min(4) };
        let mut minus_one = 0u64;
        if bins.context(base + inc)? {
            minus_one = 1;
            let next = base + 5 + greater.min(if cat == 3 { 3 } else { 4 });
            while minus_one < 14 && bins.context(next)? {
                minus_one += 1;
            }
            if minus_one == 14 {
                let mut length = 0;
                while bins.bypass()? {
                    length += 1;
                    if length > 30 {
                        return Err(invalid("CABAC coefficient escape exceeds supported range"));
                    }
                }
                let mut suffix = 0u64;
                for _ in 0..length {
                    suffix = (suffix << 1) | u64::from(bins.bypass()?);
                }
                minus_one += (1u64 << length) - 1 + suffix;
            }
        }
        let magnitude =
            i32::try_from(minus_one + 1).map_err(|_| invalid("CABAC coefficient exceeds i32"))?;
        out.coefficients[pos] = if bins.bypass()? {
            -magnitude
        } else {
            magnitude
        };
        if minus_one == 0 {
            ones += 1;
        } else {
            greater += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Trace(VecDeque<(Option<usize>, bool)>);
    impl Bins for Trace {
        fn context(&mut self, index: usize) -> Result<bool> {
            let (expected, bin) = self.0.pop_front().expect("extra context bin");
            assert_eq!(expected, Some(index));
            Ok(bin)
        }
        fn bypass(&mut self) -> Result<bool> {
            let (expected, bin) = self.0.pop_front().expect("extra bypass bin");
            assert_eq!(expected, None);
            Ok(bin)
        }
    }
    #[test]
    fn extended_444_bank_is_opt_in_and_preserves_base_contexts() {
        let ordinary = AvcCabac::new(&[0; 16], 0, SliceType::I, 0, 26).unwrap();
        let extended = AvcCabac::new_444(&[0; 16], 0, SliceType::I, 0, 26).unwrap();
        assert_eq!(&ordinary.contexts[..460], &extended.contexts[..460]);
        assert!(ordinary.contexts[460..].iter().all(Option::is_none));
        assert!(extended.contexts[460..].iter().all(Option::is_some));
        assert!(extended.contexts[276].is_none());
        // Normative table sentinels, including the corrected index label in 9-26.
        assert_eq!(
            colour444_init::INIT_444.map(|g| g[0]),
            [(-17, 123), (-7, 92), (0, 80), (11, 80)]
        );
        assert_eq!(colour444_init::INIT_444[0][541 - 460], (0, 61));
        assert_eq!(colour444_init::INIT_444[0][641 - 460], (10, 55));
        assert_eq!(colour444_init::INIT_444[0][1023 - 460], (-30, 127));
    }
    #[test]
    fn full_resolution_categories_use_component_and_field_contexts() {
        let categories = [
            (ResidualCategory::CbDc, 460, 484, 572, 776, 864, 952),
            (ResidualCategory::CbAc, 464, 499, 587, 791, 879, 962),
            (ResidualCategory::Cb4, 468, 513, 601, 805, 893, 972),
            (ResidualCategory::Cb8, 1016, 660, 690, 675, 699, 708),
            (ResidualCategory::CrDc, 472, 528, 616, 820, 908, 982),
            (ResidualCategory::CrAc, 476, 543, 631, 835, 923, 992),
            (ResidualCategory::Cr4, 480, 557, 645, 849, 937, 1002),
            (ResidualCategory::Cr8, 1020, 718, 748, 733, 757, 766),
            (ResidualCategory::Luma8, 1012, 402, 417, 436, 451, 426),
        ];
        for (category, cbf, sig, last, sig_field, last_field, absolute) in categories {
            for field in [false, true] {
                for inc in 0..4 {
                    let mut zero = Trace(VecDeque::from([(Some(cbf + inc), false)]));
                    assert_eq!(
                        read_residual_impl::<64>(
                            &mut zero, category, inc as u8, field, false, true
                        )
                        .unwrap()
                        .total_coefficients,
                        0
                    );
                    assert!(zero.0.is_empty());
                    let mut nonzero = Trace(VecDeque::from([
                        (Some(cbf + inc), true),
                        (Some(if field { sig_field } else { sig }), true),
                        (Some(if field { last_field } else { last }), true),
                        (Some(absolute + 1), false),
                        (None, false),
                    ]));
                    let r = read_residual_impl::<64>(
                        &mut nonzero,
                        category,
                        inc as u8,
                        field,
                        false,
                        true,
                    )
                    .unwrap();
                    assert_eq!(r.total_coefficients, 1);
                    assert_eq!(r.coefficients[0], 1);
                    assert!(r.coefficients[1..].iter().all(|v| *v == 0));
                    assert!(nonzero.0.is_empty());
                }
            }
        }
    }
    #[test]
    fn eight_by_eight_uses_frame_and_field_context_maps() {
        for (field, indices) in [(false, [402, 403, 404, 418]), (true, [436, 437, 437, 452])] {
            let mut trace = Trace(VecDeque::from([
                (Some(indices[0]), false),
                (Some(indices[1]), false),
                (Some(indices[2]), true),
                (Some(indices[3]), true),
                (Some(427), false),
                (None, true),
            ]));
            let result =
                read_residual::<64>(&mut trace, ResidualCategory::Luma8, 0, field, false).unwrap();
            let mut expected = [0; 64];
            expected[2] = -1;
            assert_eq!(result.coefficients, expected);
            assert_eq!(result.total_coefficients, 1);
            assert!(trace.0.is_empty());
        }
    }
    #[test]
    fn reverse_levels_and_context_changes() {
        // Luma4: significant scan positions 0 and 2, levels +1 and -2.
        let mut trace = Trace(VecDeque::from([
            (Some(96), true), // coded block, A=B=1
            (Some(134), true),
            (Some(195), false),
            (Some(135), false),
            (Some(136), true),
            (Some(197), true),
            (Some(248), true),
            (Some(252), false),
            (None, true),
            (Some(247), false),
            (None, false),
        ]));
        let result =
            read_residual::<16>(&mut trace, ResidualCategory::Luma4, 3, false, false).unwrap();
        assert_eq!(result.total_coefficients, 2);
        let mut expected = [0; 16];
        expected[0] = 1;
        expected[2] = -2;
        assert_eq!(result.coefficients, expected);
        assert!(trace.0.is_empty());
    }
    #[test]
    fn implicit_last_chroma422_and_escape() {
        let mut events = VecDeque::from([(Some(97), true)]);
        // No significance at positions 0..6; position 7 is implicit.
        for i in [321, 321, 322, 322, 323, 323, 323] {
            events.push_back((Some(i), false));
        }
        events.push_back((Some(258), true));
        for _ in 0..13 {
            events.push_back((Some(262), true));
        }
        // EG0 extension 5 => 11010, so absolute level is 20.
        for bit in [true, true, false, true, false, false] {
            events.push_back((None, bit));
        }
        let mut trace = Trace(events);
        let result =
            read_residual::<16>(&mut trace, ResidualCategory::ChromaDc, 0, true, true).unwrap();
        assert_eq!(result.coefficients[7], 20);
        assert_eq!(result.total_coefficients, 1);
        assert!(trace.0.is_empty());
    }
    #[test]
    fn initialization_tables_and_unused_contexts() {
        assert_eq!(INIT[0][3], Some((20, -15)));
        assert_eq!(INIT[1][30], Some((-46, 127)));
        assert_eq!(INIT[2][178], Some((102, -94)));
        assert_eq!(INIT[3][459], Some((20, 64)));
        for group in 0..4 {
            assert!(INIT[group][276].is_none());
            assert_eq!(
                INIT[group].iter().filter(|v| v.is_some()).count(),
                if group == 0 { 410 } else { 459 }
            );
        }
        let mut decoder = AvcCabac::new(&[0; 16], 0, SliceType::I, 0, 26).unwrap();
        assert!(decoder.decision(11).is_err());
        assert!(decoder.decision(276).is_err());
        assert!(decoder.decision(460).is_err());
        assert_eq!(decoder.bit_position(), 9);
        let mut trace = Trace(VecDeque::from([(Some(102), false)]));
        assert_eq!(
            read_residual::<16>(&mut trace, ResidualCategory::ChromaAc, 1, false, false)
                .unwrap()
                .total_coefficients,
            0
        );
        assert!(trace.0.is_empty());
    }
    #[test]
    fn pcm_discards_arithmetic_flush_bits_and_restarts_cabac() {
        // Initial 9-bit offset 509 selects the terminal interval. The seven
        // remaining bits are all one: the former PCM zero-bit check refused it.
        let mut rbsp = vec![0xfe, 0xff];
        let samples: Vec<u8> = (0..384).map(|i| ((i * 17 + 31) % 256) as u8).collect();
        rbsp.extend_from_slice(&samples);
        rbsp.extend_from_slice(&[0, 0]); // restart arithmetic offset 0
        let mut decoder = AvcCabac::new(&rbsp, 0, SliceType::I, 0, 0).unwrap();
        assert!(decoder.terminate().unwrap());
        assert_eq!(decoder.bit_position(), 9);
        let block = decoder.pcm(8, 8).unwrap();
        let super::super::avc_macroblock::IntraLuma::Pcm { y, cb, cr } = block else {
            panic!("PCM expected")
        };
        let actual: Vec<u8> = y.into_iter().chain(cb).chain(cr).map(|v| v as u8).collect();
        assert_eq!(actual, samples);
        assert_eq!(decoder.bit_position(), (2 + 384) * 8 + 9);
        assert!(!decoder.terminate().unwrap());
        assert!(decoder.pcm(8, 8).is_err());
    }
}
