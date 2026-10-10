//! Compatibility LD ICS parsing; PCM processing remains owned by fvid-media.
use super::bits::BitReader;
use crate::{Result, invalid};
include!("../../crates/fvid-media/src/owned_aac/aac_ld_syntax_impl.rs");
