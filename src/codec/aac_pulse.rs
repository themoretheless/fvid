//! AAC pulse reconstruction on long-window quantized coefficients.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_aac/aac_pulse_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut n = 0;
        for &(value, width) in fields {
            for bit in (0..width).rev() {
                if n % 8 == 0 {
                    out.push(0);
                }
                *out.last_mut().unwrap() |= ((value >> bit & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        out
    }
    #[test]
    fn signed_zero_and_repeated_pulses_apply_before_quantization() {
        let data = pack(&[
            (3, 2),
            (1, 6),
            (0, 5),
            (3, 4),
            (1, 5),
            (2, 4),
            (1, 5),
            (4, 4),
            (0, 5),
            (5, 4),
        ]);
        let mut bits = BitReader::new(&data);
        let pulse = PulseData::read(&mut bits, WindowSequence::OnlyLong, &[0, 4, 1024]).unwrap();
        assert_eq!(bits.position(), 44);
        let mut coefficients = vec![0; 1024];
        coefficients[4] = 2;
        coefficients[5] = -1;
        pulse.apply(&mut coefficients).unwrap();
        assert_eq!(&coefficients[4..7], &[5, -3, -9]);
        let mut spectrum = [0.0; 3];
        super::super::aac_quant::inverse_quantize(&coefficients[4..7], 100, &mut spectrum).unwrap();
        assert!(spectrum[0] > 0.0 && spectrum[1] < 0.0 && spectrum[2] < spectrum[1]);
    }
    #[test]
    fn maximum_pulses_remain_reconstructible() {
        let data = pack(&[
            (3, 2),
            (0, 6),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
            (0, 5),
            (15, 4),
        ]);
        let pulse = PulseData::read(
            &mut BitReader::new(&data),
            WindowSequence::LongStart,
            &[0, 960],
        )
        .unwrap();
        for initial in [-8191, 8191] {
            let mut values = vec![0; 960];
            values[0] = initial;
            pulse.apply(&mut values).unwrap();
            assert_eq!(values[0].abs(), 8251);
            super::super::aac_quant::inverse_quantize(&values, 255, &mut vec![0.0; 960]).unwrap();
        }
    }
    #[test]
    fn invalid_offsets_and_truncation_preserve_cursor() {
        for data in [
            pack(&[(0, 2), (1, 6), (1, 5), (1, 4)]),
            pack(&[(0, 2), (2, 6), (0, 5), (1, 4)]),
            vec![0],
        ] {
            let mut bits = BitReader::new(&data);
            assert!(
                PulseData::read(&mut bits, WindowSequence::OnlyLong, &[0, 1023, 1024]).is_err()
            );
            assert_eq!(bits.position(), 0);
        }
        assert!(
            PulseData::read(
                &mut BitReader::new(&[0; 8]),
                WindowSequence::EightShort,
                &[0, 1024]
            )
            .is_err()
        );
    }
}
