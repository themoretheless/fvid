//! Exact self-contained MP4 EP1 elementary-stream groups. Sample bytes are bare
//! AUs under SL predefined configuration 2; padb/fragment flags supply bit ends.
//! Presentation edits and player navigation are owned by the caller.
use crate::{
    owned_aac::{self, aac_ep1, config::AudioSpecificConfig},
    owned_codec_config::aac_stream_descriptor,
    owned_mp4::{Mp4Reader, Track},
};
use std::io::{Read, Seek};
type Result<T> = owned_aac::Result<T>;
fn invalid(message: &str) -> owned_aac::Error {
    owned_aac::Error(message.into())
}
include!("owned_mp4_ep1_impl.rs");
