//! Opaque overlay of packed RGB or matching planar YUV, without conversion.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_overlay_impl.rs");
