//! AAC-LC individual-channel window and section syntax.
//! Spectral Huffman decoding and sample-rate-specific band tables are separate.
use super::{Result, invalid, unsupported};

include!("aac_ics_impl.rs");
