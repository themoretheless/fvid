//! Opus transport metadata only; this module does not decode audio samples.
//! Framing: RFC 6716 section 3; identification: RFC 7845 section 5.1.
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_opus_packet_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_toc_durations_and_frame_count_limits() {
        for config in 0..32u8 {
            let expected = if config < 12 {
                [10, 20, 40, 60][usize::from(config & 3)] * 1_000_000
            } else if config < 16 {
                [10, 20][usize::from(config & 1)] * 1_000_000
            } else {
                [2500, 5000, 10000, 20000][usize::from(config & 3)] * 1000
            };
            assert_eq!(duration_ns(&[config << 3]).unwrap(), expected);
            assert_eq!(duration_ns(&[(config << 3) | 1]).unwrap(), expected * 2);
        }
        assert_eq!(duration_ns(&[(16 << 3) | 3, 48]).unwrap(), 120_000_000);
        for bad in [
            &[][..],
            &[3],
            &[3, 0],
            &[3, 13],
            &[1, 0],
            &[2, 5],
            &[3, 0x41, 255],
            &[3, 0x41, 10],
            &[3, 0x82, 252],
        ] {
            assert!(duration_ns(bad).is_err(), "{bad:?}");
        }
    }
    #[test]
    fn vbr_padding_and_short_packets_are_checked() {
        assert_eq!(duration_ns(&[3, 0x42, 1, 9, 10, 0]).unwrap(), 20_000_000);
        assert_eq!(duration_ns(&[3, 0x82, 1, 9, 10, 11]).unwrap(), 20_000_000);
        let mut extended = vec![2, 252, 0];
        extended.extend(vec![0; 252]);
        assert_eq!(duration_ns(&extended).unwrap(), 20_000_000);
        for first in 0..=255u8 {
            for second in 0..=255u8 {
                let _ = duration_ns(&[first, second]);
            }
        }
    }
    #[test]
    fn identification_and_preskip() {
        let mut header = b"OpusHead\x01\x02\x38\x01\x80\xbb\x00\x00\x00\x00\x00".to_vec();
        assert_eq!(header_channels(&header).unwrap(), 2);
        assert_eq!(pre_skip_ns(&header).unwrap(), 6_500_000);
        header[18] = 1;
        assert!(header_channels(&header).is_err());
    }
}
