//! Shared codec records; no audio decoder or media backend dependency.
use crate::codec::hevc_nal;
use crate::{Result, invalid};
include!("../../fvid-media/src/owned_codec_config_impl.rs");
