//! AAC temporal noise shaping side-information parser.
use super::{aac_synthesis::WindowSequence, bits::BitReader};
use crate::{Result, invalid};
pub use fvid_media::owned_aac::aac_tns::{TnsData, TnsFilter};
use std::f64::consts::FRAC_PI_2;

/// Called after tns_data_present. Input cursor commits only on success.
pub fn read(bits: &mut BitReader<'_>, sequence: WindowSequence) -> Result<TnsData> {
    let short = sequence == WindowSequence::EightShort;
    let mut cursor = bits.clone();
    let mut windows = Vec::with_capacity(if short { 8 } else { 1 });
    for _ in 0..if short { 8 } else { 1 } {
        let count = cursor.read(if short { 1 } else { 2 })?;
        let resolution = if count > 0 {
            3 + cursor.read(1)? as u8
        } else {
            3
        };
        let mut filters = Vec::new();
        for _ in 0..count {
            let length = cursor.read(if short { 4 } else { 6 })? as usize;
            let order = cursor.read(if short { 3 } else { 5 })? as usize;
            if order > if short { 7 } else { 12 } {
                return Err(invalid("AAC-LC TNS order exceeds limit"));
            }
            let mut reverse = false;
            let mut lpc = Vec::with_capacity(order);
            if order > 0 {
                reverse = cursor.bit()?;
                let width = resolution - u8::from(cursor.bit()?);
                for _ in 0..order {
                    let raw = cursor.read(width)? as i32;
                    let signed = if raw & (1 << (width - 1)) != 0 {
                        raw - (1 << width)
                    } else {
                        raw
                    };
                    let base = (1u32 << (resolution - 1)) as f64;
                    let denominator = base + if signed < 0 { 0.5 } else { -0.5 };
                    let reflection = (f64::from(signed) * FRAC_PI_2 / denominator).sin();
                    let previous = lpc.clone();
                    for i in 0..previous.len() {
                        lpc[i] = previous[i] + reflection * previous[previous.len() - 1 - i];
                    }
                    lpc.push(reflection);
                }
            }
            filters.push(TnsFilter {
                length,
                reverse,
                lpc,
            });
        }
        windows.push(filters);
    }
    *bits = cursor;
    Ok(TnsData { windows })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, w) in fields {
            for b in (0..w).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn signed_compressed_coefficients_use_original_resolution() {
        for resolution in [3u8, 4] {
            for compressed in [false, true] {
                let width = resolution - u8::from(compressed);
                let (data, n) = pack(&[
                    (1, 2),
                    ((resolution - 3) as u32, 1),
                    (20, 6),
                    (2, 5),
                    (1, 1),
                    (compressed as u32, 1),
                    (1, width),
                    ((1u32 << width) - 1, width),
                ]);
                let mut bits = BitReader::new(&data);
                let tns = read(&mut bits, WindowSequence::OnlyLong).unwrap();
                let f = &tns.windows[0][0];
                let base = (1u32 << (resolution - 1)) as f64;
                let a = (FRAC_PI_2 / (base - 0.5)).sin();
                let b = (-FRAC_PI_2 / (base + 0.5)).sin();
                assert_eq!((f.length, f.reverse), (20, true));
                assert!((f.lpc[0] - a * (1.0 + b)).abs() < 1e-15);
                assert!((f.lpc[1] - b).abs() < 1e-15);
                assert_eq!(bits.position(), n);
            }
        }
    }
    #[test]
    fn short_empty_windows_and_zero_order_do_not_read_coefficients() {
        let (data, n) = pack(&[(1, 1), (0, 1), (4, 4), (0, 3), (0, 7)]);
        let mut bits = BitReader::new(&data);
        let tns = read(&mut bits, WindowSequence::EightShort).unwrap();
        assert_eq!(tns.windows.len(), 8);
        assert!(tns.windows[0][0].lpc.is_empty());
        assert!(tns.windows[1..].iter().all(Vec::is_empty));
        assert_eq!(bits.position(), n);
    }
    #[test]
    fn excessive_order_and_truncation_preserve_cursor() {
        for data in [
            pack(&[(1, 2), (0, 1), (20, 6), (13, 5)]).0,
            vec![0x40],
            vec![],
        ] {
            let mut bits = BitReader::new(&data);
            assert!(read(&mut bits, WindowSequence::OnlyLong).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
