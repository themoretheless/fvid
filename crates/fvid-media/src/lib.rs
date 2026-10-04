//! FVid media layer. Owned operation contracts are available without libav.
//! The temporary legacy backend retains existing operations during migration.
mod input_policy;
pub mod owned_capabilities;
pub mod owned_expression;
pub use owned_capabilities::capabilities;
#[cfg(feature = "http-input")]
pub mod owned_http;
pub use input_policy::with_standalone_inputs;
#[cfg(feature = "legacy-ffmpeg")]
include!("legacy.rs");
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_media_info::*;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use fvid_control::{CancelFlag, CopyOptions, ProgressEvent, ProgressHook};

pub mod owned_wav;
pub mod owned_y4m;
pub mod owned_hls;
pub mod owned_colorize;
pub mod owned_colorhold;
pub mod owned_colorcontrast;
pub mod owned_vibrance;
pub mod owned_colorlevels;
mod owned_color_preserve;
pub mod owned_colorchannelmixer;
pub mod owned_exposure;
pub mod owned_colorbalance;
pub mod owned_curves;
pub mod owned_vignette;
pub mod owned_smartblur;
pub mod owned_sab;
pub mod owned_bitplanenoise;
pub mod owned_deband;
pub mod owned_perspective;
pub mod owned_gradfun;
pub mod owned_lenscorrection;
pub mod owned_draw;
pub mod owned_removegrain;
pub mod owned_yaepblur;
pub mod owned_rgba;
mod owned_color_names;
pub mod owned_colorcorrect;
pub mod owned_cas;
pub mod owned_grayworld;
pub mod owned_timeline;
pub mod owned_yuv_rgb;
pub mod owned_monochrome;
pub mod owned_lutyuv;
pub mod owned_y4m_probe;
pub mod owned_y4m_decode;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_video_decode::{decode_video, decode_video_transformed};
pub mod owned_aac;
pub mod owned_avc;

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
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_probe::{probe, probe_as};

pub mod owned_pcm_channels;

pub mod owned_audio_mix;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_audio_mix::{mix_audio, merge_audio, plan_mix_audio, plan_merge_audio};

pub mod owned_audio_export;
pub mod owned_audio_plan;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_audio_plan::plan_decode_audio;
mod owned_adts_export;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_audio_export::{decode_audio, decode_audio_interval, decode_audio_transformed};

pub mod owned_wave_metadata;

pub mod owned_budget;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_budget::{parse_max_memory_mib, parse_max_rss_mib};

pub mod owned_wave_remux;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_remux::{trim, trim_pcm};
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_concat::concat;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_remux::remux;

pub mod owned_wave_plan;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_plan::{plan_trim, plan_trim_pcm};
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_concat::plan_concat;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_remux_plan::plan_remux;

pub mod owned_true_peak;

pub mod owned_wave_loudness;
pub mod owned_adts_loudness;
pub mod owned_adts_loudnorm;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_wave_loudness::{measure_loudness, plan_loudness};

pub mod owned_negate;
pub mod owned_hue;
pub mod owned_eq;
pub mod owned_unsharp;

pub mod owned_frame;
pub mod owned_frame_layout;
pub mod owned_pixel_format;
pub mod owned_avgblur;
pub mod owned_gblur;
pub mod owned_bilateral;
pub mod owned_rotate;
pub mod owned_boxblur;

pub mod owned_pixelize;
pub mod owned_chromashift;

pub mod owned_gradient;

pub mod owned_morphology;

pub mod owned_overlay;
pub mod owned_xfade;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_xfade::{plan_xfade, xfade_video};
mod owned_y4m_overlay;

pub mod owned_ffv1_encoder;

pub mod owned_matroska;

pub mod owned_lossless;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_lossless::{
    crop_lossless, overlay_video, plan_overlay, plan_transcode_lossless, transcode, transcode_lossless,
};

pub mod owned_file_tags;

pub mod owned_ffv1_decoder;

pub mod owned_ebml;

pub mod owned_webm;

pub mod owned_webm_probe;

pub mod owned_codec_config;
pub mod owned_codec_metadata;

pub mod owned_hevc_nal;

pub mod owned_opus_packet;

pub mod owned_matroska_copy;

pub mod owned_matroska_remux;
pub mod owned_remux;

pub mod owned_remux_plan;

pub mod owned_adts_remux;

pub mod owned_concat;

pub mod owned_subtitle;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub fn convert_subtitles(source: &std::path::Path, destination: &std::path::Path,
    options: &fvid_media_info::SubtitleConvertOptions) -> std::result::Result<fvid_media_info::NativeSubtitleStats, String> {
    owned_subtitle::try_convert_with_options(source, destination, options)
        .map_err(|e| e.to_string())?.ok_or_else(|| "subtitle source is not supported by owned conversion".into())
}

pub mod owned_alac;

pub mod owned_matroska_alac;

mod owned_matroska_alac_export;

mod owned_matroska_audio;
pub mod owned_matroska_aac;
mod owned_matroska_aac_export;

pub mod owned_pcm_decoder;
pub mod owned_matroska_pcm;

mod owned_matroska_pcm_export;

pub mod owned_mp4;

pub mod owned_mp4_matroska;
pub mod owned_mp4_remux;

pub mod owned_mp4_audio_schedule;
pub mod owned_mp4_audio;

mod owned_mp4_audio_export;

pub mod owned_container_loudness;

pub mod owned_container_loudnorm;

pub mod owned_mp4_probe;

mod owned_backend_error;

pub mod owned_framestep;

mod owned_shuffleframes;

mod owned_reverse;

mod owned_video_decode;

mod owned_ffv1_export;

pub mod owned_ima4;

pub mod owned_ima_wav;

pub mod owned_ms_adpcm;

pub mod owned_play_controls;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_play_controls::*;

pub mod owned_text_raster;

pub mod owned_subtitle_burn;
#[cfg(not(feature = "legacy-ffmpeg"))]
pub use owned_subtitle_burn::{burn_subtitles, plan_burn_subtitles};

pub mod owned_fade;

pub mod owned_lagfun;

pub mod owned_tmix;
pub mod owned_hqdn3d;
mod owned_pixel_context;

mod owned_compressed_video;
mod owned_mp4_video_decode;
pub mod owned_video_timeline;
mod owned_webm_codec;

/// Shared owned AVC/HEVC/VP9/AV1 packet decoders; container workflow admission is separate.
pub use fvid_codecs::codec as owned_codecs;
