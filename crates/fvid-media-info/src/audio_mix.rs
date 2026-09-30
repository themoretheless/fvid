use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixDuration {
    #[default]
    Shortest,
}

impl MixDuration {
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        match value {
            "shortest" => Ok(Self::Shortest),
            _ => Err("mix-audio duration must be shortest (v1)".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shortest => "shortest",
        }
    }
}

#[derive(Clone, Debug)]
pub struct MixAudioOptions {
    pub duration: MixDuration,
    /// Match FFmpeg `amix=normalize=1` (scale by weight sum). `false` → weighted sum.
    pub normalize: bool,
    /// Per-input weights (FFmpeg `weights`). Empty → all 1.0. Shorter lists repeat the last weight.
    pub weights: Vec<f32>,
}

impl Default for MixAudioOptions {
    fn default() -> Self {
        Self {
            duration: MixDuration::Shortest,
            normalize: true,
            weights: Vec::new(),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct MixAudioStats {
    pub backend: &'static str,
    pub sample_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    pub sample_format: String,
    pub inputs: usize,
    pub duration: &'static str,
    pub normalize: bool,
    pub weights: Vec<f32>,
}

#[derive(Serialize, Debug)]
pub struct MergeAudioStats {
    pub backend: &'static str,
    pub sample_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    pub sample_format: String,
    pub inputs: usize,
}
