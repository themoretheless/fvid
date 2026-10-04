//! Owned block reduction and fill for planar integer YUV.
use crate::{Result, invalid, native_geometry::GeometryFrame};
use fvid_media::owned_expression as pixelize_expression;
include!("../crates/fvid-media/src/owned_pixelize_impl.rs");
