//! Owned Opus transport metadata and packet duration; no audio decoding.
pub use crate::owned_matroska::{Error, Result};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_opus_packet_impl.rs");

#[cfg(test)]
mod header_transport_tests {
    use super::*;
    fn header(channels: u8, family: u8) -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.extend([1, channels]);
        h.extend(312u16.to_le_bytes());
        h.extend(48000u32.to_le_bytes());
        h.extend(0i16.to_le_bytes());
        h.push(family);
        if family == 1 {
            let layout = fvid_opus::ChannelLayout::surround(usize::from(channels), 1).unwrap();
            h.extend([layout.nb_streams as u8, layout.nb_coupled_streams as u8]);
            h.extend(layout.mapping);
        }
        h
    }
    #[test]
    fn transport_accepts_standard_mono_stereo_and_surround_without_relabelling() {
        for channels in 1..=8 {
            let h = header(channels, 1);
            assert_eq!(header_channels(&h).unwrap(), channels);
            assert_eq!(pre_skip_ns(&h).unwrap(), 6_500_000);
            assert!(
                header_channels(&h[..h.len() - 1]).is_err(),
                "truncated channel mapping"
            );
        }
        for channels in [1, 2] {
            assert_eq!(header_channels(&header(channels, 0)).unwrap(), channels);
        }
    }
    #[test]
    fn transport_uses_the_owned_identification_headers_version_rules() {
        let mut h = header(2, 0);
        for minor in [0, 1, 15] {
            h[8] = minor;
            assert_eq!(header_channels(&h).unwrap(), 2);
        }
        for major in [16, 255] {
            h[8] = major;
            assert!(header_channels(&h).is_err());
        }
        assert!(header_channels(b"OpusHead").is_err());
        h[8] = 1;
        h[9] = 0;
        assert!(header_channels(&h).is_err());
    }
}
