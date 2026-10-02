//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::aac_coupling_syntax::Coupling;
use super::aac_geometry::BandTables;
use super::{invalid, unsupported, Result};
fn default_pcm_mask(channels: u16) -> Result<u32> {
    crate::owned_wav::default_pcm_mask(channels).map_err(|e| invalid(&e))
}
use super::Error;
include!("aac_native_impl.rs");

#[cfg(test)]
mod channel_window_tests {
    use super::*;
    #[test]
    fn all_standard_channels_share_immutable_synthesis_windows() {
        for configuration in 1..=7 {
            let decoder = NativeAacDecoder::new(&[0x12, configuration << 3]).unwrap();
            assert_eq!(
                decoder.synthesis.len(),
                usize::from(decoder.config.channels)
            );
            for channel in &decoder.synthesis {
                assert!(decoder.synthesis[0].shares_windows_with(channel));
            }
        }
    }
}
