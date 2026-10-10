//! H.264 4x4 inverse scan, scaling and integer reconstruction (8.5.6, 8.5.12).
//! Entropy decoding supplies levels; intra/inter prediction supplies the predictor.
use crate::{Result, invalid};

pub const FRAME_SCAN_4X4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
pub const FIELD_SCAN_4X4: [usize; 16] = [0, 4, 1, 8, 12, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];
const NORMALIZATION: [[i64; 3]; 6] = [
    [10, 16, 13],
    [11, 18, 14],
    [13, 20, 16],
    [14, 23, 18],
    [16, 25, 20],
    [18, 29, 23],
];

pub fn inverse_scan_4x4(levels: &[i32; 16], field: bool) -> [i32; 16] {
    let scan = if field {
        &FIELD_SCAN_4X4
    } else {
        &FRAME_SCAN_4X4
    };
    let mut block = [0; 16];
    for (i, &index) in scan.iter().enumerate() {
        block[index] = levels[i];
    }
    block
}
fn validate(qp: u8, bit_depth: u8, weights: &[u8; 16]) -> Result<()> {
    if !(8..=14).contains(&bit_depth) || u16::from(qp) > 51 + 6 * (u16::from(bit_depth) - 8) {
        return Err(invalid("AVC QP or bit depth out of range"));
    }
    if weights.contains(&0) {
        return Err(invalid("AVC scaling weights must be nonzero"));
    }
    Ok(())
}
fn scale(level: i32, qp: u8, weight: u8, index: usize) -> i64 {
    let row = index / 4;
    let col = index % 4;
    let category = if row % 2 == 0 && col % 2 == 0 {
        0
    } else if row % 2 == 1 && col % 2 == 1 {
        1
    } else {
        2
    };
    let value = i64::from(level) * i64::from(weight) * NORMALIZATION[usize::from(qp % 6)][category];
    let shift = qp / 6;
    if shift >= 4 {
        value << (shift - 4)
    } else {
        (value + (1i64 << (3 - shift))) >> (4 - shift)
    }
}
fn butterfly(v: [i64; 4]) -> [i64; 4] {
    let even0 = v[0] + v[2];
    let even1 = v[0] - v[2];
    let odd0 = (v[1] >> 1) - v[3];
    let odd1 = v[1] + (v[3] >> 1);
    [even0 + odd1, even1 + odd0, even1 - odd0, even0 - odd1]
}
/// Levels and scaling weights are in raster order. QP includes the bit-depth offset.
/// `dc` is the already-scaled DC from the separate Intra16x16/chroma DC transform.
pub fn residual_4x4(
    levels: &[i32; 16],
    qp: u8,
    bit_depth: u8,
    weights: &[u8; 16],
    dc: Option<i32>,
) -> Result<[i32; 16]> {
    validate(qp, bit_depth, weights)?;
    let mut block = [0i64; 16];
    for i in 0..16 {
        block[i] = scale(levels[i], qp, weights[i], i);
    }
    if let Some(dc) = dc {
        block[0] = i64::from(dc);
    }
    for row in block.chunks_exact_mut(4) {
        let transformed = butterfly([row[0], row[1], row[2], row[3]]);
        row.copy_from_slice(&transformed);
    }
    let mut residual = [0; 16];
    for x in 0..4 {
        let column = butterfly([block[x], block[4 + x], block[8 + x], block[12 + x]]);
        for y in 0..4 {
            residual[y * 4 + x] = i32::try_from((column[y] + 32) >> 6)
                .map_err(|_| invalid("AVC residual exceeds numeric range"))?;
        }
    }
    Ok(residual)
}
/// Add a decoded residual and clip to the component bit depth before deblocking.
pub fn reconstruct_4x4(
    prediction: &[u16; 16],
    residual: &[i32; 16],
    bit_depth: u8,
) -> Result<[u16; 16]> {
    reconstruct(prediction, residual, bit_depth)
}
pub(super) fn reconstruct<const N: usize>(
    prediction: &[u16; N],
    residual: &[i32; N],
    bit_depth: u8,
) -> Result<[u16; N]> {
    if !(8..=14).contains(&bit_depth) {
        return Err(invalid("AVC bit depth out of range"));
    }
    let max = (1u16 << bit_depth) - 1;
    if prediction.iter().any(|&p| p > max) {
        return Err(invalid("AVC predictor exceeds bit depth"));
    }
    let mut result = [0; N];
    for i in 0..N {
        result[i] =
            (i64::from(prediction[i]) + i64::from(residual[i])).clamp(0, i64::from(max)) as u16;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_orders_are_bijections_with_known_positions() {
        let levels = std::array::from_fn(|i| i as i32);
        assert_eq!(
            inverse_scan_4x4(&levels, false),
            [0, 1, 5, 6, 2, 4, 7, 12, 3, 8, 11, 13, 9, 10, 14, 15]
        );
        assert_eq!(
            inverse_scan_4x4(&levels, true),
            [0, 2, 8, 12, 1, 5, 9, 13, 3, 6, 10, 14, 4, 7, 11, 15]
        );
    }
    #[test]
    fn zero_dc_and_clipping_vectors() {
        assert_eq!(
            residual_4x4(&[0; 16], 0, 8, &[16; 16], None).unwrap(),
            [0; 16]
        );
        let mut levels = [0; 16];
        levels[0] = 64;
        // Flat scaling: at QP 0 the DC normalization factor is 10, hence 640/64.
        assert_eq!(
            residual_4x4(&levels, 0, 8, &[16; 16], None).unwrap(),
            [10; 16]
        );
        levels[0] = -64;
        assert_eq!(
            residual_4x4(&levels, 0, 8, &[16; 16], None).unwrap(),
            [-10; 16]
        );
        assert_eq!(
            residual_4x4(&levels, 0, 8, &[16; 16], Some(128)).unwrap(),
            [2; 16]
        );
        assert_eq!(
            reconstruct_4x4(&[250; 16], &[10; 16], 8).unwrap(),
            [255; 16]
        );
        assert_eq!(reconstruct_4x4(&[5; 16], &[-10; 16], 10).unwrap(), [0; 16]);
        assert!(reconstruct_4x4(&[1024; 16], &[0; 16], 10).is_err());
    }
    #[test]
    fn transform_matches_matrix_for_even_intermediates() {
        // Independent matrix formulation: C rows correspond to the inverse transform basis.
        // Multiples of 16 make both half-shifts exact, so no rounding is hidden here.
        let c = [
            [2i64, 2, 2, 1],
            [2, 1, -2, -2],
            [2, -1, -2, 2],
            [2, -2, 2, -1],
        ];
        for position in 0..16 {
            let mut levels = [0; 16];
            levels[position] = 64;
            let actual = residual_4x4(&levels, 0, 8, &[16; 16], None).unwrap();
            let normalization = if position / 4 % 2 == 0 && position % 4 % 2 == 0 {
                10
            } else if position / 4 % 2 == 1 && position % 4 % 2 == 1 {
                16
            } else {
                13
            };
            for y in 0..4 {
                for x in 0..4 {
                    let scaled = 64 * normalization;
                    let expected = (c[y][position / 4] * scaled * c[x][position % 4] / 4 + 32) >> 6;
                    assert_eq!(
                        i64::from(actual[y * 4 + x]),
                        expected,
                        "coefficient {position}, pixel {x},{y}"
                    );
                }
            }
        }
    }
    #[test]
    fn rejects_invalid_parameters_and_bounds_extreme_coefficients() {
        assert!(residual_4x4(&[0; 16], 52, 8, &[16; 16], None).is_err());
        assert!(residual_4x4(&[0; 16], 0, 7, &[16; 16], None).is_err());
        assert!(residual_4x4(&[0; 16], 0, 8, &[0; 16], None).is_err());
        for value in [i32::MIN, i32::MAX] {
            let _ = residual_4x4(&[value; 16], 87, 14, &[255; 16], None);
        }
    }
}

fn hadamard4(v: [i64; 4]) -> [i64; 4] {
    let a = v[0] + v[2];
    let b = v[0] - v[2];
    let c = v[1] - v[3];
    let d = v[1] + v[3];
    [a + d, b + c, b - c, a - d]
}
/// Separate Intra16x16 luma DC inverse transform (8.5.10).
/// The result is passed as `dc` to each residual_4x4 call, in raster block order.
pub fn luma_dc_4x4(levels: &[i32; 16], qp: u8, bit_depth: u8, weight: u8) -> Result<[i32; 16]> {
    validate(qp, bit_depth, &[weight; 16])?;
    let mut block = levels.map(i64::from);
    for row in block.chunks_exact_mut(4) {
        let output = hadamard4([row[0], row[1], row[2], row[3]]);
        row.copy_from_slice(&output);
    }
    let mut output = [0; 16];
    let normalization = NORMALIZATION[usize::from(qp % 6)][0] * i64::from(weight);
    for x in 0..4 {
        let column = hadamard4([block[x], block[4 + x], block[8 + x], block[12 + x]]);
        for y in 0..4 {
            let value = column[y] * normalization;
            let shift = qp / 6;
            let dc = if shift >= 6 {
                value << (shift - 6)
            } else {
                (value + (1i64 << (5 - shift))) >> (6 - shift)
            };
            output[y * 4 + x] =
                i32::try_from(dc).map_err(|_| invalid("AVC luma DC exceeds numeric range"))?;
        }
    }
    Ok(output)
}
/// Separate 4:2:0 chroma DC inverse transform (8.5.11), in raster block order.
/// QP must already include the chroma QP mapping and bit-depth offset.
pub fn chroma_dc_2x2(levels: &[i32; 4], qp: u8, bit_depth: u8, weight: u8) -> Result<[i32; 4]> {
    validate(qp, bit_depth, &[weight; 16])?;
    let v = levels.map(i64::from);
    let a = v[0] + v[1];
    let b = v[0] - v[1];
    let c = v[2] + v[3];
    let d = v[2] - v[3];
    let transformed = [a + c, b + d, a - c, b - d];
    let mut output = [0; 4];
    let scale = NORMALIZATION[usize::from(qp % 6)][0] * i64::from(weight);
    for i in 0..4 {
        output[i] = i32::try_from(((transformed[i] * scale) << (qp / 6)) >> 5)
            .map_err(|_| invalid("AVC chroma DC exceeds numeric range"))?;
    }
    Ok(output)
}

#[cfg(test)]
mod dc_tests {
    use super::*;
    #[test]
    fn qp_steps_scale_flat_dc_and_custom_weights() {
        let mut levels = [0; 16];
        levels[0] = 64;
        for qp in 0..=87 {
            let expected = [10, 11, 13, 14, 16, 18][usize::from(qp % 6)] << (qp / 6);
            assert_eq!(
                residual_4x4(&levels, qp, 14, &[16; 16], None).unwrap(),
                [expected; 16]
            );
        }
        assert_eq!(
            residual_4x4(&levels, 0, 8, &[32; 16], None).unwrap(),
            [20; 16]
        );
    }
    #[test]
    fn dc_hadamard_impulse_and_constant() {
        let mut levels = [0; 16];
        levels[0] = 64;
        assert_eq!(luma_dc_4x4(&levels, 0, 8, 16).unwrap(), [160; 16]);
        let mut expected = [0; 16];
        expected[0] = 2560;
        assert_eq!(luma_dc_4x4(&[64; 16], 0, 8, 16).unwrap(), expected);
        assert_eq!(chroma_dc_2x2(&[64, 0, 0, 0], 0, 8, 16).unwrap(), [320; 4]);
        assert_eq!(chroma_dc_2x2(&[64; 4], 0, 8, 16).unwrap(), [1280, 0, 0, 0]);
        assert_eq!(chroma_dc_2x2(&[-1, 0, 0, 0], 0, 8, 16).unwrap(), [-5; 4]);
    }
}

/// H.264 8.6 luma reconstruction for Extended-profile SP/SI macroblocks.
/// `levels` are raster-ordered parsed residual levels (not already dequantized).
/// `switching` selects SI/secondary SP; false selects primary SP.
/// Extended profile uses eight-bit samples and flat 4x4 scaling weights.
pub fn switching_luma_4x4(
    prediction: &[u16; 16],
    levels: &[i32; 16],
    qp: u8,
    qs: u8,
    switching: bool,
) -> Result<[u16; 16]> {
    validate(qp, 8, &[16; 16])?;
    validate(qs, 8, &[16; 16])?;
    if prediction.iter().any(|&v| v > 255) {
        return Err(invalid("AVC switching predictor exceeds bit depth"));
    }
    if levels.iter().any(|&v| !(-32768..=32767).contains(&v)) {
        return Err(invalid("AVC switching coefficient exceeds numeric range"));
    }
    let mut predicted = prediction.map(i64::from);
    for row in predicted.chunks_exact_mut(4) {
        row.copy_from_slice(&switching_forward([row[0], row[1], row[2], row[3]]));
    }
    for x in 0..4 {
        let c = switching_forward([
            predicted[x],
            predicted[4 + x],
            predicted[8 + x],
            predicted[12 + x],
        ]);
        for y in 0..4 {
            predicted[y * 4 + x] = c[y];
        }
    }
    let mut combined = [0; 16];
    for i in 0..16 {
        let row = i / 4;
        let col = i % 4;
        let category = if row % 2 == 0 && col % 2 == 0 {
            0
        } else if row % 2 == 1 && col % 2 == 1 {
            1
        } else {
            2
        };
        let value = if switching {
            predicted[i]
        } else {
            // Equations 8-416/417, flat LevelScale4x4 = 16 * normalization.
            predicted[i]
                + ((i64::from(levels[i])
                    * 16
                    * NORMALIZATION[usize::from(qp % 6)][category]
                    * [16, 25, 20][category]
                    << (qp / 6))
                    >> 10)
        };
        let quantized = value.signum()
            * ((value.abs() * SWITCHING_QUANT[usize::from(qs % 6)][category] + (1i64 << (14 + qs / 6)))
                >> (15 + qs / 6));
        combined[i] = i32::try_from(quantized + if switching { i64::from(levels[i]) } else { 0 })
            .map_err(|_| invalid("AVC switching coefficient exceeds numeric range"))?;
    }
    let output = residual_4x4(&combined, qs, 8, &[16; 16], None)?;
    reconstruct_4x4(&[0; 16], &output, 8)
}

fn switching_forward(v: [i64; 4]) -> [i64; 4] {
    let a = v[0] + v[3];
    let b = v[1] + v[2];
    let c = v[1] - v[2];
    let d = v[0] - v[3];
    [a + b, 2 * d + c, a - b, d - 2 * c]
}

const SWITCHING_QUANT: [[i64; 3]; 6] = [
    [13107, 5243, 8066],
    [11916, 4660, 7490],
    [10082, 4194, 6554],
    [9362, 3647, 5825],
    [8192, 3355, 5243],
    [7282, 2893, 4559],
];

/// Primary SP (sp_for_switch_flag = 0), H.264 8.6.1.2, one 8x8 chroma plane.
/// QPC/QSC are already mapped component QPs (0..39), including chroma offset.
/// DC levels and 4x4 AC blocks use raster block order; AC index zero must be zero.
/// Eight-bit Extended profile uses flat scaling lists.
pub fn primary_sp_chroma_420(
    prediction: &[u16; 64],
    dc_levels: &[i32; 4],
    ac_levels: &[[i32; 16]; 4],
    qp: u8,
    qs: u8,
) -> Result<[u16; 64]> {
    if qp > 39 || qs > 39 {
        return Err(invalid("AVC switching chroma QP out of range"));
    }
    if prediction.iter().any(|&v| v > 255) {
        return Err(invalid("AVC switching predictor exceeds bit depth"));
    }
    if ac_levels.iter().any(|v| v[0] != 0)
        || dc_levels
            .iter()
            .chain(ac_levels.iter().flatten())
            .any(|&v| !(-32768..=32767).contains(&v))
    {
        return Err(invalid(
            "AVC switching chroma coefficient exceeds numeric range",
        ));
    }
    fn hadamard(v: [i64; 4]) -> [i64; 4] {
        [
            v[0] + v[1] + v[2] + v[3],
            v[0] - v[1] + v[2] - v[3],
            v[0] + v[1] - v[2] - v[3],
            v[0] - v[1] - v[2] + v[3],
        ]
    }
    fn quant(value: i64, qs: u8, category: usize, dc: bool) -> i64 {
        let shift = 15 + qs / 6 + u8::from(dc);
        value.signum()
            * ((value.abs() * SWITCHING_QUANT[usize::from(qs % 6)][category]
                + (1i64 << (shift - 1)))
                >> shift)
    }
    let mut coefficients = [[0i64; 16]; 4];
    for block in 0..4 {
        let ox = (block % 2) * 4;
        let oy = (block / 2) * 4;
        let mut c = std::array::from_fn(|i| i64::from(prediction[(oy + i / 4) * 8 + ox + i % 4]));
        for row in c.chunks_exact_mut(4) {
            row.copy_from_slice(&switching_forward([row[0], row[1], row[2], row[3]]));
        }
        for x in 0..4 {
            let v = switching_forward([c[x], c[4 + x], c[8 + x], c[12 + x]]);
            for y in 0..4 {
                c[y * 4 + x] = v[y];
            }
        }
        coefficients[block] = c;
    }
    let predicted_dc = hadamard(coefficients.map(|c| c[0]));
    let mut quantized_dc = [0; 4];
    for i in 0..4 {
        let sum = predicted_dc[i]
            + ((i64::from(dc_levels[i]) * 16 * NORMALIZATION[usize::from(qp % 6)][0] * 16
                << (qp / 6))
                >> 9);
        quantized_dc[i] = quant(sum, qs, 0, true);
    }
    let dc = hadamard(quantized_dc);
    let mut output = [0; 64];
    for block in 0..4 {
        let mut levels = [0; 16];
        for i in 1..16 {
            let row = i / 4;
            let col = i % 4;
            let category = if row % 2 == 0 && col % 2 == 0 {
                0
            } else if row % 2 == 1 && col % 2 == 1 {
                1
            } else {
                2
            };
            let sum = coefficients[block][i]
                + ((i64::from(ac_levels[block][i])
                    * 16
                    * NORMALIZATION[usize::from(qp % 6)][category]
                    * [16, 25, 20][category]
                    << (qp / 6))
                    >> 10);
            levels[i] = i32::try_from(quant(sum, qs, category, false))
                .map_err(|_| invalid("AVC switching chroma coefficient exceeds numeric range"))?;
        }
        let scaled_dc = i32::try_from(
            (dc[block] * 16 * NORMALIZATION[usize::from(qs % 6)][0] << (qs / 6)) >> 5,
        )
        .map_err(|_| invalid("AVC switching chroma DC exceeds numeric range"))?;
        let samples = residual_4x4(&levels, qs, 8, &[16; 16], Some(scaled_dc))?;
        let ox = (block % 2) * 4;
        let oy = (block / 2) * 4;
        for i in 0..16 {
            output[(oy + i / 4) * 8 + ox + i % 4] = samples[i].clamp(0, 255) as u16;
        }
    }
    Ok(output)
}
