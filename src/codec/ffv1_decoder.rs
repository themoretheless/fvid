//! Owned FFV1 v0/v1/v3 range and Rice decoding with persistent slice contexts.
use super::ffv1_encoder::TRANSITION;
use crate::{Result, buffer, invalid, native_geometry::GeometryFrame, unsupported};
include!("../../crates/fvid-media/src/owned_ffv1_decoder_impl.rs");
