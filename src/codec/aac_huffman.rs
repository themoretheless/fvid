//! Owned AAC Huffman decoding over protocol codeword tables.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_huffman_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    fn packed(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        let mut count = 0;
        for &(value, n) in fields {
            for shift in (0..n).rev() {
                if count % 8 == 0 {
                    bytes.push(0);
                }
                *bytes.last_mut().unwrap() |= ((value >> shift & 1) as u8) << (7 - count % 8);
                count += 1;
            }
        }
        (bytes, count)
    }
    #[test]
    fn every_scalefactor_code_decodes_and_tables_are_prefix_free() {
        for book in 0..=11 {
            let (codes, lens) = if book == 0 {
                (&SCF_CODEBOOK_CODES[..], &SCF_CODEBOOK_LENS[..])
            } else {
                table(book).unwrap()
            };
            for (i, (&code, &len)) in codes.iter().zip(lens).enumerate() {
                for (j, (&other, &other_len)) in codes.iter().zip(lens).enumerate() {
                    if i != j && len <= other_len {
                        assert_ne!(code, other >> (other_len - len));
                    }
                }
                let (data, _) = packed(&[(code, len)]);
                let mut bits = BitReader::new(&data);
                if book == 0 {
                    assert_eq!(scalefactor(&mut bits).unwrap(), i as i16 - 60);
                } else {
                    assert_eq!(symbol(&mut bits, codes, lens).unwrap(), i);
                }
                assert_eq!(bits.position(), len as usize);
            }
        }
    }
    #[test]
    fn all_spectral_symbols_decode_with_signed_components() {
        for book in 1..=11 {
            let (codes, lens) = table(book).unwrap();
            let (width, radix, bias): (usize, usize, i16) = match book {
                1 | 2 => (4, 3, 1),
                3 | 4 => (4, 3, 0),
                5 | 6 => (2, 9, 4),
                7 | 8 => (2, 8, 0),
                9 | 10 => (2, 13, 0),
                _ => (2, 17, 0),
            };
            for i in 0..codes.len() {
                let mut fields = vec![(codes[i], lens[i])];
                let mut expected = [0i16; 4];
                for (k, value) in expected[..width].iter_mut().enumerate() {
                    let digit = (i / radix.pow((width - 1 - k) as u32)) % radix;
                    *value = digit as i16 - bias;
                    if bias == 0 && digit != 0 {
                        fields.push((1, 1));
                        *value = -*value;
                    }
                }
                if book == 11 {
                    for &v in &expected[..width] {
                        if v.abs() == 16 {
                            fields.extend([(0, 1), (0, 4)]);
                        }
                    }
                }
                let (bytes, count) = packed(&fields);
                let mut bits = BitReader::new(&bytes);
                assert_eq!(spectral(&mut bits, book).unwrap(), (expected, width));
                assert_eq!(bits.position(), count);
            }
        }
    }
    #[test]
    fn escape_extremes_and_truncation_are_transactional() {
        let (codes, lens) = table(11).unwrap();
        let index = 16 * 17 + 16;
        let (bytes, count) = packed(&[
            (codes[index], lens[index]),
            (1, 1),
            (0, 1),
            (255, 8),
            (0, 1),
            (4095, 12),
            (0, 1),
            (0, 4),
        ]);
        let mut bits = BitReader::new(&bytes);
        assert_eq!(spectral(&mut bits, 11).unwrap(), ([-8191, 16, 0, 0], 2));
        assert_eq!(bits.position(), count);
        for size in 0..bytes.len() - 1 {
            let mut bits = BitReader::new(&bytes[..size]);
            assert!(spectral(&mut bits, 11).is_err());
            assert_eq!(bits.position(), 0);
        }
        assert!(spectral(&mut BitReader::new(&bytes), 12).is_err());
    }
}
