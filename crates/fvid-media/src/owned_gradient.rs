//! Owned planar spatial edge operators.
use crate::owned_frame::GeometryFrame;
use crate::owned_expression as gradient_expression;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_gradient_impl.rs");
