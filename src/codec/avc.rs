//! H.264 sequence parameter syntax, implemented independently of codec libraries.
use super::bits::{BitReader, unescape_rbsp};
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_avc_impl.rs");
