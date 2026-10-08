//! Owned AAC-LC framing, decoding and transforms. No foreign decoder or libav.
#![forbid(unsafe_code)]
pub mod aac_coupling;
pub mod aac_imdct;
pub mod aac_synthesis;
pub mod aac_sbr_bands;
pub mod aac_sbr_header;
pub mod aac_sbr_grid;
pub mod aac_sbr_controls;
pub mod aac_sbr_huffman;
pub mod aac_sbr_coefficients;
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
pub mod bits;
pub mod config;
fn unsupported(message: &str) -> Error {
    Error(message.into())
}

pub mod aac_geometry;

pub mod aac_coupling_syntax;

pub mod aac_channel;

pub mod aac_ics;

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
