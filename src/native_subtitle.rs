//! Owned subtitle conversion shared with the media library.
use crate::container::{matroska_write::{Encoding, PacketWriter, TrackSpec}, webm};
use crate::{Result, invalid, media_info};
use crate::native_export::is_matroska_source;
include!("../crates/fvid-media/src/owned_subtitle_impl.rs");
