//! Owned bounded AAC gain-control syntax. Active SSR gain synthesis is separate.
use super::{Result, aac_synthesis::WindowSequence, bits::BitReader};

include!("aac_gain_control_impl.rs");
