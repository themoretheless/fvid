//! FVid-owned compressed bitstream building blocks.
//! Configuration parsing alone does not imply that a codec can decode frames.
pub mod bits;

pub mod config;

pub mod avc;

pub mod avc_transform;

pub mod avc_prediction;

pub mod avc_slice;

pub mod cavlc;
mod cavlc_tables;

pub mod avc_intra;

pub mod avc_macroblock;

mod avc_8x8_tables;
pub mod avc_cabac;
mod avc_cabac_init;
pub mod avc_cabac_macroblock;
pub mod avc_deblock;
pub mod avc_motion;
pub mod avc_mv;
pub mod avc_picture;
pub mod avc_poc;
pub mod avc_references;
pub mod avc_dpb;
pub mod avc_transform8;
pub mod cabac;
mod cabac_tables;

pub mod avc_inter;

pub mod avc_compensation;

pub mod avc_boundary;

pub mod avc_motion_field;

pub mod avc_inter_prediction;

pub mod avc_residual_syntax;

pub mod avc_inter_coefficients;

pub mod avc_coefficient_field;

pub mod avc_inter_slice;

pub mod avc_inter_picture;

pub mod avc_decoder;

pub mod avc_cabac_inter;

pub mod avc_cabac_motion;

pub mod avc_cabac_slice;

pub mod avc_reference_motion;

pub mod avc_direct;

pub mod hevc_nal;

pub mod hevc_profile;

pub mod hevc_vps;

pub mod hevc_hrd;

pub mod hevc_vui;

pub mod hevc_sps;

pub mod hevc_rps;

pub mod hevc_scaling;

pub mod hevc_pps;

pub mod hevc_slice;

pub mod hevc_cabac;

mod hevc_cabac_tables;

pub mod hevc_residual;

pub mod hevc_transform;
mod hevc_transform_tables;
pub mod hevc_intra;
pub mod hevc_intra_syntax;
pub mod hevc_sao;
pub mod hevc_tree;
pub mod hevc_transform_tree;
pub mod hevc_block;
pub mod hevc_qp;
pub mod hevc_plane;
pub mod hevc_picture;
pub mod hevc_decoder;
pub mod hevc_inter_syntax;
pub mod hevc_motion;
pub mod hevc_deblock;
pub mod vp9;
pub mod vp9_bool;
pub mod vp9_probs;
pub mod vp9_intra;
pub mod vp9_transform;
pub mod vp9_residual;
pub mod vp9_picture;
pub mod vp9_filter;
pub mod vp9_motion;
pub mod vp9_decoder;
mod vp9_tables;

pub mod av1;

pub mod av1_sequence;

pub mod av1_symbol;

pub mod av1_tiles;

pub mod av1_frame;

mod av1_cdfs;

pub mod av1_picture;

pub mod av1_decoder;

pub mod av1_transform;
mod av1_warp;

mod av1_tables;

pub mod av1_intra;

mod av1_filter;

#[cfg(feature = "player")]
pub mod aac_decoder;
#[cfg(feature = "player")]
pub mod vorbis_decoder;

/// Build the decoder that matches a container's codec tag.
#[cfg(feature = "player")]
pub fn make_audio_decoder(
    codec: &str,
    extra_data: &[u8],
    sample_rate: u32,
    channels: u16,
) -> crate::Result<Box<dyn crate::audio::AudioDecode>> {
    let decoder: Box<dyn crate::audio::AudioDecode> = match codec {
        "mp4a" => Box::new(aac_decoder::AacDecoder::new(extra_data, sample_rate, channels)?),
        "A_VORBIS" => {
            Box::new(vorbis_decoder::VorbisDecoder::new(extra_data, sample_rate, channels)?)
        }
        other => return Err(crate::invalid(&format!("unsupported audio codec {other}"))),
    };
    Ok(decoder)
}
