//! Owned Opus transport metadata and packet duration; no audio decoding.
pub use crate::owned_matroska::{Error, Result};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_opus_packet_impl.rs");
