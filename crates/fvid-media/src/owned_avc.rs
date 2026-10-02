//! Owned AVC SPS/PPS syntax, geometry, VUI and slice-group descriptions.
//! Parsing parameter sets is not acceptance of every profile/tool for decoding.
use crate::owned_aac::bits::{BitReader, unescape_rbsp};
pub use crate::owned_aac::{Error, Result};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_avc_impl.rs");

fn profile_label(profile: u8, constraints: u8) -> Option<&'static str> {
    // Preserve the profiles admitted by this SPS-only probe. Metadata naming
    // does not expand the parser or decoder's supported profiles.
    if !matches!(
        profile,
        44 | 66 | 77 | 88 | 100 | 110 | 118 | 122 | 128 | 244
    ) {
        return None;
    }
    let mut identifier = i32::from(profile);
    if profile == 66 && constraints & 0x40 != 0 {
        identifier |= 512;
    }
    if matches!(profile, 110 | 122 | 244) && constraints & 0x10 != 0 {
        identifier |= 2048;
    }
    crate::owned_codec_metadata::descriptor("h264")?.profile_name(identifier)
}

/// Describe consistent SPS profile/level declarations, without decoding frames.
/// Malformed or heterogeneous SPS sets remain unknown to container-only callers.
pub fn configuration_profile_level(configuration: &[u8]) -> Option<(&'static str, u8)> {
    let config = crate::owned_codec_config::AvcConfig::parse(configuration).ok()?;
    let mut description = None;
    for nal in config.sps {
        let sps = Sps::parse(nal).ok()?;
        let current = (profile_label(sps.profile, sps.constraints)?, sps.level);
        if description.is_some_and(|previous| previous != current) {
            return None;
        }
        description = Some(current);
    }
    description
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn constraint_flags_have_profile_specific_names() {
        assert_eq!(profile_label(66, 0), Some("Baseline"));
        assert_eq!(profile_label(66, 0x40), Some("Constrained Baseline"));
        assert_eq!(profile_label(77, 0x40), Some("Main"));
        assert_eq!(profile_label(100, 0x10), Some("High"));
        assert_eq!(profile_label(110, 0), Some("High 10"));
        assert_eq!(profile_label(110, 0x10), Some("High 10 Intra"));
        assert_eq!(profile_label(244, 0x10), Some("High 4:4:4 Intra"));
        assert_eq!(profile_label(255, 0), None);
        assert_eq!(configuration_profile_level(&[]), None);
    }
}
