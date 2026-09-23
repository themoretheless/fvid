//! VP9 sample loop filtering (specification section 8.8.5).
use crate::{Result, invalid};
/// Sixteen original samples ordered p7..p0,q0..q7. All decisions and weighted
/// sums use original inputs. Width is the maximum filter width (4, 8 or 16).
pub fn filter(
    samples: [u16; 16],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> Result<[u16; 16]> {
    if ![8, 10, 12].contains(&depth)
        || ![4, 8, 16].contains(&width)
        || level > 63
        || sharpness > 7
        || samples.iter().any(|&v| v >= 1u16 << depth)
    {
        return Err(invalid("invalid VP9 loop filter parameters"));
    }
    Ok(filter_fast(samples, depth, width, level, sharpness))
}
/// Hot-path filter without per-call validation. The caller guarantees
/// `depth ∈ {8,10,12}`, `width ∈ {4,8,16}`, `level ≤ 63`, `sharpness ≤ 7`,
/// and all samples are within `depth` bits.
#[inline]
pub fn filter_fast(
    samples: [u16; 16],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> [u16; 16] {
    if level == 0 {
        return samples;
    }
    let s = samples.map(i32::from);
    let bd = depth - 8;
    let shift = if sharpness > 4 {
        2
    } else {
        u8::from(sharpness > 0)
    };
    let mut limit = (i32::from(level) >> shift).max(1);
    if sharpness > 0 {
        limit = limit.min(9 - i32::from(sharpness));
    }
    let blimit = (2 * (i32::from(level) + 2) + limit) << bd;
    limit <<= bd;
    let thresh = i32::from(level >> 4) << bd;
    let diff = |a: usize, b: usize| (s[a] - s[b]).abs();
    if (4..7).any(|i| diff(i, i + 1) > limit)
        || (8..11).any(|i| diff(i, i + 1) > limit)
        || 2 * diff(7, 8) + diff(6, 9) / 2 > blimit
    {
        return samples;
    }
    let hev = diff(6, 7) > thresh || diff(9, 8) > thresh;
    let flat = width >= 8
        && (4..7).all(|i| diff(i, 7) <= 1 << bd)
        && (9..12).all(|i| diff(i, 8) <= 1 << bd);
    let flat2 = width == 16
        && (0..4).all(|i| diff(i, 7) <= 1 << bd)
        && (12..16).all(|i| diff(i, 8) <= 1 << bd);
    let mut out = samples;
    if flat {
        let log = if flat2 { 4 } else { 3 };
        let n = (1i32 << (log - 1)) - 1;
        for i in -n..n {
            let mut sum = s[(8 + i) as usize];
            for j in -n..=n {
                sum += s[(8 + (i + j).clamp(-n - 1, n)) as usize];
            }
            out[(8 + i) as usize] = ((sum + (1 << (log - 1))) >> log) as u16;
        }
    } else {
        let mid = 1i32 << (depth - 1);
        let clamp = |v: i32| v.clamp(-mid, mid - 1);
        let f = clamp(if hev { clamp(s[6] - s[9]) } else { 0 } + 3 * (s[8] - s[7]));
        let f1 = clamp(f + 4) >> 3;
        let f2 = clamp(f + 3) >> 3;
        out[8] = (clamp(s[8] - mid - f1) + mid) as u16;
        out[7] = (clamp(s[7] - mid + f2) + mid) as u16;
        if !hev {
            let v = (f1 + 1) >> 1;
            out[9] = (clamp(s[9] - mid - v) + mid) as u16;
            out[6] = (clamp(s[6] - mid + v) + mid) as u16;
        }
    }
    out
}
/// Four independent edge lines that share one set of filter parameters. Each
/// lane is sixteen original samples ordered p7..p0,q0..q7. The lanes are fully
/// independent, which is what lets the driver fill SIMD lanes with four parallel
/// rows along an edge. The scalar reference applies [`filter_fast`] per lane.
#[inline]
pub fn filter_batch4(
    lines: [[u16; 16]; 4],
    depth: u8,
    width: usize,
    level: u8,
    sharpness: u8,
) -> [[u16; 16]; 4] {
    fvid_cpu::vp9_filter_batch4(lines, depth, width, level, sharpness)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch4_matches_per_lane_filter() {
        // Deterministic pseudo-random lines spanning flat, hev, masked and
        // level-zero cases so every branch of `filter` is exercised per lane.
        let mut state = 0x2545F491_4F6CDD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for depth in [8u8, 10, 12] {
            let mask = (1u16 << depth) - 1;
            for width in [4usize, 8, 16] {
                for level in [0u8, 1, 16, 33, 63] {
                    for sharpness in [0u8, 4, 7] {
                        let mut lines = [[0u16; 16]; 4];
                        for line in lines.iter_mut() {
                            for v in line.iter_mut() {
                                *v = (next() as u16) & mask;
                            }
                        }
                        let batched = filter_batch4(lines, depth, width, level, sharpness);
                        let expected =
                            lines.map(|s| filter(s, depth, width, level, sharpness).unwrap());
                        assert_eq!(batched, expected);
                    }
                }
            }
        }
    }
}
