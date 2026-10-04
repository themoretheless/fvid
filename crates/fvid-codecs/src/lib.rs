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
