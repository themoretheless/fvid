//! Byte-preserving validated Matroska identity copy, including unindexed metadata.
pub use crate::owned_ebml::{Error, Result};
use crate::owned_webm::{Limits, WebmReader};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};
fn invalid(message: &str) -> Error {
    Error(message.into())
}
fn buffer(len: usize) -> Result<Vec<u8>> {
    crate::owned_frame::buffer(len).map_err(Error)
}
include!("owned_matroska_copy_impl.rs");
