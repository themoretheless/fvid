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
pub mod owned_loudnorm;
mod owned_dynamic_loudnorm;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_loudnorm::{apply_loudnorm, apply_loudnorm_dual, plan_loudnorm};

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
pub use owned_wave_remux::{concat, remux, trim, trim_pcm};

pub mod owned_wave_plan;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_plan::{plan_remux, plan_trim, plan_trim_pcm, plan_concat};

pub mod owned_true_peak;

pub mod owned_wave_loudness;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_loudness::{measure_loudness, plan_loudness};
