//! Owned FFV1 v0/v1 decoding, persistent context models and bounded storage.
//! RFC 9043 bitstream core is shared with native frontend playback.
use crate::owned_ffv1_encoder::TRANSITION;
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
fn unsupported(message: &str) -> String {
    message.into()
}
include!("owned_ffv1_decoder_impl.rs");
