//! Container-preserving Matroska copy for an identity remux request.
//! Validate the complete owned index, then retain every EBML byte, including
//! metadata and attachments that the playback-facing index does not expose.
use super::webm::{Limits, WebmReader};
use crate::{Result, invalid};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::io::{Read, Seek, SeekFrom, Write};

fn buffer(len: usize) -> Result<Vec<u8>> { crate::buffer(len) }
include!("../../crates/fvid-media/src/owned_matroska_copy_impl.rs");
