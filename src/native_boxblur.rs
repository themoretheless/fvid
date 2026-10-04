//! Owned separable repeated box blur on planar YUV samples.
use fvid_media::owned_expression::Expression as BoxBlurExpression;
use crate::{Result, invalid, buffer, native_geometry::GeometryFrame};
include!("../crates/fvid-media/src/owned_boxblur_impl.rs");
