//! FVid media layer. Owned operation contracts are available without libav.
//! The temporary legacy backend retains existing operations during migration.
#[cfg(feature = "legacy-ffmpeg")]
include!("legacy.rs");
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_media_info::*;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_control::{CancelFlag, CopyOptions, ProgressEvent, ProgressHook};

pub mod owned_wav;
pub mod owned_aac;

pub mod owned_shuffleplanes;

pub mod owned_k_weight;
pub mod owned_loudness;
pub mod owned_normalize;

pub mod owned_pcm_gain;

pub mod owned_resample;

pub mod owned_pcm_layout;

pub mod owned_wav_file;

pub mod owned_resample_f64;

pub mod owned_pcm_gain_f64;

pub mod owned_pcm_integer;

pub mod owned_pcm_format;

pub mod owned_time;

pub mod owned_wave_inspect;
pub mod owned_probe;

pub mod owned_pcm_channels;
