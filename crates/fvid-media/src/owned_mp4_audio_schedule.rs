//! Sample-clock scheduling for unit-rate MP4 audio edits.
pub use crate::owned_mp4::{Error, Result};
use crate::owned_mp4::{Edit, invalid};
include!("owned_mp4_audio_schedule_impl.rs");
