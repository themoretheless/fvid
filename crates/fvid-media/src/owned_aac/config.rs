//! Owned AAC AudioSpecificConfig and explicit program parsing.
use super::bits::BitReader;
use super::{Result, invalid};
include!("config_impl.rs");

#[cfg(test)]
mod ld_tests {
    use super::*;
    #[test]
    fn ld_frame_flag_ep_config_and_resilience_fields_have_distinct_meanings() {
        for (asc,n) in [([0xbb,0x08,0],512),([0xbb,0x0c,0],480)] {
            let c=AacConfig::parse(&asc).unwrap();
            assert_eq!((c.object_type,c.sample_rate,c.channels,c.frame_samples),(23,24000,1,n));
        }
        let c=AacConfig::parse(&[0xbb,0x09,0xe0]).unwrap();
        assert!(c.section_data_resilience && c.scalefactor_data_resilience && c.spectral_data_resilience);
        assert!(AacConfig::parse(&[0xbb,0x08,0x40]).unwrap_err().to_string().contains("LD epConfig"));
        assert!(AacConfig::parse(&[0xbb,0x00,0]).unwrap_err().to_string().contains("LD PCE"));
        assert!(AacConfig::parse(&[0xbc,0x08,0]).unwrap_err().to_string().contains("LD band geometry"));
    }
}
