//! Main/Main10 QP delta syntax and 4:2:0 component quantization parameters.
use super::{hevc_cabac::Syntax, hevc_residual::ResidualBins};
use crate::{Result, invalid};
fn offset(depth: u8) -> Result<i32> {
    if !(8..=10).contains(&depth) {
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
/// Return nonnegative Y/Cb/Cr QPs for inverse scaling. Chroma offsets are the
/// validated PPS+slice sums; Main/Main10 has no CU chroma-offset-list extension.
pub fn components(qp_y: i32, depths: [u8; 2], chroma_offsets: [i32; 2]) -> Result<[u8; 3]> {
    let ybd = offset(depths[0])?;
    let cbd = offset(depths[1])?;
    if !(-ybd..=51).contains(&qp_y) || chroma_offsets.iter().any(|v| !(-12..=12).contains(v)) {
        return Err(invalid("invalid HEVC component QP input"));
    }
    let mut result = [(qp_y + ybd) as u8, 0, 0];
    for (i, offset) in chroma_offsets.into_iter().enumerate() {
        let index = (qp_y + offset).clamp(-cbd, 57);
        let mapped = match index {
            ..=29 => index,
            30..=43 => {
                [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37][(index - 30) as usize]
            }
            _ => index - 6,
        };
        result[i + 1] = (mapped + cbd) as u8;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
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
        for depth in [8, 10] {
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
