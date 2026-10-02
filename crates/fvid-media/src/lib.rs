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

pub mod owned_audio_mix;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_audio_mix::{mix_audio, merge_audio};

pub mod owned_audio_export;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_audio_export::{decode_audio, decode_audio_interval, decode_audio_transformed};

pub mod owned_wave_metadata;

pub mod owned_budget;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_budget::{parse_max_memory_mib, parse_max_rss_mib};

pub mod owned_wave_remux;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_remux::remux;
