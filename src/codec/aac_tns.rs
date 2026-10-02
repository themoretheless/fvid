//! AAC temporal noise shaping side-information parser.
use crate::{Result, invalid};
pub use fvid_media::owned_aac::aac_tns::{TnsData, TnsFilter};
include!("../../crates/fvid-media/src/owned_aac/aac_tns_impl.rs");

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
