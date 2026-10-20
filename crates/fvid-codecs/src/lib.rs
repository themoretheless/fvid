//! Owned video decoding below the frontend and media-operation libraries.
//! Canonical codec sources remain at their existing paths; root FVid reexports
//! these modules, so the decoding implementations compile in one library.
#![forbid(unsafe_code)]
pub use fvid_control::error::{Error, Result};
pub(crate) use fvid_control::error::{buffer, invalid, unsupported};
pub mod codec;
pub mod color;
#[cfg(test)]
mod container {
    pub use fvid_media::owned_mp4 as mp4;
    pub use fvid_media::owned_webm as webm;
}

pub mod vorbis;

pub mod vorbis_synthesis;

pub mod vorbis_imdct;

pub mod vorbis_audio;

pub mod vorbis_timeline;

pub mod vorbis_admission;

pub mod g711;

pub mod flac;

pub mod mpeg_synthesis;

pub mod mp1;
pub mod mp2;

pub mod mp3_hybrid;

pub mod mp3_frame;

pub mod mp3_huffman;

pub mod mp3_scalefactors;

pub mod mp3_spectral;

pub mod mp3_stereo;

pub mod mp3;

pub mod mp3_gapless;
