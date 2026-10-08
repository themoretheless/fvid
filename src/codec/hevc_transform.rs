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
/// `scratch` and `out` are caller-provided buffers reused across calls (resized as needed).
pub fn reconstruct(
    coefficients: &[i32],
    log2_size: u8,
    bit_depth: u8,
    qp: u8,
    transform: Transform,
    scaling: &ScalingLists,
    matrix_id: usize,
    scratch: &mut Vec<i32>,
    out: &mut Vec<i32>,
) -> Result<()> {
    reconstruct_with_precision(
        coefficients,
        log2_size,
        bit_depth,
        qp,
        transform,
        scaling,
        matrix_id,
        false,
        scratch,
        out,
    )
}

/// Extended dynamic range as specified by H.265 8.6.2–8.6.4.
pub fn reconstruct_with_precision(
    coefficients: &[i32],
    log2_size: u8,
    bit_depth: u8,
    qp: u8,
    transform: Transform,
    scaling: &ScalingLists,
    matrix_id: usize,
    extended_precision: bool,
    scratch: &mut Vec<i32>,
    out: &mut Vec<i32>,
) -> Result<()> {
    if !(2..=5).contains(&log2_size) || !(8..=16).contains(&bit_depth) {
        return Err(invalid("unsupported HEVC transform geometry or bit depth"));
    }
    let range = if extended_precision {
        (bit_depth + 6).max(15)
    } else {
        15
    };
    let minimum = -(1i64 << range);
    let maximum = (1i64 << range) - 1;
    let side = 1usize << log2_size;
    if coefficients.len() != side * side
        || coefficients
            .iter()
            .any(|&c| !(minimum..=maximum).contains(&i64::from(c)))
        || qp > 51 + 6 * (bit_depth - 8)
        || matrix_id > 5
    {
        return Err(invalid(
            "invalid HEVC coefficient block or quantization parameters",
        ));
    }
    if transform == Transform::Dst4 && side != 4 {
        return Err(invalid("HEVC DST requires 4x4 blocks"));
    }
    out.resize(side * side, 0);
    if transform == Transform::Bypass {
        out.copy_from_slice(coefficients);
        return Ok(());
    }
    let shift = bit_depth + log2_size + 10 - range;
    let level_scale = [40i64, 45, 51, 57, 64, 72][usize::from(qp % 6)];
    scratch.resize(side * side, 0);
    for (index, &coefficient) in coefficients.iter().enumerate() {
        if coefficient == 0 {
            scratch[index] = 0;
            continue;
        }
        let factor = if transform == Transform::Skip && side > 4 {
            16
        } else {
            i64::from(scaling.factor(
                usize::from(log2_size - 2),
                matrix_id,
                index % side,
                index / side,
            )?)
        };
        let product = (i64::from(coefficient) * factor * level_scale) << (qp / 6);
        scratch[index] = ((product + (1 << (shift - 1))) >> shift).clamp(minimum, maximum) as i32;
    }
    let final_shift = (20 - bit_depth).max(if extended_precision { 11 } else { 0 });
    let round = 1i64 << (final_shift - 1);
    if transform == Transform::Skip {
        for (i, &v) in scratch.iter().enumerate() {
            out[i] = ((((v as i64) << (5 + log2_size)) + round) >> final_shift) as i32;
        }
        return Ok(());
    }
    if transform == Transform::Dct && scratch[1..].iter().all(|&v| v == 0) {
        let first = ((scratch[0] as i64 * 64 + 64) >> 7).clamp(minimum, maximum);
        let value = ((first * 64 + round) >> final_shift) as i32;
        for v in out.iter_mut() {
            *v = value;
        }
        return Ok(());
    }
    let inverse = |input: &[i32], output: &mut [i64]| {
        if transform == Transform::Dst4 {
            for (x, value) in output.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|k| i64::from(DST[k][x]) * i64::from(input[k]))
                    .sum();
            }
        } else if range > 19 {
            // 14/16-bit extended precision can overflow i32 before clipping.
            for (x, value) in output.iter_mut().enumerate() {
                *value = input
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| i64::from(v) * i64::from(DCT[k * (32 / input.len())][x]))
                    .sum();
            }
        } else {
            let mut narrow = [0i32; 32];
            inverse_dct(input, &mut narrow[..input.len()]);
            for (out, &v) in output.iter_mut().zip(&narrow) {
                *out = i64::from(v);
            }
        }
    };
    // Use scratch for intermediate (column-wise inverse output)
    // We need a second scratch buffer, so we'll use a portion of scratch for intermediate
    // and reuse the beginning for column/output temporaries
    let intermediate_start = side * side;
    scratch.resize(intermediate_start + side * side, 0);
    let (scaled, intermediate) = scratch.split_at_mut(intermediate_start);
    let mut column = [0i32; 32];
    let mut output_buf = [0i64; 32];
    for x in 0..side {
        for k in 0..side {
            column[k] = scaled[k * side + x];
        }
        inverse(&column[..side], &mut output_buf[..side]);
        for y in 0..side {
            intermediate[y * side + x] =
                ((i64::from(output_buf[y]) + 64) >> 7).clamp(minimum, maximum) as i32;
        }
    }
    for y in 0..side {
        inverse(
            &intermediate[y * side..(y + 1) * side],
            &mut output_buf[..side],
        );
        for x in 0..side {
            out[y * side + x] = ((output_buf[x] + round) >> final_shift) as i32;
        }
    }
    Ok(())
}

// Even frequencies form the smaller transform; odd frequencies are antisymmetric.
// Caller bounds the dynamic range to 19 bits: 32 * 90 * 2^18 fits i32.
fn inverse_dct(input: &[i32], output: &mut [i32]) {
    fvid_cpu::hevc_inverse_dct(input, output);
}
fn inverse2(input: &[i32], output: &mut [i32]) {
    output[0] = 64 * (input[0] + input[1]);
    output[1] = 64 * (input[0] - input[1]);
}
macro_rules! inverse_size {
    ($name:ident, $smaller:ident, $n:expr) => {
        fn $name(input: &[i32], output: &mut [i32]) {
            let input: &[i32; $n] = input.try_into().unwrap();
            let output: &mut [i32; $n] = output.try_into().unwrap();
            let even = std::array::from_fn::<_, { $n / 2 }, _>(|k| input[2 * k]);
            let mut values = [0; $n / 2];
            $smaller(&even, &mut values);
            let mut odd = [0; $n / 2];
            for k in (1..$n).step_by(2) {
                if input[k] == 0 {
                    continue;
                }
                for x in 0..$n / 2 {
                    odd[x] += input[k] * i32::from(DCT[k * (32 / $n)][x]);
                }
            }
            for x in 0..$n / 2 {
                output[x] = values[x] + odd[x];
                output[$n - 1 - x] = values[x] - odd[x];
            }
        }
    };
}
inverse_size!(inverse4, inverse2, 4);
inverse_size!(inverse8, inverse4, 8);
inverse_size!(inverse16, inverse8, 16);
inverse_size!(inverse32, inverse16, 32);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn factored_dct_matches_dense_matrix_for_all_sizes() {
        let mut state = 17u32;
        for n in [4usize, 8, 16, 32] {
            for case in 0..128 {
                let mut input = vec![0; n];
                for (k, v) in input.iter_mut().enumerate() {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *v = if case < n && k != case {
                        0
                    } else {
                        (state >> 16) as i16 as i32
                    };
                }
                let mut actual = vec![0; n];
                inverse_dct(&input, &mut actual);
                for x in 0..n {
                    let expected: i64 = (0..n)
                        .map(|k| i64::from(input[k]) * i64::from(DCT[k * (32 / n)][x]))
                        .sum();
                    assert_eq!(i64::from(actual[x]), expected, "n={n} case={case} x={x}");
                }
            }
        }
    }
    #[test]
    fn dc_scaling_depth_sizes_and_signed_rounding() {
        let flat = ScalingLists::flat();
        for log in 2..=5 {
            for depth in [8, 10, 12] {
                for (value, expected) in [(64, 10), (-64, -10)] {
                    let mut block = vec![0; 1 << (2 * log)];
                    block[0] = value << (log - 2);
                    let mut scratch = Vec::new();
                    let mut out = Vec::new();
                    reconstruct(
                        &block,
                        log,
                        depth,
                        0,
                        Transform::Dct,
                        &flat,
                        0,
                        &mut scratch,
                        &mut out,
                    )
                    .unwrap();
                    assert!(out.iter().all(|&v| v == expected));
                }
            }
        }
    }
    #[test]
    fn dst_impulse_is_not_dct_and_skip_and_bypass_are_distinct() {
        let flat = ScalingLists::flat();
        let mut block = [0; 16];
        block[0] = 64;
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        reconstruct(
            &block,
            2,
            8,
            0,
            Transform::Dst4,
            &flat,
            0,
            &mut scratch,
            &mut out,
        )
        .unwrap();
        assert_eq!(
            out,
            [2, 4, 5, 6, 4, 7, 10, 11, 5, 10, 13, 15, 6, 11, 15, 17]
        );
        reconstruct(
            &block,
            2,
            8,
            0,
            Transform::Skip,
            &flat,
            0,
            &mut scratch,
            &mut out,
        )
        .unwrap();
        assert_eq!(out[0], 40);
        assert!(out[1..].iter().all(|&v| v == 0));
        reconstruct(
            &block,
            2,
            8,
            0,
            Transform::Bypass,
            &flat,
            0,
            &mut scratch,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, block);
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
        for (depth, extended) in [
            (8, false),
            (10, false),
            (12, false),
            (8, true),
            (10, true),
            (12, true),
        ] {
            let range = if extended { (depth + 6).max(15) } else { 15 };
            let minimum = -(1i64 << range);
            let maximum = (1i64 << range) - 1;
            for qp in 0..=51 + 6 * (depth - 8) {
                let mut block = [0i32; 16];
                for value in &mut block {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *value = ((u64::from(state) % (1u64 << (range + 1))) as i64 + minimum) as i32;
                }
                let divisor = 2i64.pow(u32::from(depth + 12 - range));
                let multiplier = [40, 45, 51, 57, 64, 72][usize::from(qp % 6)]
                    * 16
                    * 2i64.pow(u32::from(qp / 6));
                let scaled = block.map(|v| {
                    ((i64::from(v) * multiplier + divisor / 2).div_euclid(divisor))
                        .clamp(minimum, maximum)
                });
                let mut intermediate = [0i64; 16];
                for x in 0..4 {
                    let values = inverse(std::array::from_fn(|y| scaled[y * 4 + x]));
                    for y in 0..4 {
                        intermediate[y * 4 + x] =
                            (values[y] + 64).div_euclid(128).clamp(minimum, maximum);
                    }
                }
                let divisor = 2i64.pow(u32::from((20 - depth).max(if extended { 11 } else { 0 })));
                let mut expected = [0i32; 16];
                for y in 0..4 {
                    let values = inverse(std::array::from_fn(|x| intermediate[y * 4 + x]));
                    for x in 0..4 {
                        expected[y * 4 + x] = (values[x] + divisor / 2).div_euclid(divisor) as i32;
                    }
                }
                let mut scratch = Vec::new();
                let mut out = Vec::new();
                reconstruct_with_precision(
                    &block,
                    2,
                    depth,
                    qp,
                    Transform::Dct,
                    &flat,
                    0,
                    extended,
                    &mut scratch,
                    &mut out,
                )
                .unwrap();
                assert_eq!(out, expected);
            }
        }
    }
    #[test]
    fn nonflat_scaling_changes_ac_and_extreme_skip_is_bounded() {
        let flat = ScalingLists::flat();
        let defaults = ScalingLists::default();
        let mut block = [0; 64];
        block[63] = 128;
        let mut scratch = Vec::new();
        let mut out_a = Vec::new();
        let mut out_b = Vec::new();
        reconstruct(
            &block,
            3,
            8,
            12,
            Transform::Dct,
            &flat,
            0,
            &mut scratch,
            &mut out_a,
        )
        .unwrap();
        reconstruct(
            &block,
            3,
            8,
            12,
            Transform::Dct,
            &defaults,
            0,
            &mut scratch,
            &mut out_b,
        )
        .unwrap();
        assert_ne!(out_a, out_b);
        for (value, expected) in [(32767, 4096), (-32768, -4096)] {
            let mut out = Vec::new();
            reconstruct(
                &[value; 16],
                2,
                10,
                63,
                Transform::Skip,
                &flat,
                0,
                &mut scratch,
                &mut out,
            )
            .unwrap();
            assert_eq!(out, [expected; 16]);
        }
    }
    #[test]
    fn rejects_invalid_inputs_before_shifting_or_indexing() {
        let flat = ScalingLists::flat();
        let mut scratch = Vec::new();
        let mut out = Vec::new();
        for (log, depth, qp, id) in [
            (0, 8, 0, 0),
            (6, 8, 0, 0),
            (2, 7, 0, 0),
            (2, 17, 0, 0),
            (2, 8, 52, 0),
            (2, 10, 64, 0),
            (2, 8, 0, 6),
        ] {
            assert!(
                reconstruct(
                    &[0; 16],
                    log,
                    depth,
                    qp,
                    Transform::Dct,
                    &flat,
                    id,
                    &mut scratch,
                    &mut out
                )
                .is_err()
            );
        }
        assert!(
            reconstruct(
                &[0; 15],
                2,
                8,
                0,
                Transform::Dct,
                &flat,
                0,
                &mut scratch,
                &mut out
            )
            .is_err()
        );
        assert!(
            reconstruct(
                &[i32::MAX; 16],
                2,
                8,
                0,
                Transform::Dct,
                &flat,
                0,
                &mut scratch,
                &mut out
            )
            .is_err()
        );
        for log in 3..=5 {
            let coefficients = vec![16; 1 << (2 * log)];
            for depth in [8, 10, 12] {
                reconstruct(
                    &coefficients,
                    log,
                    depth,
                    0,
                    Transform::Skip,
                    &ScalingLists::default(),
                    0,
                    &mut scratch,
                    &mut out,
                )
                .unwrap();
                let expected = if depth == 12 && log == 5 { 12 } else { 10 };
                assert!(out.iter().all(|&value| value == expected));
            }
        }
        for mode in [Transform::Dst4] {
            assert!(
                reconstruct(&[0; 64], 3, 8, 0, mode, &flat, 0, &mut scratch, &mut out).is_err()
            );
        }
    }
}
