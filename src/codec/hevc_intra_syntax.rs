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

/// H.265 7.3.8.5: 4:4:4 reads one chroma mode per prediction block;
/// subsampled chroma shares one mode derived from the first luma block.
pub fn read_chroma_modes(
    b: &mut impl ResidualBins, luma_modes: &[u8], chroma_format: u8,
) -> Result<Vec<u8>> {
    if !matches!(luma_modes.len(), 1 | 4) || luma_modes.iter().any(|&mode| mode > 34)
        || !(1..=3).contains(&chroma_format) {
        return Err(crate::invalid("invalid HEVC chroma mode ownership"));
    }
    let count = if chroma_format == 3 { luma_modes.len() } else { 1 };
    luma_modes[..count].iter().map(|&mode| {
        hevc_intra::chroma_mode(mode, read_chroma(b)?)
    }).collect()
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
    fn full_chroma_modes_follow_prediction_block_order_and_local_luma() {
        use std::collections::VecDeque;
        struct Interleaved(VecDeque<(bool, bool)>);
        impl ResidualBins for Interleaved {
            fn decision(&mut self, syntax: Syntax, context: usize) -> Result<bool> {
                assert!(matches!(syntax, Syntax::IntraChroma));
                assert_eq!(context, 0);
                let (bypass, value) = self.0.pop_front().ok_or_else(|| invalid("truncated chroma mode"))?;
                assert!(!bypass);
                Ok(value)
            }
            fn bypass(&mut self) -> Result<bool> {
                let (bypass, value) = self.0.pop_front().ok_or_else(|| invalid("truncated chroma mode"))?;
                assert!(bypass);
                Ok(value)
            }
        }
        let events = VecDeque::from([
            (false,true), (true,false), (true,false), // planar collides with local luma: 34
            (false,false), // derived from local luma 10
            (false,true), (true,true), (true,false), // explicit horizontal 10
            (false,true), (true,true), (true,true), // explicit DC 1
        ]);
        let luma = [0,10,26,34];
        let mut full = Interleaved(events.clone());
        assert_eq!(read_chroma_modes(&mut full, &luma, 3).unwrap(), [34,10,10,1]);
        assert!(full.0.is_empty());
        for cut in 0..events.len() {
            let mut short = Interleaved(events.iter().take(cut).copied().collect());
            assert!(read_chroma_modes(&mut short, &luma, 3).is_err());
        }
        for format in [1,2] {
            let mut shared = Interleaved(events.clone());
            assert_eq!(read_chroma_modes(&mut shared, &luma, format).unwrap(), [34]);
            assert_eq!(shared.0.len(), events.len() - 3);
        }
        for modes in [&[][..], &[0,1][..], &[35][..]] {
            let mut invalid_modes = Interleaved(events.clone());
            assert!(read_chroma_modes(&mut invalid_modes, modes, 3).is_err());
            assert_eq!(invalid_modes.0, events);
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
