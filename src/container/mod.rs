//! FVid-owned container readers. No external demultiplexer or codec backend.
pub mod adts;
pub mod audio_timeline;
pub mod avi;
pub mod mp4;
pub mod mp4_relocate;
pub mod mp4_write;
pub mod mp4_matroska;
pub mod mp4_concat;
pub mod matroska_write;
pub mod matroska_copy;
pub mod ogg;
pub mod opus_packet;
pub mod smf;
pub mod webm;
pub mod xm;

pub use fvid_media::owned_file_tags::FileTags;
