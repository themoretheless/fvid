//! Owned Annex E AHT scalar/GAQ reconstruction and six-block inverse DCT.
//! Normative VQ codebooks and GAQ cover every high-efficiency allocation.
use crate::codec::ac3_decoder::Bits;
use crate::{Result, invalid};
#[path = "eac3_aht_tables.rs"]
mod tables;
pub(crate) const HEBAP: [u8; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 8, 8, 8, 9, 9, 9, 10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12,
    13, 13, 13, 13, 14, 14, 14, 14, 15, 15, 15, 15, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18,
    18, 18, 18, 18, 19, 19, 19, 19, 19, 19, 19, 19, 19,
];
const WIDTH: [usize; 20] = [
    0, 2, 3, 4, 5, 7, 8, 9, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 16,
];
// Table E.2.6, Q15 signed fractional remapping constants.
const UNITY: [i32; 12] = [
    0x1249, 0x0889, 0x0421, 0x0208, 0x0102, 0x0081, 0x0040, 0x0020, 0x0010, 0x0008, 0x0002, 0,
];
const LARGE_A: [[i16; 2]; 9] = [
    [-10923, -4681],
    [-14043, -6554],
    [-15292, -7399],
    [-15855, -7802],
    [-16124, -7998],
    [-16255, -8096],
    [-16320, -8144],
    [-16352, -8168],
    [-16368, -8180],
];
const LARGE_B: [[i16; 2]; 9] = [
    [-5461, -1170],
    [-11703, -4915],
    [-14199, -6606],
    [-15327, -7412],
    [-15864, -7805],
    [-16126, -7999],
    [-16255, -8096],
    [-16320, -8144],
    [-16352, -8168],
];
fn signed(bits: &mut Bits<'_>, width: usize) -> i32 {
    let value = bits.take(width);
    (value << (32 - width)) >> (32 - width)
}
pub(super) fn inverse(input: [f32; 6]) -> [f32; 6] {
    std::array::from_fn(|m| {
        let mut value = f64::from(input[0]);
        for j in 1..6 {
            value += f64::from(input[j])
                * std::f64::consts::SQRT_2
                * ((2 * m + 1) as f64 * j as f64 * std::f64::consts::PI / 12.0).cos();
        }
        value as f32
    })
}
fn scalar(bits: &mut Bits<'_>, hebap: usize, gain: usize) -> f32 {
    let m = WIDTH[hebap];
    let small = m - gain;
    let q = signed(bits, small);
    if gain != 0 && q == -(1 << (small - 1)) {
        let width = m - (2 - gain);
        let q = signed(bits, width);
        let x = q as f32 / (1u32 << (width - 1)) as f32;
        let a = f32::from(LARGE_A[hebap - 8][gain - 1]) / 32768.0;
        let b = if q >= 0 {
            1.0 / (1u32 << gain) as f32
        } else {
            f32::from(LARGE_B[hebap - 8][gain - 1]) / 32768.0
        };
        x * (1.0 + a) + b
    } else {
        let x = q as f32 / (1u32 << (m - 1)) as f32;
        if gain == 0 {
            x * (1.0 + UNITY[hebap - 8] as f32 / 32768.0)
        } else {
            x
        }
    }
}
pub(super) fn read_plane(
    bits: &mut Bits<'_>,
    pointers: &[u8],
    mut noise: impl FnMut() -> f32,
) -> Result<Vec<[f32; 6]>> {
    if pointers.len() > 256 || pointers.iter().any(|&p| p > 19) {
        return Err(invalid("invalid E-AC-3 AHT allocation"));
    }
    let mode = bits.take(2) as usize;
    let end = if mode < 2 { 12 } else { 17 };
    let mut gains = [0usize; 256];
    let active: Vec<usize> = pointers
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| (mode != 0 && p > 7 && usize::from(p) < end).then_some(i))
        .collect();
    if mode == 3 {
        for group in active.chunks(3) {
            let packed = bits.take(5) as usize;
            if packed > 26 {
                return Err(invalid("reserved E-AC-3 AHT gain group"));
            }
            for (index, divisor) in group.iter().zip([9, 3, 1]) {
                gains[*index] = (packed / divisor) % 3;
            }
        }
    } else if mode != 0 {
        for i in active {
            gains[i] = (bits.take(1) as usize) * (mode);
        }
    }
    let mut out = Vec::new();
    out.try_reserve_exact(pointers.len())
        .map_err(|_| invalid("cannot allocate E-AC-3 AHT spectrum"))?;
    for (i, &p) in pointers.iter().enumerate() {
        let spectrum = match p {
            0 => std::array::from_fn(|_| noise()),
            1..=7 => {
                let index = bits.take(WIDTH[usize::from(p)]) as usize;
                let table: &[[i16; 6]] = match p {
                    1 => &tables::VQ1,
                    2 => &tables::VQ2,
                    3 => &tables::VQ3,
                    4 => &tables::VQ4,
                    5 => &tables::VQ5,
                    6 => &tables::VQ6,
                    _ => &tables::VQ7,
                };
                table[index].map(|v| f32::from(v) / 32768.0)
            }
            _ => std::array::from_fn(|_| scalar(bits, usize::from(p), gains[i])),
        };
        out.push(inverse(spectrum));
    }
    if bits.overrun {
        return Err(invalid("truncated E-AC-3 AHT spectrum"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(words: &[(i32, usize)]) -> Vec<u8> {
        let count: usize = words.iter().map(|x| x.1).sum();
        let mut out = vec![0; count.div_ceil(8)];
        let mut at = 0;
        for &(value, width) in words {
            for i in (0..width).rev() {
                out[at / 8] |= (((value as u32 >> i) & 1) as u8) << (7 - at % 8);
                at += 1;
            }
        }
        out
    }
    #[test]
    fn inverse_dct_literal_basis_and_energy_are_normalized() {
        assert_eq!(inverse([0.25, 0., 0., 0., 0., 0.]), [0.25; 6]);
        let output = inverse([0., 1., 0., 0., 0., 0.]);
        let literal = [1.3660254, 1., 0.3660254, -0.3660254, -1., -1.3660254];
        for (a, b) in output.into_iter().zip(literal) {
            assert!((a - b).abs() < 0.0000002);
        }
        for i in 0..6 {
            for j in 0..6 {
                let mut a = [0.; 6];
                a[i] = 1.;
                let a = inverse(a);
                let mut b = [0.; 6];
                b[j] = 1.;
                let b = inverse(b);
                let dot: f32 = a.into_iter().zip(b).map(|(a, b)| a * b).sum();
                assert!(
                    (dot - if i == j { 6. } else { 0. }).abs() < 0.000002,
                    "basis {i},{j}"
                );
            }
        }
    }
    #[test]
    fn every_normative_vq_codeword_preserves_the_six_dimensional_spectrum() {
        let books: [&[[i16; 6]]; 7] = [
            &tables::VQ1,
            &tables::VQ2,
            &tables::VQ3,
            &tables::VQ4,
            &tables::VQ5,
            &tables::VQ6,
            &tables::VQ7,
        ];
        for (book, rows) in books.into_iter().enumerate() {
            for (index, row) in rows.iter().enumerate() {
                let bytes = pack(&[(0, 2), (index as i32, WIDTH[book + 1]), (0x55, 7)]);
                let mut bits = Bits::at(&bytes, 0);
                let decoded = read_plane(&mut bits, &[(book + 1) as u8], || {
                    panic!("VQ requested noise")
                })
                .unwrap()[0];
                assert_eq!(bits.take(7), 0x55);
                // Forward projection independently recovers each Q15 codebook coordinate.
                for j in 0..6 {
                    let recovered: f64 = decoded
                        .iter()
                        .enumerate()
                        .map(|(m, &v)| {
                            f64::from(v)
                                * if j == 0 {
                                    1.
                                } else {
                                    std::f64::consts::SQRT_2
                                        * ((2 * m + 1) as f64 * j as f64 * std::f64::consts::PI
                                            / 12.)
                                            .cos()
                                }
                        })
                        .sum::<f64>()
                        / 6.;
                    assert!(
                        (recovered - f64::from(row[j]) / 32768.).abs() < 0.0000002,
                        "book {} index {index} coordinate {j}",
                        book + 1
                    );
                }
            }
        }
    }
    #[test]
    fn gain_adaptive_small_large_and_ungrouped_paths_consume_exact_words() {
        // Literal Q15 remap results at hebap 8: small gain-2/4 values, positive
        // and negative large escapes, and unamplified symmetric quantization.
        for (gain, words, expected) in [
            (0, vec![(1, 3)], 0.2857132f32),
            (1, vec![(1, 2)], 0.25),
            (1, vec![(-2, 2), (0, 2)], 0.5),
            (1, vec![(-2, 2), (-2, 2)], -0.8333129883),
            (2, vec![(-1, 1), (0, 3)], 0.25),
            (2, vec![(-1, 1), (-4, 3)], -0.8928527832),
        ] {
            let mut words = words;
            words.push((0x55, 7));
            let bytes = pack(&words);
            let mut bits = Bits::at(&bytes, 0);
            let got = scalar(&mut bits, 8, gain);
            assert!(
                (got - expected).abs() < 0.0000001,
                "gain {gain}: {got} vs {expected}"
            );
            assert_eq!(bits.take(7), 0x55);
        }
        for mode in 0..4 {
            let mut words = vec![(mode, 2)];
            if mode == 1 || mode == 2 {
                words.push((0, 1));
            }
            if mode == 3 {
                words.push((0, 5));
            }
            words.extend([(1, 3); 6]);
            words.push((0x55, 7));
            let bytes = pack(&words);
            let mut bits = Bits::at(&bytes, 0);
            let got = read_plane(&mut bits, &[8], || panic!("scalar requested noise")).unwrap();
            assert_eq!(bits.take(7), 0x55);
            let input = inverse([0.2857132; 6]);
            for (a, b) in got[0].iter().zip(input) {
                assert!((a - b).abs() < 0.000001);
            }
        }
    }
    #[test]
    fn reserved_gain_groups_truncated_data_and_invalid_allocations_are_errors() {
        let bytes = pack(&[(3, 2), (27, 5)]);
        let mut bits = Bits::at(&bytes, 0);
        assert!(
            read_plane(&mut bits, &[8], || 0.)
                .err()
                .unwrap()
                .to_string()
                .contains("gain group")
        );
        let mut bits = Bits::at(&[], 0);
        assert!(read_plane(&mut bits, &[19], || 0.).is_err());
        let mut bits = Bits::at(&[], 0);
        assert!(read_plane(&mut bits, &[20], || 0.).is_err());
        let mut bits = Bits::at(&[], 0);
        assert!(read_plane(&mut bits, &[0; 257], || 0.).is_err());
    }
}
