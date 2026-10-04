//! Owned integer shifts in chroma-sample coordinates, with smear or wrap edges.
use crate::owned_frame::GeometryFrame;
use crate::owned_frame::buffer;
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
use crate::owned_expression as chromashift_expression;
include!("owned_chromashift_impl.rs");
