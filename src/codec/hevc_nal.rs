//! H.265 7.3.1 NAL headers and bounded RBSP extraction.
use super::bits::unescape_rbsp;
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NalHeader {
    pub unit_type: u8,
    pub layer_id: u8,
    pub temporal_id: u8,
}
impl NalHeader {
    /// Parse header syntax without interpreting reserved NAL types as known data.
    /// Multilayer headers remain representable; a base-layer decoder must reject
    /// unsupported layer IDs explicitly before processing their payloads.
    pub fn parse(nal: &[u8]) -> Result<Self> {
        let header = nal
            .get(..2)
            .ok_or_else(|| invalid("truncated HEVC NAL header"))?;
        if header[0] & 0x80 != 0 || header[1] & 7 == 0 {
            return Err(invalid("invalid HEVC forbidden/temporal header bits"));
        }
        Ok(Self {
            unit_type: (header[0] >> 1) & 63,
            layer_id: ((header[0] & 1) << 5) | (header[1] >> 3),
            temporal_id: (header[1] & 7) - 1,
        })
    }
    pub fn is_vcl(self) -> bool {
        self.unit_type < 32
    }
    pub fn is_irap(self) -> bool {
        (16..=23).contains(&self.unit_type)
    }
    pub fn is_idr(self) -> bool {
        matches!(self.unit_type, 19 | 20)
    }
    pub fn require_base_layer(self) -> Result<()> {
        if self.layer_id != 0 {
            return Err(invalid("HEVC multilayer decoding is not implemented"));
        }
        Ok(())
    }
}

pub struct NalRbsp {
    pub header: NalHeader,
    pub bytes: Vec<u8>,
}
impl NalRbsp {
    /// Budget bounds the allocation before de-escaping untrusted input.
    /// RBSP trailing bits depend on payload syntax and are checked by its reader.
    pub fn parse(nal: &[u8], budget: usize) -> Result<Self> {
        let header = NalHeader::parse(nal)?;
        let payload = &nal[2..];
        if payload.is_empty() || payload.len() > budget {
            return Err(invalid("empty HEVC payload or RBSP budget exceeded"));
        }
        Ok(Self {
            header,
            bytes: unescape_rbsp(payload)?,
        })
    }
}

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
