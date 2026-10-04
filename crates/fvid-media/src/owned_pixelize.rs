//! Owned block reduction and fill for planar integer YUV.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
use crate::owned_expression as pixelize_expression;
include!("owned_pixelize_impl.rs");
