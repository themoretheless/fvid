//! Main/Main10 inverse scaling and integer transforms (H.265 8.6.2–8.6.4).
use super::{hevc_scaling::ScalingLists, hevc_transform_tables::DCT};
use crate::{Result, invalid};

const DST: [[i16; 4]; 4] = [
    [29, 55, 74, 84],
    [74, 74, 0, -74],
    [84, -29, -74, 55],
    [55, -84, 74, -29],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Dct,
    /// Intra luma 4x4, when neither transform skip nor transquant bypass applies.
    Dst4,
    Skip,
    Bypass,
}

/// All coefficient/sample arrays are raster ordered. `qp` is the derived,
/// nonnegative component QP including its bit-depth offset; chroma QP mapping
/// belongs to the caller. `matrix_id` is 0..2 for intra Y/Cb/Cr, 3..5 for inter.
/// Supply resolved flat/default/explicit scaling lists from SPS/PPS.
pub fn reconstruct(
    coefficients: &[i32],
    log2_size: u8,
    bit_depth: u8,
    qp: u8,
    transform: Transform,
    scaling: &ScalingLists,
    matrix_id: usize,
) -> Result<Vec<i32>> {
    if !(2..=5).contains(&log2_size) || !(8..=10).contains(&bit_depth) {
        return Err(invalid("unsupported HEVC transform geometry or bit depth"));
    }
    let side = 1usize << log2_size;
    if coefficients.len() != side * side
        || coefficients.iter().any(|&c| !(-32768..=32767).contains(&c))
        || qp > 51 + 6 * (bit_depth - 8)
        || matrix_id > 5
    {
        return Err(invalid(
            "invalid HEVC coefficient block or quantization parameters",
        ));
    }
    if matches!(transform, Transform::Dst4 | Transform::Skip) && side != 4 {
        return Err(invalid(
            "Main/Main10 DST and transform skip require 4x4 blocks",
        ));
    }
    if transform == Transform::Bypass {
        return Ok(coefficients.to_vec());
    }
    let shift = bit_depth + log2_size - 5;
    let level_scale = [40i64, 45, 51, 57, 64, 72][usize::from(qp % 6)];
    let mut scaled = Vec::with_capacity(coefficients.len());
    for (index, &coefficient) in coefficients.iter().enumerate() {
        let factor = i64::from(scaling.factor(
            usize::from(log2_size - 2),
            matrix_id,
            index % side,
            index / side,
        )?);
        let product = (i64::from(coefficient) * factor * level_scale) << (qp / 6);
        scaled.push(((product + (1 << (shift - 1))) >> shift).clamp(-32768, 32767));
    }
    let final_shift = 20 - bit_depth;
    let round = 1i64 << (final_shift - 1);
    if transform == Transform::Skip {
        return Ok(scaled
            .into_iter()
            .map(|v| (((v << (5 + log2_size)) + round) >> final_shift) as i32)
            .collect());
    }
    let weight = |frequency: usize, sample: usize| -> i64 {
        i64::from(if transform == Transform::Dst4 {
            DST[frequency][sample]
        } else {
            DCT[frequency << (5 - log2_size)][sample]
        })
    };
    let mut intermediate = vec![0i64; side * side];
    for x in 0..side {
        for y in 0..side {
            let sum: i64 = (0..side).map(|k| weight(k, y) * scaled[k * side + x]).sum();
            intermediate[y * side + x] = ((sum + 64) >> 7).clamp(-32768, 32767);
        }
    }
    let mut residual = vec![0i32; side * side];
    for y in 0..side {
        for x in 0..side {
            let sum: i64 = (0..side)
                .map(|k| weight(k, x) * intermediate[y * side + k])
                .sum();
            residual[y * side + x] = ((sum + round) >> final_shift) as i32;
        }
    }
    Ok(residual)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dc_scaling_depth_sizes_and_signed_rounding() {
        let flat = ScalingLists::flat();
        for log in 2..=5 {
            for depth in [8, 10] {
                for (value, expected) in [(64, 10), (-64, -10)] {
                    let mut block = vec![0; 1 << (2 * log)];
                    block[0] = value << (log - 2);
                    let output =
                        reconstruct(&block, log, depth, 0, Transform::Dct, &flat, 0).unwrap();
                    assert!(output.iter().all(|&v| v == expected));
                }
            }
        }
    }
    #[test]
    fn dst_impulse_is_not_dct_and_skip_and_bypass_are_distinct() {
        let flat = ScalingLists::flat();
        let mut block = [0; 16];
        block[0] = 64;
        assert_eq!(
            reconstruct(&block, 2, 8, 0, Transform::Dst4, &flat, 0).unwrap(),
            [2, 4, 5, 6, 4, 7, 10, 11, 5, 10, 13, 15, 6, 11, 15, 17]
        );
        let skip = reconstruct(&block, 2, 8, 0, Transform::Skip, &flat, 0).unwrap();
        assert_eq!(skip[0], 40);
        assert!(skip[1..].iter().all(|&v| v == 0));
        assert_eq!(
            reconstruct(&block, 2, 8, 0, Transform::Bypass, &flat, 0).unwrap(),
            block
        );
    }
    #[test]
    fn four_by_four_matches_factored_reference_with_clipping() {
        // Independent factored 4-point inverse, including the asymmetric odd
        // part. Dense/saturated inputs exercise both intermediate clipping and
        // column-before-row ordering, which a DC-only vector cannot distinguish.
        fn inverse(v: [i64; 4]) -> [i64; 4] {
            let a = 64 * (v[0] + v[2]);
            let b = 64 * (v[0] - v[2]);
            let c = 83 * v[1] + 36 * v[3];
            let d = 36 * v[1] - 83 * v[3];
            [a + c, b + d, b - d, a - c]
        }
        let flat = ScalingLists::flat();
        let mut state = 719u32;
        for depth in [8, 10] {
            for qp in 0..=51 + 6 * (depth - 8) {
                let mut block = [0i32; 16];
                for value in &mut block {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *value = i32::from((state >> 16) as i16);
                }
                let divisor = 2i64.pow(u32::from(depth - 3));
                let multiplier = [40, 45, 51, 57, 64, 72][usize::from(qp % 6)]
                    * 16
                    * 2i64.pow(u32::from(qp / 6));
                let scaled = block.map(|v| {
                    ((i64::from(v) * multiplier + divisor / 2).div_euclid(divisor))
                        .clamp(-32768, 32767)
                });
                let mut intermediate = [0i64; 16];
                for x in 0..4 {
                    let values = inverse(std::array::from_fn(|y| scaled[y * 4 + x]));
                    for y in 0..4 {
                        intermediate[y * 4 + x] =
                            (values[y] + 64).div_euclid(128).clamp(-32768, 32767);
                    }
                }
                let divisor = 2i64.pow(u32::from(20 - depth));
                let mut expected = [0i32; 16];
                for y in 0..4 {
                    let values = inverse(std::array::from_fn(|x| intermediate[y * 4 + x]));
                    for x in 0..4 {
                        expected[y * 4 + x] = (values[x] + divisor / 2).div_euclid(divisor) as i32;
                    }
                }
                assert_eq!(
                    reconstruct(&block, 2, depth, qp, Transform::Dct, &flat, 0).unwrap(),
                    expected
                );
            }
        }
    }
    #[test]
    fn nonflat_scaling_changes_ac_and_extreme_skip_is_bounded() {
        let flat = ScalingLists::flat();
        let defaults = ScalingLists::default();
        let mut block = [0; 64];
        block[63] = 128;
        let a = reconstruct(&block, 3, 8, 12, Transform::Dct, &flat, 0).unwrap();
        let b = reconstruct(&block, 3, 8, 12, Transform::Dct, &defaults, 0).unwrap();
        assert_ne!(a, b);
        for (value, expected) in [(32767, 4096), (-32768, -4096)] {
            let output = reconstruct(&[value; 16], 2, 10, 63, Transform::Skip, &flat, 0).unwrap();
            assert_eq!(output, [expected; 16]);
        }
    }
    #[test]
    fn rejects_invalid_inputs_before_shifting_or_indexing() {
        let flat = ScalingLists::flat();
        for (log, depth, qp, id) in [
            (0, 8, 0, 0),
            (6, 8, 0, 0),
            (2, 7, 0, 0),
            (2, 11, 0, 0),
            (2, 8, 52, 0),
            (2, 10, 64, 0),
            (2, 8, 0, 6),
        ] {
            assert!(reconstruct(&[0; 16], log, depth, qp, Transform::Dct, &flat, id).is_err());
        }
        assert!(reconstruct(&[0; 15], 2, 8, 0, Transform::Dct, &flat, 0).is_err());
        assert!(reconstruct(&[i32::MAX; 16], 2, 8, 0, Transform::Dct, &flat, 0).is_err());
        for mode in [Transform::Dst4, Transform::Skip] {
            assert!(reconstruct(&[0; 64], 3, 8, 0, mode, &flat, 0).is_err());
        }
    }
}
