//! Backend-independent loudness results and argument compatibility.
use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct LoudnessStats {
    pub backend: &'static str,
    pub sample_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    /// Integrated loudness (LUFS).
    pub integrated_lufs: f64,
    /// Loudness range (LU).
    pub range_lu: f64,
    pub lra_low_lufs: f64,
    pub lra_high_lufs: f64,
    /// True peak (dBFS); requires ebur128 peak=true.
    pub true_peak_dbfs: f64,
    pub sample_peak_dbfs: f64,
}

#[derive(Serialize, Debug, Clone)]
pub struct LoudnormStats {
    pub backend: &'static str,
    pub sample_frames: u64,
    pub sample_rate: i32,
    pub channels: i32,
    /// Filter args after defaults applied (FFmpeg `-af loudnorm=`).
    pub args: String,
    /// True when a measure pass supplied `measured_*` + `linear=true`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dual_pass: bool,
}

/// Default FFmpeg `loudnorm` targets (I/TP/LRA).
pub const DEFAULT_LOUDNORM_ARGS: &str = "I=-16:TP=-1.5:LRA=11";

pub fn validate_loudnorm_args(args: &str) -> std::result::Result<(), String> {
    if args.is_empty() {
        return Ok(());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "loudnorm args must match [A-Za-z0-9=.:_-] (FFmpeg loudnorm= key/value list)".into(),
        );
    }
    Ok(())
}

pub fn resolve_loudnorm_args(args: Option<&str>) -> std::result::Result<String, String> {
    match args {
        None | Some("") => Ok(DEFAULT_LOUDNORM_ARGS.into()),
        Some(value) => {
            validate_loudnorm_args(value)?;
            Ok(value.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn argument_defaults_and_safe_filter_values() {
        for value in [None, Some("")] {
            assert_eq!(resolve_loudnorm_args(value).unwrap(), DEFAULT_LOUDNORM_ARGS);
        }
        let explicit = "I=-23:TP=-2:LRA=7:linear=true:measured_I=-24.25";
        assert_eq!(resolve_loudnorm_args(Some(explicit)).unwrap(), explicit);
        for invalid in [
            "I=-16,volume=2",
            "I=-16;anull",
            "I=-16[output]",
            "I=-16\n",
            "I=−16",
        ] {
            assert!(validate_loudnorm_args(invalid).is_err(), "{invalid}");
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct PcmLoudnessStats {
    pub sample_frames: u64,
    pub measured_blocks: u64,
    /// None for silence or streams shorter than a complete 400 ms window.
    pub integrated_lufs: Option<f64>,
    /// Unweighted peak across all input channels; None for silence/empty input.
    pub sample_peak_dbfs: Option<f64>,
    /// Gated short-term 10th-to-95th percentile range; None without valid 3 s windows.
    pub range_lu: Option<f64>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct NormalizeTarget {
    pub integrated_lufs: f64,
    /// Sample-peak ceiling, not an interpolated true-peak ceiling.
    pub sample_peak_dbfs: f64,
}
impl Default for NormalizeTarget {
    fn default() -> Self {
        Self {
            integrated_lufs: -16.0,
            sample_peak_dbfs: -1.5,
        }
    }
}
#[derive(Debug, serde::Serialize)]
pub struct NormalizeReport {
    pub source: PcmLoudnessStats,
    pub gain_db: f64,
    pub peak_limited: bool,
    pub sample_frames: u64,
}

impl NormalizeTarget {
    pub fn validate(self) -> std::result::Result<(), String> {
        if !self.integrated_lufs.is_finite()
            || !(-70.0..=0.0).contains(&self.integrated_lufs)
            || !self.sample_peak_dbfs.is_finite()
            || !(-70.0..=0.0).contains(&self.sample_peak_dbfs)
        {
            return Err("normalization targets must be finite and within -70..=0 dB".into());
        }
        Ok(())
    }
}
