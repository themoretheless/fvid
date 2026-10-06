//! HEVC deblocking thresholds and chroma sample filtering (8.7.2.5).
use crate::{Result, invalid};
const TC: [i32; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
    3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LumaFilter {
    Off,
    Weak { second: [bool; 2] },
    Strong,
}
/// Return beta/tC for boundary strength 1 or 2; strength zero disables filtering.
pub fn luma_thresholds(
    qps: [i32; 2],
    strength: u8,
    offsets_div2: [i8; 2],
    depth: u8,
) -> Result<[i32; 2]> {
    if !(8..=16).contains(&depth)
        || strength > 2
        || offsets_div2.iter().any(|v| !(-6..=6).contains(v))
    {
        return Err(invalid("invalid HEVC luma threshold parameters"));
    }
    let min = -6 * i32::from(depth - 8);
    if qps.iter().any(|v| !(min..=51).contains(v)) {
        return Err(invalid("invalid HEVC luma boundary QP"));
    }
    if strength == 0 {
        return Ok([0, 0]);
    }
    let qp = (qps[0] + qps[1] + 1) >> 1;
    let beta_index = (qp + 2 * i32::from(offsets_div2[0])).clamp(0, 51);
    let beta = match beta_index {
        0..=15 => 0,
        16..=28 => beta_index - 10,
        _ => 2 * beta_index - 38,
    };
    let tc_index =
        (qp + 2 * (i32::from(strength) - 1) + 2 * i32::from(offsets_div2[1])).clamp(0, 53);
    Ok([beta << (depth - 8), TC[tc_index as usize] << (depth - 8)])
}
/// Select filter for a four-sample edge segment using its first and last lines.
/// Each line has p/q arrays ordered nearest-to-edge first.
pub fn luma_decision(
    lines: [[[u16; 4]; 2]; 2],
    thresholds: [i32; 2],
    depth: u8,
) -> Result<LumaFilter> {
    let [beta, tc] = thresholds;
    if !(8..=16).contains(&depth)
        || !(0..=(64 << (depth-8).max(4))).contains(&beta)
        || !(0..=(24 << (depth-8).max(4))).contains(&tc)
        || lines
            .iter()
            .flatten()
            .flatten()
            .any(|&v| u32::from(v) >= 1u32 << depth)
    {
        return Err(invalid("invalid HEVC edge decision input"));
    }
    let lines = lines.map(|l| l.map(|v| v.map(i32::from)));
    let curvature = lines.map(|l| l.map(|v| (v[2] - 2 * v[1] + v[0]).abs()));
    let dp = curvature[0][0] + curvature[1][0];
    let dq = curvature[0][1] + curvature[1][1];
    if dp + dq >= beta {
        return Ok(LumaFilter::Off);
    }
    let strong = (0..2).all(|k| {
        let [p, q] = lines[k];
        2 * (curvature[k][0] + curvature[k][1]) < beta >> 2
            && (p[3] - p[0]).abs() + (q[0] - q[3]).abs() < beta >> 3
            && (p[0] - q[0]).abs() < (5 * tc + 1) >> 1
    });
    Ok(if strong {
        LumaFilter::Strong
    } else {
        let threshold = (beta + (beta >> 1)) >> 3;
        LumaFilter::Weak {
            second: [dp < threshold, dq < threshold],
        }
    })
}
/// Filter one luma line. Inputs/outputs are nearest-to-edge first; p3/q3 stay
/// unchanged. The edge decision is derived separately from lines 0 and 3.
#[inline]
pub fn luma_sample(
    p: [u16; 4],
    q: [u16; 4],
    tc: i32,
    depth: u8,
    filter: LumaFilter,
    enabled: [bool; 2],
) -> Result<[[u16; 4]; 2]> {
    if !(8..=16).contains(&depth) || !(0..=(24 << (depth-8).max(4))).contains(&tc) {
        return Err(invalid("invalid HEVC luma filter parameters"));
    }
    let max = (1i32 << depth) - 1;
    if p.into_iter().chain(q).any(|v| i32::from(v) > max) {
        return Err(invalid("HEVC luma sample exceeds bit depth"));
    }
    let a = p.map(i32::from);
    let b = q.map(i32::from);
    let mut out = [a, b];
    match filter {
        LumaFilter::Off => {}
        LumaFilter::Strong => {
            for side in 0..2 {
                let (a, b) = if side == 0 { (a, b) } else { (b, a) };
                let values = [
                    (a[2] + 2 * a[1] + 2 * a[0] + 2 * b[0] + b[1] + 4) >> 3,
                    (a[2] + a[1] + a[0] + b[0] + 2) >> 2,
                    (2 * a[3] + 3 * a[2] + a[1] + a[0] + b[0] + 4) >> 3,
                ];
                for i in 0..3 {
                    out[side][i] = values[i].clamp(a[i] - 2 * tc, a[i] + 2 * tc);
                }
            }
        }
        LumaFilter::Weak { second } => {
            let delta = (9 * (b[0] - a[0]) - 3 * (b[1] - a[1]) + 8) >> 4;
            if delta.abs() < 10 * tc {
                let delta = delta.clamp(-tc, tc);
                out[0][0] = (a[0] + delta).clamp(0, max);
                out[1][0] = (b[0] - delta).clamp(0, max);
                if second[0] {
                    let d =
                        ((((a[2] + a[0] + 1) >> 1) - a[1] + delta) >> 1).clamp(-(tc >> 1), tc >> 1);
                    out[0][1] = (a[1] + d).clamp(0, max);
                }
                if second[1] {
                    let d =
                        ((((b[2] + b[0] + 1) >> 1) - b[1] - delta) >> 1).clamp(-(tc >> 1), tc >> 1);
                    out[1][1] = (b[1] + d).clamp(0, max);
                }
            }
        }
    }
    for side in 0..2 {
        if !enabled[side] {
            out[side] = if side == 0 { a } else { b };
        }
    }
    Ok(out.map(|v| v.map(|x| x as u16)))
}
/// Chroma deblocking uses only the PPS chroma offset, not the slice offset.
/// qp_p/qp_q are signed luma QPs (without bit-depth offset).
pub fn chroma_tc(
    qp_p: i32,
    qp_q: i32,
    pps_offset: i8,
    tc_offset_div2: i8,
    depths: [u8; 2],
) -> Result<i32> {
    chroma_tc_with_format(qp_p, qp_q, pps_offset, tc_offset_div2, depths, 1)
}
pub fn chroma_tc_with_format(qp_p: i32, qp_q: i32, pps_offset: i8,
    tc_offset_div2: i8, depths: [u8;2], chroma_format: u8) -> Result<i32> {
    if !(1..=3).contains(&chroma_format) { return Err(invalid("invalid HEVC deblock chroma format")); }
    if depths.iter().any(|d| !(8..=16).contains(d))
        || !(-12..=12).contains(&pps_offset)
        || !(-6..=6).contains(&tc_offset_div2)
    {
        return Err(invalid("invalid HEVC deblock parameters"));
    }
    let min = -6 * i32::from(depths[0] - 8);
    if !(min..=51).contains(&qp_p) || !(min..=51).contains(&qp_q) {
        return Err(invalid("invalid HEVC deblock QP"));
    }
    let index = ((qp_p + qp_q + 1) >> 1) + i32::from(pps_offset);
    let qp = if chroma_format != 1 { index.min(51) } else { match index {
        ..=29 => index,
        30..=43 => [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37][(index - 30) as usize],
        _ => index - 6,
    } };
    let index = (qp + 2 + 2 * i32::from(tc_offset_div2)).clamp(0, 53) as usize;
    Ok(TC[index] << (depths[1] - 8))
}
/// Filter one pair across an eligible chroma edge (boundary strength 2).
/// Arrays are nearest-to-edge first. `enabled` applies PCM/bypass exclusions
/// independently per side, after deriving delta from both unmodified sides.
#[inline]
pub fn chroma_sample(
    p: [u16; 2],
    q: [u16; 2],
    tc: i32,
    depth: u8,
    enabled: [bool; 2],
) -> Result<[u16; 2]> {
    if !(8..=16).contains(&depth) || !(0..=(24 << (depth-8).max(4))).contains(&tc) {
        return Err(invalid("invalid HEVC chroma threshold"));
    }
    let max = (1i32 << depth) - 1;
    if p.into_iter().chain(q).any(|v| i32::from(v) > max) {
        return Err(invalid("HEVC chroma deblock sample exceeds depth"));
    }
    let [p0, p1] = p.map(i32::from);
    let [q0, q1] = q.map(i32::from);
    let delta = (((q0 - p0) * 4 + p1 - q1 + 4) >> 3).clamp(-tc, tc);
    Ok([
        if enabled[0] {
            (p0 + delta).clamp(0, max) as u16
        } else {
            p[0]
        },
        if enabled[1] {
            (q0 - delta).clamp(0, max) as u16
        } else {
            q[0]
        },
    ])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn luma_thresholds_scale_and_decisions_use_both_endpoint_lines() {
        let thresholds = luma_thresholds([51, 51], 2, [6, 6], 12).unwrap();
        assert_eq!(thresholds, [1024, 384]);
        assert!(luma_decision([[[0; 4]; 2]; 2], thresholds, 12).is_ok());
        assert_eq!(luma_thresholds([30, 30], 2, [0, 0], 8).unwrap(), [22, 3]);
        assert_eq!(luma_thresholds([51, 51], 2, [6, 6], 10).unwrap(), [256, 96]);
        assert_eq!(luma_thresholds([30, 30], 0, [0, 0], 8).unwrap(), [0, 0]);
        let flat = [[[100; 4], [104; 4]]; 2];
        assert_eq!(luma_decision(flat, [22, 3], 8).unwrap(), LumaFilter::Strong);
        let mut at_limit = flat;
        at_limit[1][1] = [108; 4]; // |p0-q0| = (5*tC+1)/2
        assert_eq!(
            luma_decision(at_limit, [22, 3], 8).unwrap(),
            LumaFilter::Weak {
                second: [true, true]
            }
        );
        let mut bent = flat;
        bent[1][0] = [100, 100, 122, 100]; // curvature exactly beta
        assert_eq!(luma_decision(bent, [22, 3], 8).unwrap(), LumaFilter::Off);
        bent[1][0] = [100, 100, 105, 100];
        assert_eq!(
            luma_decision(bent, [22, 3], 8).unwrap(),
            LumaFilter::Weak {
                second: [false, true]
            }
        );
    }
    #[test]
    fn strong_luma_updates_three_samples_with_bounded_changes() {
        assert_eq!(
            luma_sample([100; 4], [108; 4], 4, 8, LumaFilter::Strong, [true, true]).unwrap(),
            [[103, 102, 101, 100], [105, 106, 107, 108]]
        );
        assert_eq!(
            luma_sample([100; 4], [108; 4], 1, 8, LumaFilter::Strong, [true, false]).unwrap(),
            [[102, 102, 101, 100], [108; 4]]
        );
    }
    #[test]
    fn weak_luma_uses_original_values_and_strict_delta_gate() {
        let mode = LumaFilter::Weak {
            second: [true, true],
        };
        assert_eq!(
            luma_sample([100; 4], [108; 4], 4, 8, mode, [true, true]).unwrap(),
            [[103, 101, 100, 100], [105, 106, 108, 108]]
        );
        assert_eq!(
            luma_sample([100; 4], [108; 4], 4, 8, mode, [false, true]).unwrap(),
            [[100; 4], [105, 106, 108, 108]]
        );
        // Raw delta is exactly 10: tc=1 must reject, not clip it to 1.
        assert_eq!(
            luma_sample([0; 4], [18, 0, 0, 0], 1, 8, mode, [true, true]).unwrap(),
            [[0; 4], [18, 0, 0, 0]]
        );
        assert!(luma_sample([1024; 4], [0; 4], 4, 10, mode, [true, true]).is_err());
    }
    #[test]
    fn chroma_threshold_mapping_and_depth() {
        assert_eq!(chroma_tc(16, 16, 0, 0, [8, 8]).unwrap(), 1);
        assert_eq!(chroma_tc(34, 34, 0, 0, [8, 8]).unwrap(), 4);
        assert_eq!(chroma_tc(34, 34, 1, 0, [8, 8]).unwrap(), 4);
        assert_eq!(chroma_tc(51, 51, 12, 6, [10, 10]).unwrap(), 96);
        assert_eq!(chroma_tc(-12, -12, -12, -6, [10, 10]).unwrap(), 0);
        assert!(chroma_tc(52, 0, 0, 0, [8, 8]).is_err());
    }
    #[test]
    fn signed_delta_clamping_and_independent_side_exclusion() {
        assert_eq!(
            chroma_sample([100, 100], [110, 110], 10, 8, [true, true]).unwrap(),
            [104, 106]
        );
        assert_eq!(
            chroma_sample([110, 110], [100, 100], 10, 8, [true, true]).unwrap(),
            [106, 104]
        );
        assert_eq!(
            chroma_sample([100, 100], [110, 110], 2, 8, [false, true]).unwrap(),
            [100, 108]
        );
        assert_eq!(
            chroma_sample([0, 0], [0, 255], 96, 8, [true, true]).unwrap(),
            [0, 32]
        );
        assert_eq!(
            chroma_sample([1023, 1023], [1023, 0], 96, 10, [true, true]).unwrap(),
            [1023, 927]
        );
        assert!(chroma_sample([256, 0], [0, 0], 1, 8, [true, true]).is_err());
    }
}
