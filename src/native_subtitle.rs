//! Owned subtitle conversion shared with the media library.
use crate::container::{matroska_write::{Encoding, FileMetadata, PacketWriter, TrackSpec}, webm};
use crate::{Error, Result, invalid, media_control, media_info};
use crate::native_export::is_matroska_source;
use fvid_media::owned_budget;
include!("../crates/fvid-media/src/owned_subtitle_impl.rs");
