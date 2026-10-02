//! Owned HEVC NAL header and bounded RBSP framing.
use crate::owned_aac::bits::unescape_rbsp;
pub use crate::owned_aac::{Error, Result};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_hevc_nal_impl.rs");
