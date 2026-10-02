//! H.265 7.3.1 NAL headers and bounded RBSP extraction.
use super::bits::unescape_rbsp;
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_hevc_nal_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_header_bit_patterns_preserve_fields_or_reject_invalid_bits() {
        for bits in 0u16..=u16::MAX {
            let bytes = bits.to_be_bytes();
            let parsed = NalHeader::parse(&bytes);
            if bits & 0x8000 != 0 || bits & 7 == 0 {
                assert!(parsed.is_err());
            } else {
                let header = parsed.unwrap();
                assert_eq!(header.unit_type, ((bits >> 9) & 63) as u8);
                assert_eq!(header.layer_id, ((bits >> 3) & 63) as u8);
                assert_eq!(header.temporal_id, (bits & 7) as u8 - 1);
                assert_eq!(header.require_base_layer().is_ok(), header.layer_id == 0);
            }
        }
        assert!(NalHeader::parse(&[]).is_err());
        assert!(NalHeader::parse(&[0x40]).is_err());
    }
    #[test]
    fn payload_escape_budget_and_random_access_classification() {
        let nal = [0x26, 1, 0, 0, 3, 1, 0x80];
        let parsed = NalRbsp::parse(&nal, 5).unwrap();
        assert!(parsed.header.is_idr() && parsed.header.is_irap() && parsed.header.is_vcl());
        assert_eq!(parsed.bytes, [0, 0, 1, 0x80]);
        assert!(NalRbsp::parse(&nal, 4).is_err());
        assert!(NalRbsp::parse(&[0x40, 1], 10).is_err());
        for payload in [&[0, 0, 1][..], &[0, 0, 3], &[0, 0, 3, 4]] {
            let mut bytes = vec![0x40, 1];
            bytes.extend_from_slice(payload);
            assert!(NalRbsp::parse(&bytes, 100).is_err());
        }
        let sps = NalHeader::parse(&[0x42, 1]).unwrap();
        assert!(!sps.is_vcl() && !sps.is_irap() && !sps.is_idr());
    }
}
