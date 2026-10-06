//! Main/Main10 QP delta syntax and 4:2:0 component quantization parameters.
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};
fn offset(depth: u8) -> Result<i32> {
    if !(8..=16).contains(&depth) {
        return Err(invalid("unsupported HEVC QP bit depth"));
    }
    Ok(6 * i32::from(depth - 8))
}
pub fn read_delta(b: &mut impl ResidualBins, depth: u8) -> Result<i32> {
    let bd = offset(depth)?;
    let limit = 26 + bd / 2;
    let mut absolute = 0;
    while absolute < 5 && b.decision(Syntax::QpDelta, usize::from(absolute != 0))? {
        absolute += 1;
    }
    if absolute == 5 {
        let mut order = 0;
        while b.bypass()? {
            absolute += 1 << order;
            order += 1;
            if absolute > limit {
                return Err(invalid("HEVC QP delta suffix exceeds range"));
            }
        }
        for shift in (0..order).rev() {
            absolute += i32::from(b.bypass()?) << shift;
        }
    }
    let delta = if absolute != 0 && b.bypass()? {
        -absolute
    } else {
        absolute
    };
    if !(-limit..limit).contains(&delta) {
        return Err(invalid("HEVC QP delta out of range"));
    }
    Ok(delta)
}
/// Neighbours are QPs within the same CTU and available to the quantization
/// group. Supply None otherwise; `previous` already accounts for slice/tile/WPP
/// resets. Returns signed QpY (without the bit-depth offset).
pub fn luma(previous: i32, neighbours: [Option<i32>; 2], delta: i32, depth: u8) -> Result<i32> {
    let bd = offset(depth)?;
    let range = -bd..=51;
    if !range.contains(&previous)
        || neighbours.iter().flatten().any(|v| !range.contains(v))
        || !(-(26 + bd / 2)..26 + bd / 2).contains(&delta)
    {
        return Err(invalid("invalid HEVC QP prediction input"));
    }
    let [a, b] = neighbours.map(|v| v.unwrap_or(previous));
    let prediction = (a + b + 1) >> 1;
    Ok((prediction + delta + bd).rem_euclid(52 + bd) - bd)
}
/// Read the bounded HEVC range-extension chroma adjustment selection.
pub fn read_chroma_offset(b: &mut impl ResidualBins, entries: &[[i8; 2]]) -> Result<[i32; 2]> {
    if !(1..=6).contains(&entries.len())
        || entries.iter().flatten().any(|v| !(-12..=12).contains(v))
    {
        return Err(invalid("invalid HEVC chroma QP list"));
    }
    if !b.decision(Syntax::ChromaQpOffsetFlag, 0)? {
        return Ok([0; 2]);
    }
    let mut index = 0;
    while index + 1 < entries.len() && b.decision(Syntax::ChromaQpOffsetIndex, 0)? {
        index += 1;
    }
    Ok(entries[index].map(i32::from))
}
/// Return nonnegative Y/Cb/Cr QPs for inverse scaling.
pub fn components(qp_y: i32, depths: [u8; 2], chroma_offsets: [i32; 2]) -> Result<[u8; 3]> {
    components_with_cu(qp_y, depths, chroma_offsets, [0; 2])
}
/// Keep PPS/slice and CU list bounds separate before adding their offsets.
pub fn components_with_cu(
    qp_y: i32,
    depths: [u8; 2],
    chroma_offsets: [i32; 2],
    cu_offsets: [i32; 2],
) -> Result<[u8; 3]> {
    components_with_format(qp_y, depths, chroma_offsets, cu_offsets, 1)
}
/// H.265 8.6.1: only 4:2:0 applies Table 8-10's nonlinear chroma mapping.
pub fn components_with_format(
    qp_y: i32, depths: [u8; 2], chroma_offsets: [i32; 2],
    cu_offsets: [i32; 2], chroma_format: u8,
) -> Result<[u8; 3]> {
    if chroma_format == 0 {
        let bd = offset(depths[0])?;
        if !(-bd..=51).contains(&qp_y) { return Err(invalid("invalid HEVC monochrome QP")); }
        return Ok([(qp_y + bd) as u8,0,0]);
    }
    if !(1..=3).contains(&chroma_format) {
        return Err(invalid("invalid HEVC component QP chroma format"));
    }
    let ybd = offset(depths[0])?;
    let cbd = offset(depths[1])?;
    if !(-ybd..=51).contains(&qp_y)
        || chroma_offsets
            .iter()
            .chain(cu_offsets.iter())
            .any(|v| !(-12..=12).contains(v))
    {
        return Err(invalid("invalid HEVC component QP input"));
    }
    let mut result = [(qp_y + ybd) as u8, 0, 0];
    for (i, offset) in chroma_offsets.into_iter().enumerate() {
        let index = (qp_y + offset + cu_offsets[i]).clamp(-cbd, 57);
        let mapped = if chroma_format != 1 { index.min(51) } else { match index {
            ..=29 => index,
            30..=43 => {
                [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37][(index - 30) as usize]
            }
            _ => index - 6,
        } };
        result[i + 1] = (mapped + cbd) as u8;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn full_chroma_qp_uses_linear_mapping_and_saturation() {
        for format in [2, 3] {
            for depth in [8, 10, 12] {
                let bd = 6 * i32::from(depth - 8);
                for qp in -bd..=51 {
                    for delta in [-12, 0, 12] {
                        let actual = super::components_with_format(qp, [depth; 2],
                            [delta, -delta], [12, -12], format).unwrap();
                        assert_eq!(actual, [(qp + bd) as u8,
                            ((qp + delta + 12).clamp(-bd, 51) + bd) as u8,
                            ((qp - delta - 12).clamp(-bd, 51) + bd) as u8]);
                    }
                }
            }
        }
        assert_eq!(super::components_with_format(34, [8; 2], [0; 2], [0; 2], 3).unwrap(), [34; 3]);
        assert_eq!(super::components(34, [8; 2], [0; 2]).unwrap(), [34, 33, 33]);
        assert_eq!(super::components_with_format(34,[12,8],[0;2],[0;2],0).unwrap(),[58,0,0]);
        for format in [4,255] {
            assert!(super::components_with_format(0, [8; 2], [0; 2], [0; 2], format).is_err());
        }
    }
    use super::*;
    use std::collections::VecDeque;
    struct Bins {
        context: VecDeque<bool>,
        bypass: VecDeque<bool>,
        calls: usize,
    }
    impl ResidualBins for Bins {
        fn decision(&mut self, s: Syntax, c: usize) -> Result<bool> {
            assert!(matches!(s, Syntax::QpDelta));
            assert_eq!(c, usize::from(self.calls != 0));
            self.calls += 1;
            self.context
                .pop_front()
                .ok_or_else(|| invalid("missing QP bin"))
        }
        fn bypass(&mut self) -> Result<bool> {
            assert!(self.context.is_empty());
            self.bypass
                .pop_front()
                .ok_or_else(|| invalid("missing QP bypass"))
        }
    }
    fn encoded(delta: i32) -> Bins {
        let a = delta.unsigned_abs();
        let mut c = VecDeque::new();
        let mut b = VecDeque::new();
        for _ in 0..a.min(5) {
            c.push_back(true);
        }
        if a < 5 {
            c.push_back(false);
        } else {
            let mut value = a - 5;
            let mut order = 0;
            while value >= 1 << order {
                b.push_back(true);
                value -= 1 << order;
                order += 1;
            }
            b.push_back(false);
            for bit in (0..order).rev() {
                b.push_back(value & (1 << bit) != 0);
            }
        }
        if a != 0 {
            b.push_back(delta < 0);
        }
        Bins {
            context: c,
            bypass: b,
            calls: 0,
        }
    }
    #[test]
    fn every_delta_and_asymmetric_limits() {
        for depth in [8, 10, 12] {
            let limit = 26 + 3 * i32::from(depth - 8);
            for delta in -limit..limit {
                let mut b = encoded(delta);
                assert_eq!(read_delta(&mut b, depth).unwrap(), delta);
                assert!(b.context.is_empty() && b.bypass.is_empty());
            }
            assert!(read_delta(&mut encoded(limit), depth).is_err());
            assert!(read_delta(&mut encoded(-limit - 1), depth).is_err());
        }
    }
    #[test]
    fn prediction_wrap_negative_rounding_and_chroma_plateaus() {
        assert_eq!(luma(51, [None, None], 1, 8).unwrap(), 0);
        assert_eq!(luma(-12, [None, None], -1, 10).unwrap(), 51);
        assert_eq!(luma(0, [Some(-3), Some(-2)], 0, 10).unwrap(), -2);
        assert_eq!(components(34, [8, 8], [0, 1]).unwrap(), [34, 33, 33]);
        assert_eq!(components(51, [10, 10], [12, -12]).unwrap(), [63, 63, 47]);
        assert_eq!(components(-12, [10, 10], [-12, 0]).unwrap(), [0, 0, 0]);
        assert!(components(52, [8, 8], [0, 0]).is_err());
        assert!(luma(0, [Some(52), None], 0, 8).is_err());
    }
}

#[cfg(test)]
mod chroma_selection_tests {
    use super::*;
    struct Bins {
        bins: std::collections::VecDeque<bool>,
        calls: usize,
    }
    impl ResidualBins for Bins {
        fn decision(&mut self, syntax: Syntax, context: usize) -> Result<bool> {
            assert_eq!(context, 0);
            assert!(matches!(
                (self.calls, syntax),
                (0, Syntax::ChromaQpOffsetFlag) | (1.., Syntax::ChromaQpOffsetIndex)
            ));
            self.calls += 1;
            self.bins
                .pop_front()
                .ok_or_else(|| invalid("missing chroma selection bin"))
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("chroma selection never uses bypass bins")
        }
    }
    #[test]
    fn every_list_size_index_zero_flag_and_truncation() {
        for len in 1..=6 {
            let entries: Vec<_> = (0..len).map(|i| [i as i8 - 3, 12 - i as i8]).collect();
            let mut zero = Bins {
                bins: [false].into(),
                calls: 0,
            };
            assert_eq!(read_chroma_offset(&mut zero, &entries).unwrap(), [0; 2]);
            for index in 0..len {
                let mut encoded = vec![true; index + 1];
                if index + 1 < len {
                    encoded.push(false);
                }
                for cut in 0..encoded.len() {
                    let mut short = Bins {
                        bins: encoded[..cut].iter().copied().collect(),
                        calls: 0,
                    };
                    assert!(read_chroma_offset(&mut short, &entries).is_err());
                }
                let mut full = Bins {
                    bins: encoded.into(),
                    calls: 0,
                };
                assert_eq!(
                    read_chroma_offset(&mut full, &entries).unwrap(),
                    entries[index].map(i32::from)
                );
                assert!(full.bins.is_empty());
            }
        }
        for entries in [vec![], vec![[0, 0]; 7], vec![[13, 0]]] {
            let mut bins = Bins {
                bins: [true].into(),
                calls: 0,
            };
            assert!(read_chroma_offset(&mut bins, &entries).is_err());
            assert_eq!(bins.calls, 0);
        }
    }
    #[test]
    fn cu_offsets_add_before_mapping_without_relaxing_base_bounds() {
        assert_eq!(
            components_with_cu(24, [8, 8], [12, -12], [12, -12]).unwrap(),
            [24, 42, 0]
        );
        assert_eq!(
            components_with_cu(-12, [10, 10], [-12, 12], [-12, 12]).unwrap(),
            [0, 0, 24]
        );
        assert!(components_with_cu(24, [8, 8], [13, 0], [-1, 0]).is_err());
        assert!(components_with_cu(24, [8, 8], [0, 0], [0, -13]).is_err());
    }
}
