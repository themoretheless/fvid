//! Owned ESC to logical ELD reconstruction.
use crate::{Result, invalid};
use super::{bits::BitReader, config::AacConfig, aac_ep::RecoveredFrame};
include!("../../crates/fvid-media/src/owned_aac/aac_esc_impl.rs");
