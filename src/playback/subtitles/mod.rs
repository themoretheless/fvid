//! Subtitle readers attached to supported video containers.
pub mod mp4_subtitles;
pub mod webm_subtitles;

pub use mp4_subtitles as mp4;
pub use webm_subtitles as webm;
