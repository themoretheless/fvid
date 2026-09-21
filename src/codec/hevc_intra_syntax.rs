//! Intra coding-unit mode syntax; flag pass precedes the mode-value pass.
use super::{hevc_cabac::Syntax, hevc_intra, hevc_residual::ResidualBins};
use crate::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LumaCode {
    pub mpm: bool,
    pub value: u8,
}
impl LumaCode {
    /// Resolve in raster prediction-block order, publishing each derived mode
    /// before resolving the next block's neighbours.
    pub fn resolve(self, left: Option<u8>, top: Option<u8>) -> Result<u8> {
        hevc_intra::luma_mode(left, top, self.mpm, self.value)
    }
}

/// Read one 2Nx2N or four NxN prediction-block codes, after PCM is excluded.
/// No chroma bins are consumed; the caller reads chroma once for a 4:2:0 CU.
pub fn read_luma(b: &mut impl ResidualBins, nxn: bool) -> Result<Vec<LumaCode>> {
    let count = if nxn { 4 } else { 1 };
    let mut codes = Vec::with_capacity(count);
    for _ in 0..count {
        codes.push(LumaCode {
            mpm: b.decision(Syntax::PreviousIntraLuma, 0)?,
            value: 0,
        });
    }
    for code in &mut codes {
        if code.mpm {
            if b.bypass()? {
                code.value = 1 + u8::from(b.bypass()?);
            }
        } else {
            for _ in 0..5 {
                code.value = (code.value << 1) | u8::from(b.bypass()?);
            }
        }
    }
    Ok(codes)
}

/// Return intra_chroma_pred_mode: 4 means derived from the colocated luma mode.
pub fn read_chroma(b: &mut impl ResidualBins) -> Result<u8> {
    if !b.decision(Syntax::IntraChroma, 0)? {
        return Ok(4);
    }
    Ok((u8::from(b.bypass()?) << 1) | u8::from(b.bypass()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invalid;
    struct Bins {
        flags: Vec<bool>,
        bits: Vec<bool>,
        flag: usize,
        bit: usize,
    }
    impl ResidualBins for Bins {
        fn decision(&mut self, syntax: Syntax, increment: usize) -> Result<bool> {
            assert!(matches!(
                syntax,
                Syntax::PreviousIntraLuma | Syntax::IntraChroma
            ));
            assert_eq!(increment, 0);
            assert_eq!(self.bit, 0);
            let value = self
                .flags
                .get(self.flag)
                .copied()
                .ok_or_else(|| invalid("truncated flags"))?;
            self.flag += 1;
            Ok(value)
        }
        fn bypass(&mut self) -> Result<bool> {
            assert_eq!(self.flag, self.flags.len());
            let value = self
                .bits
                .get(self.bit)
                .copied()
                .ok_or_else(|| invalid("truncated bypass"))?;
            self.bit += 1;
            Ok(value)
        }
    }
    fn bins(flags: Vec<bool>, bits: Vec<bool>) -> Bins {
        Bins {
            flags,
            bits,
            flag: 0,
            bit: 0,
        }
    }
    #[test]
    fn nxn_reads_all_flags_before_indices_and_remainders() {
        let mut b = bins(
            vec![true, false, true, true],
            vec![
                false, true, false, true, false, true, true, false, true, true,
            ],
        );
        let codes = read_luma(&mut b, true).unwrap();
        assert_eq!(
            codes,
            [
                LumaCode {
                    mpm: true,
                    value: 0
                },
                LumaCode {
                    mpm: false,
                    value: 21
                },
                LumaCode {
                    mpm: true,
                    value: 1
                },
                LumaCode {
                    mpm: true,
                    value: 2
                }
            ]
        );
        assert_eq!(b.bit, b.bits.len());
        assert_eq!(codes[0].resolve(None, None).unwrap(), 0);
        assert_eq!(codes[2].resolve(Some(26), None).unwrap(), 1);
        for length in 0..10 {
            let mut short = bins(b.flags.clone(), b.bits[..length].to_vec());
            assert!(read_luma(&mut short, true).is_err());
        }
    }
    #[test]
    fn all_chroma_codes_and_unsplit_remainder_values() {
        assert_eq!(read_chroma(&mut bins(vec![false], vec![])).unwrap(), 4);
        for value in 0..4 {
            assert_eq!(
                read_chroma(&mut bins(vec![true], vec![value & 2 != 0, value & 1 != 0])).unwrap(),
                value
            );
        }
        for value in 0..32 {
            let bits = (0..5).rev().map(|i| value & (1 << i) != 0).collect();
            assert_eq!(
                read_luma(&mut bins(vec![false], bits), false).unwrap(),
                [LumaCode { mpm: false, value }]
            );
        }
        assert!(read_chroma(&mut bins(vec![true], vec![false])).is_err());
    }
}
