//! H.264 CAVLC residual block decoding (9.2). Coefficients are returned in scan order.
use super::{bits::BitReader, cavlc_tables::*};
use crate::{Result, invalid};
#[derive(Debug, PartialEq, Eq)]
pub struct Residual {
    pub coefficients: [i32; 16],
    pub total_coefficients: u8,
    pub trailing_ones: u8,
}
fn vlc(b: &mut BitReader<'_>, table: &[Code]) -> Result<u8> {
    let mut code = 0u16;
    for len in 1..=16 {
        code = (code << 1) | u16::from(b.bit()?);
        if let Some(&(_, _, value)) = table.iter().find(|&&(bits, n, _)| n == len && bits == code) {
            return Ok(value);
        }
    }
    Err(invalid("invalid CAVLC codeword"))
}
/// nC is derived from available left/top blocks; -1 / -2 select chroma DC 420 / 422.
/// For AC-only blocks use max_coefficients=15; position zero then denotes the first AC.
pub fn read_residual(b: &mut BitReader<'_>, nc: i8, max_coefficients: u8) -> Result<Residual> {
    let table = match (nc, max_coefficients) {
        (-1, 4) => 4,
        (-2, 8) => 5,
        (0..=1, 15 | 16) => 0,
        (2..=3, 15 | 16) => 1,
        (4..=7, 15 | 16) => 2,
        (8..=16, 15 | 16) => 3,
        _ => return Err(invalid("invalid CAVLC context or block size")),
    };
    let token = vlc(b, COEFF_TOKEN[table])?;
    let total = token >> 2;
    let trailing = token & 3;
    if total > max_coefficients || trailing > total {
        return Err(invalid("CAVLC coefficient count exceeds block"));
    }
    let mut result = Residual {
        coefficients: [0; 16],
        total_coefficients: total,
        trailing_ones: trailing,
    };
    if total == 0 {
        return Ok(result);
    }
    let mut levels = [0i32; 16];
    let mut runs = [0u8; 16];
    for level in levels.iter_mut().take(trailing as usize) {
        *level = if b.bit()? { -1 } else { 1 };
    }
    let mut suffix_length = if total > 10 && trailing < 3 { 1u8 } else { 0 };
    for i in trailing..total {
        let mut prefix = 0u8;
        while !b.bit()? {
            prefix += 1;
            if prefix > 31 {
                return Err(invalid("CAVLC level prefix exceeds numeric limit"));
            }
        }
        let suffix_bits = if prefix == 14 && suffix_length == 0 {
            4
        } else if prefix >= 15 {
            prefix - 3
        } else {
            suffix_length
        };
        let suffix = b.read(suffix_bits)?;
        let mut code = (i64::from(prefix.min(15)) << suffix_length) + i64::from(suffix);
        if prefix >= 15 && suffix_length == 0 {
            code += 15;
        }
        if prefix >= 16 {
            code += (1i64 << (prefix - 3)) - 4096;
        }
        if i == trailing && trailing < 3 {
            code += 2;
        }
        let level = if code & 1 == 0 {
            (code + 2) >> 1
        } else {
            (-code - 1) >> 1
        };
        levels[i as usize] =
            i32::try_from(level).map_err(|_| invalid("CAVLC coefficient overflow"))?;
        if suffix_length == 0 {
            suffix_length = 1;
        }
        if level.abs() > (3i64 << (suffix_length - 1)) && suffix_length < 6 {
            suffix_length += 1;
        }
    }
    let mut zeros = if total < max_coefficients {
        let tables = match max_coefficients {
            4 => CHROMA_420_ZEROS,
            8 => CHROMA_422_ZEROS,
            _ => TOTAL_ZEROS,
        };
        let zeros = vlc(b, tables[usize::from(total - 1)])?;
        if zeros > max_coefficients - total {
            return Err(invalid("CAVLC zeros exceed block"));
        }
        zeros
    } else {
        0
    };
    for i in 0..total - 1 {
        if zeros == 0 {
            break;
        }
        let run = vlc(b, RUN_BEFORE[usize::from(zeros.min(7) - 1)])?;
        if run > zeros {
            return Err(invalid("CAVLC run exceeds remaining zeros"));
        }
        runs[i as usize] = run;
        zeros -= run;
    }
    runs[usize::from(total - 1)] = zeros;
    let mut position = 0usize;
    for i in (0..usize::from(total)).rev() {
        position += usize::from(runs[i]);
        if position >= usize::from(max_coefficients) {
            return Err(invalid("CAVLC coefficient position outside block"));
        }
        result.coefficients[position] = levels[i];
        position += 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(text: &str) -> Vec<u8> {
        let text: String = text.chars().filter(|c| matches!(c, '0' | '1')).collect();
        let mut bytes = vec![0; text.len().div_ceil(8)];
        for (i, b) in text.bytes().enumerate() {
            bytes[i / 8] |= (b - b'0') << (7 - i % 8);
        }
        bytes
    }
    #[test]
    fn every_normative_codeword_decodes_without_consuming_following_bits() {
        for group in [
            COEFF_TOKEN,
            TOTAL_ZEROS,
            CHROMA_420_ZEROS,
            CHROMA_422_ZEROS,
            RUN_BEFORE,
        ] {
            for table in group {
                for &(code, len, value) in *table {
                    let data = ((u32::from(code) << (32 - len)) | ((1u32 << (32 - len)) - 1))
                        .to_be_bytes();
                    let mut reader = BitReader::new(&data);
                    assert_eq!(vlc(&mut reader, table).unwrap(), value);
                    assert_eq!(reader.position(), usize::from(len));
                    for &(other, n, _) in *table {
                        if n > len {
                            assert_ne!(other >> (n - len), code);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn known_level_and_run_vectors() {
        // coeff_token=(3,3), signs + - +, total_zeros=3, runs 2,0,1.
        let data = bits("00011 010 101 01 1");
        let mut b = BitReader::new(&data);
        let r = read_residual(&mut b, 0, 16).unwrap();
        assert_eq!(r.total_coefficients, 3);
        assert_eq!(r.trailing_ones, 3);
        assert_eq!(
            r.coefficients,
            [0, 1, -1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(b.position(), 14);
        let data = bits("000101 1 1");
        let mut b = BitReader::new(&data);
        assert_eq!(read_residual(&mut b, 0, 16).unwrap().coefficients[0], 2);
        assert_eq!(b.position(), 8);
        let data = bits("000101 01 1");
        let mut b = BitReader::new(&data);
        assert_eq!(read_residual(&mut b, 0, 16).unwrap().coefficients[0], -2);
        assert_eq!(b.position(), 9);
    }
    #[test]
    fn zeros_all_contexts_and_invalid_input() {
        for (nc, max, code) in [
            (0, 16, "1"),
            (2, 16, "11"),
            (4, 15, "1111"),
            (8, 16, "000011"),
            (-1, 4, "01"),
            (-2, 8, "1"),
        ] {
            let data = bits(code);
            let mut b = BitReader::new(&data);
            assert_eq!(
                read_residual(&mut b, nc, max).unwrap().coefficients,
                [0; 16]
            );
            assert_eq!(b.position(), code.len());
        }
        assert!(read_residual(&mut BitReader::new(&[]), 0, 16).is_err());
        assert!(read_residual(&mut BitReader::new(&[0; 32]), 0, 16).is_err());
        assert!(read_residual(&mut BitReader::new(&[255; 8]), -1, 16).is_err());
    }
    fn emit(text: &mut String, value: u64, length: u8) {
        text.push_str(&format!("{value:0width$b}", width = length as usize));
    }
    fn emit_vlc(text: &mut String, table: &[Code], value: u8) {
        let &(code, length, _) = table.iter().find(|&&(_, _, v)| v == value).unwrap();
        emit(text, u64::from(code), length);
    }
    // Test-only inverse mapper: searches representable prefix intervals rather than replaying the decoder.
    fn encode(values: &[i32], context: usize) -> String {
        let pairs: Vec<_> = values
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0)
            .map(|(i, &v)| (i, v))
            .rev()
            .collect();
        let total = pairs.len();
        let trailing = pairs
            .iter()
            .take(3)
            .take_while(|(_, v)| v.abs() == 1)
            .count();
        let mut text = String::new();
        emit_vlc(
            &mut text,
            COEFF_TOKEN[context],
            (total * 4 + trailing) as u8,
        );
        if total == 0 {
            return text;
        }
        for &(_, v) in &pairs[..trailing] {
            text.push(if v < 0 { '1' } else { '0' });
        }
        let mut suffix = usize::from(total > 10 && trailing < 3) as u8;
        for (i, &(_, level)) in pairs.iter().enumerate().skip(trailing) {
            let mapped = if level > 0 {
                2 * i64::from(level) - 2
            } else {
                -2 * i64::from(level) - 1
            };
            let target = mapped - if i == trailing && trailing < 3 { 2 } else { 0 };
            let mut encoded = false;
            for prefix in 0u8..=31 {
                let n = if prefix < 14 {
                    suffix
                } else if prefix == 14 && suffix == 0 {
                    4
                } else if prefix < 15 {
                    suffix
                } else {
                    prefix - 3
                };
                let base = (i64::from(prefix.min(15)) << suffix)
                    + if prefix >= 15 && suffix == 0 { 15 } else { 0 }
                    + if prefix >= 16 {
                        (1i64 << (prefix - 3)) - 4096
                    } else {
                        0
                    };
                let remainder = target - base;
                if remainder >= 0 && remainder < (1i64 << n) {
                    text.push_str(&"0".repeat(prefix as usize));
                    text.push('1');
                    if n > 0 {
                        emit(&mut text, remainder as u64, n);
                    }
                    encoded = true;
                    break;
                }
            }
            assert!(encoded);
            suffix = suffix.max(1);
            if i64::from(level).abs() > (3i64 << (suffix - 1)) && suffix < 6 {
                suffix += 1;
            }
        }
        let mut zeros = pairs[0].0 + 1 - total;
        if total < values.len() {
            let table = match values.len() {
                4 => CHROMA_420_ZEROS,
                8 => CHROMA_422_ZEROS,
                _ => TOTAL_ZEROS,
            };
            emit_vlc(&mut text, table[total - 1], zeros as u8);
        }
        for i in 0..total - 1 {
            if zeros == 0 {
                break;
            }
            let run = pairs[i].0 - pairs[i + 1].0 - 1;
            emit_vlc(&mut text, RUN_BEFORE[zeros.min(7) - 1], run as u8);
            zeros -= run;
        }
        text
    }
    #[test]
    fn roundtrip_sparse_dense_and_large_levels_in_every_context() {
        let mut state = 0x12345678u32;
        for (nc, size, context) in [
            (0, 16, 0),
            (2, 15, 1),
            (4, 16, 2),
            (8, 16, 3),
            (-1, 4, 4),
            (-2, 8, 5),
        ] {
            for case in 0..400 {
                let mut coefficients = vec![0; size];
                for v in &mut coefficients {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    *v = match case % 4 {
                        0 => 0,
                        1 => (state % 3) as i32 - 1,
                        2 => (state % 31) as i32 - 15,
                        _ => (state % 200001) as i32 - 100000,
                    };
                }
                let encoded = encode(&coefficients, context);
                let bytes = bits(&encoded);
                let mut b = BitReader::new(&bytes);
                let result = read_residual(&mut b, nc, size as u8).unwrap();
                assert_eq!(&result.coefficients[..size], coefficients);
                assert_eq!(b.position(), encoded.len());
            }
        }
    }
}
