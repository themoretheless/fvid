//! FVid library facade: media formats, codecs, processing, playback, and adapters.
pub use fvid_control as media_control;
pub use fvid_media_info as media_info;
#[cfg(feature = "airbug")]
pub mod airbug_runtime;
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
pub use playback::audio::{
    aac as playback_aac, ac3 as playback_ac3, aiff as playback_aiff, au as playback_au,
    avi_audio as playback_avi_audio, flac as playback_flac, mp3 as playback_mp3,
    mp4_audio as playback_mp4_audio, ogg_audio as playback_ogg_audio,
    ogg_catalog as playback_ogg_catalog, ogg_opus as playback_ogg_opus, smf as playback_smf,
    wav as playback_wav, wavpack as playback_wavpack, webm_audio as playback_webm_audio,
    xm as playback_xm,
};
pub use playback::runtime::{spool as playback_spool, thread as playback_thread};
#[cfg(feature = "player")]
pub use playback::subtitles::{
    mp4_subtitles as playback_mp4_subtitles, webm_subtitles as playback_webm_subtitles,
};
pub use playback::video::{
    mp4 as playback_mp4, native as playback_native, webm as playback_webm,
};
#[cfg(feature = "player")]
pub mod player;
#[cfg(feature = "player")]
pub use player::gpu as player_gpu;
pub mod media;
pub mod native_media;
pub mod native_capabilities;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub mod hardware_export;
pub mod native_pcm;
pub mod native_geometry;
pub mod native_chromashift;
pub mod native_pixels;
pub mod native_lossless;
pub mod native_lossless_y4m;
pub mod native_morphology;
pub mod native_export;
pub mod native_subtitle;
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
pub use y4m::{process_gpu_chain, process_gpu_stages};
pub(crate) use y4m::line;

pub mod virtual_camera;

mod pcm_resample;

pub mod native_probe;

pub mod native_plan;
pub mod native_loudnorm;

pub mod native_avgblur;
pub mod native_pixelize;

pub mod native_boxblur;

pub mod native_audio_mix;

pub mod native_shuffleplanes;
