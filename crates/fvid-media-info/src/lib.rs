//! Shared media description schema, with no demuxer or codec dependencies.
#![forbid(unsafe_code)]
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
