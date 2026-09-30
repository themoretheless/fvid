//! Shared media descriptions and operation results, with no demuxer or codec dependencies.
#![forbid(unsafe_code)]
mod video;
pub use video::{CropRect, ScaleSize, OverlaySpec, XfadeSpec, LosslessTransform, DecodeTransform, TransposeMode, PadRect, RotateAngle};

use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub index: usize,
    pub media_type: String,
    pub codec: String,
    pub time_base: [i32; 2],
    pub start: Option<i64>,
    pub duration: Option<i64>,
    pub bit_rate: Option<i64>,
    pub average_frame_rate: [i32; 2],
    pub profile: Option<String>,
    pub level: Option<i32>,
    pub disposition: i32,
    pub metadata: BTreeMap<String, String>,
    pub width: i32,
    pub height: i32,
    pub pixel_format: i32,
    pub sample_rate: i32,
    pub channels: i32,
    pub video_delay: i32,
    pub extradata_bytes: usize,
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChapterInfo {
    pub id: i64,
    pub time_base: [i32; 2],
    pub start: i64,
    pub end: i64,
    pub metadata: BTreeMap<String, String>,
}
#[derive(Serialize, Debug)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub format: String,
    pub start_us: Option<i64>,
    pub duration_us: Option<i64>,
    pub bit_rate: Option<i64>,
    pub metadata: BTreeMap<String, String>,
    pub chapters: Vec<ChapterInfo>,
    pub streams: Vec<StreamInfo>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlanStep {
    pub action: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlanStream {
    pub index: usize,
    pub media_type: String,
    pub codec: String,
    pub disposition: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MediaPlan {
    pub command: String,
    /// Primary / first input (kept for remux and lossless plans).
    pub input: PathBuf,
    /// All inputs (trim: one; concat: N≥2).
    pub inputs: Vec<PathBuf>,
    pub streams: Vec<PlanStream>,
    pub steps: Vec<PlanStep>,
    /// FFmpeg-equivalent primary video filter chain (`-vf`), when materializing.
    pub graph: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioDecodeTransform {
    /// Half-open presentation interval in microseconds from the first decoded sample.
    pub interval: Option<(i64, i64)>,
    /// Target sample rate; `None` keeps the decoded rate.
    pub sample_rate: Option<i32>,
    /// Target channel count; `None` keeps the decoded layout. Supported rematrix
    /// layouts depend on the selected execution backend.
    pub channels: Option<i32>,
    /// Linear gain on decoded float PCM. Native execution applies it after
    /// channel conversion and before resampling; consult the operation plan.
    pub volume: Option<f64>,
}


#[derive(Serialize, Default, Debug)]
pub struct CopyStats {
    pub packets: u64,
    pub payload_bytes: u64,
    pub segments: usize,
    pub backend: &'static str,
    pub fvid_payload_copies: u64,
}

#[derive(Serialize, Debug)]
pub struct DecodeStats {
    pub backend: &'static str,
    pub video_frames: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    /// Reads the demuxer reported as damaged, which were skipped while keeping what the stream had
    /// already produced. Non-zero means the format's demuxer ends the stream with an error code
    /// instead of EOF; the count is reported rather than swallowed, so a tolerant read stays
    /// auditable.
    pub decode_errors: u64,
}

#[derive(Serialize, Debug)]
pub struct AudioDecodeStats {
    pub sample_frames: u64,
    pub decoded_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    pub sample_format: String,
    pub planar_interleave_bytes: u64,
    /// Packets or frames skipped by a tolerant decoder. Strict backends return an
    /// error rather than producing a successful result with damaged media.
    pub decode_errors: u64,
}

#[derive(Serialize, Debug, Clone, Copy)]
pub struct PcmTrimStats {
    /// Packets or aligned I/O blocks, depending on the selected container path.
    pub packets: u64,
    pub sample_frames: u64,
    pub payload_bytes: u64,
    pub fvid_payload_copies: u64,
}

#[derive(Serialize, Debug)]
pub struct LosslessStats {
    pub backend: &'static str,
    pub video_frames: u64,
    pub decoded_frames: u64,
    pub seek_used: bool,
    pub video_packets: u64,
    pub copied_packets: u64,
    pub trimmed_audio_sample_frames: u64,
    pub pixel_format: String,
    pub encoder: String,
    pub fvid_crop_payload_copies: u64,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
}

mod audio_mix;
pub use audio_mix::{MixDuration, MixAudioOptions, MixAudioStats, MergeAudioStats};

mod time;
pub use time::parse_time;
