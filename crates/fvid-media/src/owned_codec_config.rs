//! Owned AVC/HEVC configuration records, AAC descriptors and NAL framing.
pub use crate::owned_aac::config::AacConfig;
pub use crate::owned_aac::{Error, Result};
use crate::owned_hevc_nal as hevc_nal;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_codec_config_impl.rs");
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_configuration_records_and_nal_lengths_are_checked() {
        assert!(AvcConfig::parse(&[]).is_err());
        assert!(HevcConfig::parse(&[]).is_err());
        assert_eq!(AacConfig::parse(&[0x12, 0x10]).unwrap().sample_rate, 44100);
        assert!(aac_specific_config(&[]).is_err());
        let mut units = NalUnits::new(&[0, 2, 0x65, 0x80], 2).unwrap();
        assert_eq!(units.next().unwrap().unwrap(), &[0x65, 0x80]);
        assert!(units.next().is_none());
        assert!(
            NalUnits::new(&[0, 3, 0x65], 2)
                .unwrap()
                .next()
                .unwrap()
                .is_err()
        );
    }
}
