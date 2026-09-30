//! FVid library facade: media formats, codecs, processing, playback, and adapters.
pub use fvid_control as media_control;
pub use fvid_media_info as media_info;
#[cfg(feature = "airbug")]
pub mod airbug_runtime;
#[cfg(feature = "player")]
pub mod audio;
#[cfg(feature = "player")]
pub mod audio_thread;
pub mod backend;
pub mod codec;
pub mod color;
pub mod container;
pub mod edit;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod playback;
#[cfg(feature = "player")]
#[path = "playback/audio/aac.rs"]
pub mod playback_aac;
#[cfg(feature = "player")]
#[path = "playback/audio/ac3.rs"]
pub mod playback_ac3;
#[cfg(feature = "player")]
#[path = "playback/audio/aiff.rs"]
pub mod playback_aiff;
#[cfg(feature = "player")]
#[path = "playback/audio/au.rs"]
pub mod playback_au;
#[cfg(feature = "player")]
#[path = "playback/audio/avi_audio.rs"]
pub mod playback_avi_audio;
#[cfg(feature = "player")]
#[path = "playback/audio/flac.rs"]
pub mod playback_flac;
#[cfg(feature = "player")]
#[path = "playback/audio/mp3.rs"]
pub mod playback_mp3;
#[path = "playback/video/mp4.rs"]
pub mod playback_mp4;
#[cfg(feature = "player")]
#[path = "playback/audio/mp4_audio.rs"]
pub mod playback_mp4_audio;
#[cfg(feature = "player")]
#[path = "playback/subtitles/mp4_subtitles.rs"]
pub mod playback_mp4_subtitles;
#[path = "playback/video/native.rs"]
pub mod playback_native;
#[cfg(feature = "player")]
#[path = "playback/audio/ogg_audio.rs"]
pub mod playback_ogg_audio;
#[cfg(feature = "player")]
#[path = "playback/audio/smf.rs"]
pub mod playback_smf;
#[path = "playback/runtime/spool.rs"]
pub mod playback_spool;
#[path = "playback/runtime/thread.rs"]
pub mod playback_thread;
#[cfg(feature = "player")]
#[path = "playback/audio/wav.rs"]
pub mod playback_wav;
#[path = "playback/video/webm.rs"]
pub mod playback_webm;
#[cfg(feature = "player")]
#[path = "playback/audio/webm_audio.rs"]
pub mod playback_webm_audio;
#[cfg(feature = "player")]
#[path = "playback/subtitles/webm_subtitles.rs"]
pub mod playback_webm_subtitles;
#[cfg(feature = "player")]
#[path = "playback/audio/xm.rs"]
pub mod playback_xm;
#[cfg(feature = "player")]
pub mod player;
#[cfg(feature = "player")]
pub mod player_gpu;
#[cfg(feature = "media")]
pub mod media;
pub mod native_media;
pub mod native_pcm;
pub mod native_geometry;
pub mod native_chromashift;
pub mod native_pixels;
pub mod native_lossless;
pub mod native_lossless_y4m;
pub mod native_morphology;
pub mod native_export;
pub mod publish;
pub mod resident;
#[cfg(feature = "player")]
pub mod snapshot;
pub mod subtitles;
mod error;
pub use error::{Error, Result};
pub(crate) use error::{invalid, unsupported};
mod view;
pub use view::{FrameView, PlaneView, RowView};
mod memory;
mod parallel;
pub(crate) use memory::buffer;
pub(crate) use parallel::span_workers;
#[cfg(feature = "gpu")]
mod gpu;
pub use backend::{Backend, ExecutionOptions};

/// Y4M parsing, checked transform planning, and streaming execution.
///
/// The types and functions are also re-exported from the crate root for backwards
/// compatibility; new code may prefer this explicit domain namespace.
pub mod y4m;
pub use y4m::{Crop, Header, PixelFormat, Plan, Stats, Transform, process, process_with_options};
#[cfg(any(feature = "gpu", feature = "cuda"))]
pub use y4m::process_gpu_chain;
pub(crate) use y4m::line;

pub mod virtual_camera;

mod pcm_resample;

#[path = "media_probe.rs"]
pub mod native_probe;

pub mod native_plan;

pub mod native_avgblur;

pub mod native_boxblur;
