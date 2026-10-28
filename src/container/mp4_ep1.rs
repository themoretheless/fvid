//! Native compatibility binding for the owned MP4 EP1 group implementation.
use crate::codec::aac_ep1;
use crate::codec::config::{AudioSpecificConfig, aac_stream_descriptor};
use crate::container::mp4::{Mp4Reader, Track};
use crate::{Result, invalid};
use std::io::{Read, Seek};
include!("../../crates/fvid-media/src/owned_mp4_ep1_impl.rs");
