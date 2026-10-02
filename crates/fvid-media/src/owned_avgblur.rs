//! Owned planar integer box average with replicated edge samples.
use crate::owned_frame::{GeometryFrame, buffer};
type Result<T> = std::result::Result<T, String>;
fn invalid(message: &str) -> String {
    message.into()
}
include!("owned_avgblur_impl.rs");
