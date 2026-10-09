//! Shared owned ordinary LTP syntax; production admission remains separate.
use crate::{Result, invalid};
use super::{aac_synthesis::WindowSequence, bits::BitReader};
include!("../../crates/fvid-media/src/owned_aac/aac_ltp_syntax_impl.rs");
