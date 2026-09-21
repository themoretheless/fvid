//! Shared AVC/HEVC binary arithmetic engine (H.264 9.3.3.2; H.265 9.3.4).
//! Syntax binarization and context selection belong to the codec using this engine.
use super::{
    bits::BitReader,
    cabac_tables::{RANGE_LPS, TRANS_LPS, TRANS_MPS},
};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    state: u8,
    mps: bool,
}
impl Context {
    /// H.264 9.3.1.1: m/n come from the table selected by slice type and cabac_init_idc.
    pub fn avc(m: i8, n: i8, slice_qp: i32) -> Self {
        let pre = ((i32::from(m) * slice_qp.clamp(0, 51) >> 4) + i32::from(n)).clamp(1, 126);
        Self {
            state: if pre <= 63 { 63 - pre } else { pre - 64 } as u8,
            mps: pre > 63,
        }
    }
    /// H.265 9.3.2.2: unpack the profile-independent 8-bit initialization entry.
    pub fn hevc(init_value: u8, slice_qp: i32) -> Self {
        let m = i16::from(init_value >> 4) * 5 - 45;
        let n = i16::from(init_value & 15) * 8 - 16;
        Self::avc(m as i8, n as i8, slice_qp)
    }
    pub fn state(&self) -> (u8, bool) {
        (self.state, self.mps)
    }
}

pub struct Cabac<'a> {
    bits: BitReader<'a>,
    range: u16,
    offset: u16,
    terminated: bool,
}
impl<'a> Cabac<'a> {
    /// Start at the byte-aligned entropy offset in an unescaped RBSP.
    pub fn new(rbsp: &'a [u8], bit_offset: usize) -> Result<Self> {
        if !bit_offset.is_multiple_of(8) {
            return Err(invalid("CABAC payload is not byte aligned"));
        }
        let mut bits = BitReader::new(rbsp);
        bits.skip(bit_offset)?;
        let offset = bits.read(9)? as u16;
        if offset >= 510 {
            return Err(invalid("invalid initial CABAC offset"));
        }
        Ok(Self {
            bits,
            range: 510,
            offset,
            terminated: false,
        })
    }
    pub fn bit_position(&self) -> usize {
        self.bits.position()
    }
    pub(super) fn is_terminated(&self) -> bool {
        self.terminated
    }
    fn active(&self) -> Result<()> {
        if self.terminated {
            Err(invalid("CABAC engine has terminated"))
        } else {
            Ok(())
        }
    }
    fn renormalize(bits: &mut BitReader<'a>, range: &mut u16, offset: &mut u16) -> Result<()> {
        while *range < 256 {
            *range <<= 1;
            *offset = (*offset << 1) | bits.read(1)? as u16;
        }
        if *offset >= *range {
            return Err(invalid("CABAC offset exceeds coding interval"));
        }
        Ok(())
    }
    /// Decode one context-coded bin. A truncated read leaves engine and context unchanged.
    pub fn decision(&mut self, context: &mut Context) -> Result<bool> {
        self.active()?;
        let lps = RANGE_LPS[usize::from(context.state)][usize::from((self.range >> 6) & 3)];
        let mut range = self.range - lps;
        let mut offset = self.offset;
        let mut next = *context;
        let bin;
        if offset >= range {
            offset -= range;
            range = lps;
            bin = !context.mps;
            if context.state == 0 {
                next.mps = !next.mps;
            }
            next.state = TRANS_LPS[usize::from(context.state)];
        } else {
            bin = context.mps;
            next.state = TRANS_MPS[usize::from(context.state)];
        }
        let mut bits = self.bits.clone();
        Self::renormalize(&mut bits, &mut range, &mut offset)?;
        self.bits = bits;
        self.range = range;
        self.offset = offset;
        *context = next;
        Ok(bin)
    }
    pub fn bypass(&mut self) -> Result<bool> {
        self.active()?;
        let mut bits = self.bits.clone();
        let mut offset = (self.offset << 1) | bits.read(1)? as u16;
        let bin = offset >= self.range;
        if bin {
            offset -= self.range;
        }
        self.bits = bits;
        self.offset = offset;
        Ok(bin)
    }
    /// Decode end_of_slice_flag or the I_PCM escape. A true bin stops this engine.
    /// Slice trailing bits or PCM alignment must be handled by the syntax reader.
    pub fn terminate(&mut self) -> Result<bool> {
        self.active()?;
        let mut range = self.range - 2;
        if self.offset >= range {
            self.range = range;
            self.terminated = true;
            return Ok(true);
        }
        let mut offset = self.offset;
        let mut bits = self.bits.clone();
        Self::renormalize(&mut bits, &mut range, &mut offset)?;
        self.bits = bits;
        self.range = range;
        self.offset = offset;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_wide_interval_encoder_roundtrips_all_states() {
        // Keep the entire coding interval in u128 rather than using the decoder's
        // bounded offset register. No carry propagation or bit reads are shared.
        for state in 0..64 {
            for seed in 1..=16u32 {
                let initial = Context {
                    state,
                    mps: seed & 1 != 0,
                };
                let mut context = initial;
                let mut low = 0u128;
                let mut range = 510u16;
                let mut shifts = 0;
                let mut random = seed;
                let mut operations = Vec::new();
                for i in 0..16 {
                    random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                    let bin = random & 0x80000000 != 0;
                    let kind = i % 5;
                    if kind == 0 {
                        low = low * 2 + if bin { u128::from(range) } else { 0 };
                        shifts += 1;
                        operations.push((0, bin));
                    } else if kind == 1 {
                        range -= 2;
                        operations.push((1, false));
                    } else {
                        let lps =
                            RANGE_LPS[usize::from(context.state)][usize::from((range >> 6) & 3)];
                        range -= lps;
                        if bin != context.mps {
                            low += u128::from(range);
                            range = lps;
                            if context.state == 0 {
                                context.mps = !context.mps;
                            }
                            context.state = TRANS_LPS[usize::from(context.state)];
                        } else {
                            context.state = TRANS_MPS[usize::from(context.state)];
                        }
                        operations.push((2, bin));
                    }
                    while range < 256 {
                        low *= 2;
                        range *= 2;
                        shifts += 1;
                    }
                }
                // Termination chooses the upper two-unit interval; its odd point
                // provides the stop bit without any guessed padding in the decoder.
                low += u128::from(range - 1);
                let bit_count = 9 + shifts;
                assert!(bit_count < 128);
                let mut bytes = vec![0; (bit_count as usize).div_ceil(8)];
                for bit in 0..bit_count as usize {
                    bytes[bit / 8] |=
                        (((low >> (bit_count as usize - bit - 1)) & 1) as u8) << (7 - bit % 8);
                }
                let mut decoder = Cabac::new(&bytes, 0).unwrap();
                let mut decoded_context = initial;
                for (kind, expected) in operations {
                    let actual = match kind {
                        0 => decoder.bypass(),
                        1 => decoder.terminate(),
                        _ => decoder.decision(&mut decoded_context),
                    }
                    .unwrap();
                    assert_eq!(actual, expected, "state={state}, seed={seed}");
                }
                assert!(decoder.terminate().unwrap());
                assert_eq!(decoded_context, context);
                assert_eq!(decoder.bit_position(), bit_count as usize);
            }
        }
    }
    #[test]
    fn initialization_and_invalid_streams() {
        assert_eq!(Context::avc(20, -15, 26).state(), (46, false));
        assert_eq!(Context::avc(-28, 127, 26).state(), (17, true));
        assert_eq!(Context::avc(0, 63, 26).state(), (0, false));
        assert_eq!(Context::avc(0, 64, 26).state(), (0, true));
        assert_eq!(Context::avc(127, 127, i32::MAX).state(), (62, true));
        assert_eq!(Context::avc(127, -128, i32::MIN).state(), (62, false));
        for bytes in [&[][..], &[0], &[255, 0], &[255, 128]] {
            assert!(Cabac::new(bytes, 0).is_err());
        }
        assert!(Cabac::new(&[0; 4], 1).is_err());
    }
    #[test]
    fn fixed_arithmetic_trace() {
        let mut c = Cabac::new(&[0x96, 0x35, 0xa0, 0], 0).unwrap();
        assert_eq!((c.range, c.offset, c.bit_position()), (510, 300, 9));
        let mut ctx = Context {
            state: 0,
            mps: false,
        };
        assert!(c.decision(&mut ctx).unwrap()); // LPS: [270,510), then renormalize
        assert_eq!(
            (c.range, c.offset, c.bit_position(), ctx.state()),
            (480, 60, 10, (0, true))
        );
        assert!(c.decision(&mut ctx).unwrap()); // MPS, interval [0,240)
        assert_eq!(
            (c.range, c.offset, c.bit_position(), ctx.state()),
            (480, 121, 11, (1, true))
        );
        assert!(!c.bypass().unwrap());
        assert_eq!((c.range, c.offset, c.bit_position()), (480, 243, 12));
        assert!(c.bypass().unwrap());
        assert_eq!((c.range, c.offset, c.bit_position()), (480, 6, 13));
        assert!(!c.terminate().unwrap());
        assert_eq!((c.range, c.offset, c.bit_position()), (478, 6, 13));
    }
    #[test]
    fn termination_and_truncation_are_explicit() {
        let mut c = Cabac::new(&[254, 128], 0).unwrap(); // offset 509
        assert!(c.terminate().unwrap());
        assert_eq!(c.bit_position(), 9);
        assert!(c.bypass().is_err());
        assert!(c.terminate().is_err());
        let mut c = Cabac::new(&[0, 0], 0).unwrap();
        for _ in 0..7 {
            assert!(!c.bypass().unwrap());
        }
        let before = (c.range, c.offset, c.bit_position());
        assert!(c.bypass().is_err());
        let mut ctx = Context {
            state: 0,
            mps: false,
        };
        let saved = ctx;
        c.decision(&mut ctx).unwrap(); // 510-240=270: no read needed
        let before_decision = (c.range, c.offset, c.bit_position());
        let saved_after = ctx;
        assert!(c.decision(&mut ctx).is_err());
        assert_eq!((c.range, c.offset, c.bit_position()), before_decision);
        assert_eq!(ctx, saved_after);
        assert_ne!(ctx, saved);
        assert_eq!(before, (510, 0, 16));
    }
}
