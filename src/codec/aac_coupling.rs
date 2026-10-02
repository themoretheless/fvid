//! Owned AAC coupling syntax and dependent spectral mixing.
use crate::{Result, invalid};
use super::aac_bands::BandTables;
use fvid_media::owned_aac::aac_coupling as coupling_mix;
use crate::Error;
include!("../../crates/fvid-media/src/owned_aac/aac_coupling_impl.rs");
