//! Owned MP4 AVC/HEVC/AAC remux with presentation edits and DTS interleaving.
use crate::owned_codec_config::aac_specific_config;
use crate::owned_matroska::{
    self as matroska_write, Encoding, FileMetadata, PacketOptions, PacketWriter, TrackOptions,
    TrackSpec, VideoMetadata,
};
pub use crate::owned_matroska::{Error, Result};
use crate::owned_mp4::{Mp4Reader, Track};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    io::{Read, Seek, Write},
};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_mp4_matroska_impl.rs");
