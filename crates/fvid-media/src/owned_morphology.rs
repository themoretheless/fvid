//! Owned threshold-limited spatial dilation and erosion.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
use crate::owned_expression as morphology_expression;
include!("owned_morphology_impl.rs");
