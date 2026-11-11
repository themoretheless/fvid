//! H.264 inverse 8x8 transform, sections 8.5.7, 8.5.9 and 8.5.13.
pub use super::avc_8x8_tables::{FIELD_SCAN, FRAME_SCAN};
use crate::{Result, invalid};
const NORMALIZATION: [[i64; 6]; 6] = [
    [20, 18, 32, 19, 25, 24],
    [22, 19, 35, 21, 28, 26],
    [26, 23, 42, 24, 33, 31],
    [28, 25, 45, 26, 35, 33],
    [32, 28, 51, 30, 40, 38],
    [36, 32, 58, 34, 46, 43],
];
const CATEGORIES: [[usize; 4]; 4] = [[0, 3, 4, 3], [3, 1, 5, 1], [4, 5, 2, 5], [3, 1, 5, 1]];
pub fn inverse_scan_8x8(levels: &[i32; 64], field: bool) -> [i32; 64] {
    let scan = if field { &FIELD_SCAN } else { &FRAME_SCAN };
    let mut out = [0; 64];
    for (i, &position) in scan.iter().enumerate() {
        out[position] = levels[i];
    }
    out
}
fn butterfly(d: [i64; 8]) -> [i64; 8] {
    let e = [
        d[0] + d[4],
        -d[3] + d[5] - d[7] - (d[7] >> 1),
        d[0] - d[4],
        d[1] + d[7] - d[3] - (d[3] >> 1),
        (d[2] >> 1) - d[6],
        -d[1] + d[7] + d[5] + (d[5] >> 1),
        d[2] + (d[6] >> 1),
        d[3] + d[5] + d[1] + (d[1] >> 1),
    ];
    let f = [
        e[0] + e[6],
        e[1] + (e[7] >> 2),
        e[2] + e[4],
        e[3] + (e[5] >> 2),
        e[2] - e[4],
        (e[3] >> 2) - e[5],
        e[0] - e[6],
        e[7] - (e[1] >> 2),
    ];
    [
        f[0] + f[7],
        f[2] + f[5],
        f[4] + f[3],
        f[6] + f[1],
        f[6] - f[1],
        f[4] - f[3],
        f[2] - f[5],
        f[0] - f[7],
    ]
}
/// Coefficients and weights are raster ordered; qp includes the bit-depth offset.
/// Uses wide intermediates and rejects output overflow rather than wrapping.
pub fn residual_8x8(
    levels: &[i32; 64],
    qp: u8,
    depth: u8,
    weights: &[u8; 64],
) -> Result<[i32; 64]> {
    if !(8..=14).contains(&depth)
        || u16::from(qp) > 51 + 6 * (u16::from(depth) - 8)
        || weights.contains(&0)
    {
        return Err(invalid("invalid AVC 8x8 quantization parameters"));
    }
    let mut block = [0i64; 64];
    for i in 0..64 {
        let factor = NORMALIZATION[usize::from(qp % 6)][CATEGORIES[i / 8 % 4][i % 4]];
        let value = i64::from(levels[i]) * i64::from(weights[i]) * factor;
        let shift = qp / 6;
        block[i] = if shift >= 6 {
            value << (shift - 6)
        } else {
            (value + (1i64 << (5 - shift))) >> (6 - shift)
        };
    }
    for row in block.chunks_exact_mut(8) {
        let transformed = butterfly((&*row).try_into().unwrap());
        row.copy_from_slice(&transformed);
    }
    let mut out = [0; 64];
    for x in 0..8 {
        let column = butterfly(std::array::from_fn(|y| block[y * 8 + x]));
        for y in 0..8 {
            out[y * 8 + x] = i32::try_from((column[y] + 32) >> 6)
                .map_err(|_| invalid("AVC 8x8 residual exceeds numeric range"))?;
        }
    }
    Ok(out)
}
pub fn reconstruct_8x8(
    prediction: &[u16; 64],
    residual: &[i32; 64],
    depth: u8,
) -> Result<[u16; 64]> {
    super::avc_transform::reconstruct(prediction, residual, depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scans_are_bijective_and_match_known_diagonals() {
        for scan in [FRAME_SCAN, FIELD_SCAN] {
            let mut sorted = scan;
            sorted.sort();
            assert_eq!(sorted, std::array::from_fn(|i| i));
        }
        assert_eq!(&FRAME_SCAN[..10], &[0, 1, 8, 16, 9, 2, 3, 10, 17, 24]);
        assert_eq!(&FIELD_SCAN[..8], &[0, 8, 16, 1, 9, 24, 32, 17]);
        let levels = std::array::from_fn(|i| i as i32);
        for field in [false, true] {
            let out = inverse_scan_8x8(&levels, field);
            for (i, &position) in if field { &FIELD_SCAN } else { &FRAME_SCAN }
                .iter()
                .enumerate()
            {
                assert_eq!(out[position], i as i32);
            }
        }
    }
    #[test]
    fn independent_matrix_basis_vectors() {
        // Exact rational basis multiplied by 8. Large impulses avoid intermediate
        // fractional rounding, so a full matrix product is an independent oracle.
        let c: [[i64; 8]; 8] = [
            [8, 12, 8, 10, 8, 6, 4, 3],
            [8, 10, 4, -3, -8, -12, -8, -6],
            [8, 6, -4, -12, -8, 3, 8, 10],
            [8, 3, -8, -6, 8, 10, -4, -12],
            [8, -3, -8, 6, 8, -10, -4, 12],
            [8, -6, -4, 12, -8, -3, 8, -10],
            [8, -10, 4, 3, -8, 12, -8, 6],
            [8, -12, 8, -10, 8, -6, 4, -3],
        ];
        for position in 0..64 {
            for sign in [-1, 1] {
                let mut levels = [0; 64];
                levels[position] = 4096 * sign;
                let actual = residual_8x8(&levels, 0, 8, &[16; 64]).unwrap();
                let factor = NORMALIZATION[0][CATEGORIES[position / 8 % 4][position % 4]];
                for y in 0..8 {
                    for x in 0..8 {
                        let scaled = i64::from(levels[position]) * factor / 4;
                        let expected =
                            (c[y][position / 8] * scaled * c[x][position % 8] / 64 + 32) >> 6;
                        assert_eq!(
                            i64::from(actual[y * 8 + x]),
                            expected,
                            "position={position}, sign={sign}, x={x}, y={y}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn dc_qp_rounding_clipping_and_invalid_parameters() {
        for qp in 0..=87 {
            let mut levels = [0; 64];
            levels[0] = 256;
            let expected = [20, 22, 26, 28, 32, 36][usize::from(qp % 6)] << (qp / 6);
            assert_eq!(
                residual_8x8(&levels, qp, 14, &[16; 64]).unwrap(),
                [expected; 64]
            );
        }
        assert_eq!(residual_8x8(&[0; 64], 0, 8, &[16; 64]).unwrap(), [0; 64]);
        assert!(residual_8x8(&[0; 64], 52, 8, &[16; 64]).is_err());
        assert!(residual_8x8(&[0; 64], 0, 7, &[16; 64]).is_err());
        assert!(residual_8x8(&[0; 64], 0, 8, &[0; 64]).is_err());
        assert!(residual_8x8(&[i32::MAX; 64], 87, 14, &[255; 64]).is_err());
        assert_eq!(
            reconstruct_8x8(&[1000; 64], &[100; 64], 10).unwrap(),
            [1023; 64]
        );
        assert_eq!(reconstruct_8x8(&[4; 64], &[-9; 64], 10).unwrap(), [0; 64]);
    }
}
