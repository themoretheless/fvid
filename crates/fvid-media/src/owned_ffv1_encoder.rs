//! Owned FFV1 version 1 range-coded, all-intra YCbCr encoder.
//! Bitstream semantics: RFC 9043. One slice and a single residual context
//! per luma/chroma model; quantization here selects contexts, not sample loss.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_ffv1_encoder_impl.rs");
