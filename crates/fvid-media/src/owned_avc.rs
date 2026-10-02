//! Owned AVC SPS/PPS syntax, geometry, VUI and slice-group descriptions.
//! Parsing parameter sets is not acceptance of every profile/tool for decoding.
use crate::owned_aac::bits::{BitReader, unescape_rbsp};
pub use crate::owned_aac::{Error, Result};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_avc_impl.rs");

fn profile_label(profile: u8, constraints: u8) -> Option<&'static str> {
    // Canonical interoperability names; flags are in SPS byte order.
    // Reference vocabulary: https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/profiles.c
    match (profile, constraints & 0x10 != 0) {
        (66, _) if constraints & 0x40 != 0 => Some("Constrained Baseline"),
        (66, _) => Some("Baseline"),
        (77, _) => Some("Main"),
        (88, _) => Some("Extended"),
        (100, _) => Some("High"),
        (110, false) => Some("High 10"),
        (110, true) => Some("High 10 Intra"),
        (122, false) => Some("High 4:2:2"),
        (122, true) => Some("High 4:2:2 Intra"),
        (244, false) => Some("High 4:4:4 Predictive"),
        (244, true) => Some("High 4:4:4 Intra"),
        (44, _) => Some("CAVLC 4:4:4"),
        (118, _) => Some("Multiview High"),
        (128, _) => Some("Stereo High"),
        _ => None,
    }
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
