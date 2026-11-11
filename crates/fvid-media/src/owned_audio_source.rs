//! Unified source descriptors and backend contract for owned audio WAVE export.
//!
//! Replaces positional 9- and 10-element geometry tuples across Matroska, MP4,
//! Ogg, raw FLAC/MP3, ADTS, and G.711 audio export backends with a named
//! [`AudioSourceDescriptor`] and [`AudioExportSource`] contract.

use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::path::Path;

/// Common preflight & metadata descriptor returned by an audio export backend.
#[derive(Clone, Debug)]
pub(crate) struct AudioSourceDescriptor {
    pub(crate) stream_index: usize,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) channel_mask: u32,
    pub(crate) codec: String,
    pub(crate) precision: &'static str,
    pub(crate) estimated_frames: u64,
    pub(crate) metadata_bytes: usize,
    pub(crate) info_chunk: Option<Vec<u8>>,
    pub(crate) mp4_target: Option<(u32, u16, u32)>,
    pub(crate) aac_fold: bool,
}

impl AudioSourceDescriptor {
    pub(crate) fn float32(
        stream_index: usize,
        sample_rate: u32,
        channels: u16,
        channel_mask: u32,
        codec: impl Into<String>,
        estimated_frames: u64,
        metadata_bytes: usize,
    ) -> Self {
        Self {
            stream_index,
            sample_rate,
            channels,
            channel_mask,
            codec: codec.into(),
            precision: "float32",
            estimated_frames,
            metadata_bytes,
            info_chunk: None,
            mp4_target: None,
            aac_fold: false,
        }
    }

    pub(crate) fn with_info(mut self, info: Vec<u8>) -> Self {
        self.metadata_bytes = info.len();
        self.info_chunk = Some(info);
        self
    }

    pub(crate) fn with_precision(mut self, precision: &'static str) -> Self {
        self.precision = precision;
        self
    }

    pub(crate) fn with_aac_fold(mut self, aac_fold: bool) -> Self {
        self.aac_fold = aac_fold;
        self
    }

    pub(crate) fn with_mp4_target(mut self, target: Option<(u32, u16, u32)>) -> Self {
        self.mp4_target = target;
        self
    }
}

/// Polymorphic contract for an already-opened container/stream audio export source.
pub(crate) trait AudioExportSource {
    /// Extract source geometry, metadata working-set size, and frame estimate for planning.
    fn descriptor(
        self,
        transform: Option<AudioDecodeTransform>,
        options: &CopyOptions,
    ) -> Result<AudioSourceDescriptor, String>;

    /// Decode and write transformed WAVE audio to `destination`.
    fn apply(
        self,
        destination: &Path,
        transform: AudioDecodeTransform,
        options: &CopyOptions,
    ) -> Result<AudioDecodeStats, String>;
}
