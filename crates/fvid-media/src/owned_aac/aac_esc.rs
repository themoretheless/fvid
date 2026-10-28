//! Exact ESC instances to logical ELD payload; ISO 4.5.2.4 and table 4.148.
use super::{Result, aac_ep::RecoveredFrame, bits::BitReader, config::AacConfig, invalid};
include!("aac_esc_impl.rs");
