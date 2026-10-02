//! Owned separable repeated box blur on planar YUV samples.
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_boxblur_impl.rs");
