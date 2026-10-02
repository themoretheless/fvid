//! Owned AAC-LC raw-data-block decoder. The caller supplies ASC and strips any
//! ADTS/container framing. Remaining tools are explicit errors, never fallbacks.
use super::aac_coupling_syntax::Coupling;
use super::aac_geometry::BandTables;
use super::{Result, invalid, unsupported};
fn default_pcm_mask(channels: u16) -> Result<u32> {
    crate::owned_wav::default_pcm_mask(channels).map_err(|e| invalid(&e))
}
use super::Error;
include!("aac_native_impl.rs");
