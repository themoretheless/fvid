//! Grouped AAC spectral payload decoding. Special bands retain zero placeholders
//! for later noise/intensity reconstruction; they consume no spectral codewords.
use super::{Result, invalid};

include!("aac_spectral_impl.rs");
