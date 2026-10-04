//! Owned planar spatial edge operators.
use crate::owned_frame::GeometryFrame;
use crate::owned_expression::constant as gradient_constant;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_gradient_impl.rs");
