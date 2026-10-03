//! Shared playback controls, DSP and text subtitles without libav.
use serde::Serialize;
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, String>;
include!("owned_play_controls_impl.rs");
