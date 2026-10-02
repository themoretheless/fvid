//! Owned integer shifts in chroma-sample coordinates, with smear or wrap edges.
use crate::{Result, invalid, native_geometry::GeometryFrame};
use crate::buffer;
include!("../crates/fvid-media/src/owned_chromashift_impl.rs");
