//! H.264 CABAC context bank and residual syntax for 4x4 and chroma DC blocks.
use super::{
    avc_cabac_init::INIT,
    avc_slice::SliceType,
    cabac::{Cabac, Context},
};
use crate::{Result, invalid};

#[derive(Clone)]
pub struct AvcCabac<'a> {
    rbsp: &'a [u8],
    engine: Cabac<'a>,
    contexts: [Option<Context>; 460],
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
            contexts: INIT[group].map(|pair| pair.map(|(m, n)| Context::avc(m, n, qp))),
        })
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
        let mut cb = [0; 64];
        let mut cr = [0; 64];
        for sample in &mut y {
            *sample = bits.read(luma_depth)? as u16;
        }
        for sample in cb.iter_mut().chain(cr.iter_mut()) {
            *sample = bits.read(chroma_depth)? as u16;
        }
        self.engine = Cabac::new(self.rbsp, bits.position())?;
        Ok(super::avc_macroblock::IntraLuma::Pcm { y, cb, cr })
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
    if coded_context > 3 {
        return Err(invalid("invalid coded-block neighbour context"));
    }
    let cat = category as usize;
    let count = match category {
        ResidualCategory::Luma8 => 64,
        ResidualCategory::LumaDc | ResidualCategory::Luma4 => 16,
        ResidualCategory::LumaAc | ResidualCategory::ChromaAc => 15,
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
    if cat != 5 && !bins.context(85 + 4 * cat + usize::from(coded_context))? {
        return Ok(out);
    }
    let offset = [0, 15, 29, 44, 47, 0][cat];
    let significant = if cat == 5 {
        if field { 436 } else { 402 }
    } else {
        (if field { 277 } else { 105 }) + offset
    };
    let last = if cat == 5 {
        if field { 451 } else { 417 }
    } else {
        (if field { 338 } else { 166 }) + offset
    };
    let mut positions = [0; N];
    for pos in 0..count {
        let inc = if cat == 3 {
            (pos / if chroma422 { 2 } else { 1 }).min(2)
        } else {
            pos
        };
        let (sig_inc, last_inc) = if cat == 5 && pos < 63 {
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
    let base = 227 + [0, 10, 20, 30, 39, 199][cat];
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
