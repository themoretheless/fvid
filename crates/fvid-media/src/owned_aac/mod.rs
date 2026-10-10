//! Owned AAC-LC framing, decoding and transforms. No foreign decoder or libav.
#![forbid(unsafe_code)]
pub mod aac_gain_control;
pub mod aac_main_predictor;
pub mod aac_ssr_ipqf;
pub mod aac_ssr_gain;
pub mod aac_ssr_synthesis;
pub mod aac_ssr_alignment;
mod aac_ssr_metadata;
pub mod aac_ps_data;
pub mod aac_ps_hybrid_filter;
pub mod aac_ps_hybrid;
pub mod aac_ps_decorrelation;
pub mod aac_ps_dsp;
pub mod aac_ps_native;
pub mod aac_ps_huffman;
pub mod aac_ps_dequant;
pub mod aac_ps_history;
pub mod aac_ps_mapping;
pub mod aac_ps_mixing;
pub mod aac_ps_matrix_controller;
pub mod aac_ps_interpolation;
pub mod aac_ps_phase_history;
mod aac_ps_mapping_tables;
mod aac_ps_huffman_tables;
pub mod aac_coupling;
pub mod aac_imdct;
pub mod aac_ld_synthesis;
pub mod aac_ld_bands;
pub mod aac_ld_ltp;
pub mod aac_ld_history;
pub mod aac_ld_channel;
pub mod aac_synthesis;
pub mod aac_sbr_bands;
pub mod aac_sbr_header;
pub mod aac_sbr_grid;
pub mod aac_sbr_controls;
pub mod aac_sbr_chirp;
pub mod aac_sbr_predictor;
pub mod aac_sbr_hf;
pub mod aac_sbr_limiter;
pub mod aac_sbr_energy;
pub mod aac_sbr_gain;
pub mod aac_sbr_mapping;
pub mod aac_sbr_assembly;
pub mod aac_sbr_buffers;
pub mod aac_sbr_prepare;
pub mod aac_sbr_dsp;
pub mod aac_sbr_qmf_dsp;
pub mod aac_sbr_ps;
mod aac_sbr_noise_table;
pub mod aac_sbr_huffman;
pub mod aac_sbr_coefficients;
pub mod aac_sbr_data;
pub mod aac_sbr_extension;
pub mod aac_sbr_history;
pub mod aac_sbr_dequant;
pub mod aac_sbr_qmf;
pub mod aac_sbr_synthesis_qmf;
pub mod aac_sbr_downsampled_qmf;
mod aac_sbr_qmf_window;
mod aac_sbr_huffman_tables;
pub mod aac_tns;
pub use fvid_control::error::BitstreamError as Error;
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
mod aac_band_tables;
pub mod aac_bands;

pub mod aac_pce;
pub mod adts;
pub mod adts_crc;
pub mod bits;
pub mod config;
fn unsupported(message: &str) -> Error {
    Error(message.into())
}

pub mod aac_geometry;

pub mod aac_coupling_syntax;

pub mod aac_channel;

pub mod aac_ics;
pub mod aac_ltp_syntax;
pub mod aac_ltp_history;
pub mod aac_ltp_analysis;

pub mod aac_pair;

pub mod aac_huffman;

pub mod aac_scalefactors;

pub mod aac_spectral;

pub mod aac_noise;

pub mod aac_pulse;

pub mod aac_quant;

pub mod aac_tns_syntax;

pub mod aac_native;

mod aac_huffman_tables;

pub use aac_native::{AacCheckpoint, NativeAacDecoder};
#[cfg(test)]
mod decoder_tests;

pub mod stream;
pub use stream::{decode_adts_pcm, AudioDecodeStats as AdtsPcmStats};

mod memory;

pub mod aac_ltp_channel;
