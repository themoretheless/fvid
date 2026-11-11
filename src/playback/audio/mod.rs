//! Audio readers by source/container. The old `crate::playback_*` names remain supported.
pub mod aac;
pub mod ac3;
pub mod aiff;
pub mod au;
pub mod avi_audio;
pub mod flac;
pub mod mp3;
pub mod mp4_audio;
pub mod ogg_audio;
pub mod ogg_catalog;
pub mod ogg_opus;
pub mod smf;
pub mod wav;
pub mod wavpack;
pub mod webm_audio;
pub mod xm;

pub use avi_audio as avi;
pub use mp4_audio as mp4;
pub use ogg_audio as ogg;
pub use webm_audio as webm;
