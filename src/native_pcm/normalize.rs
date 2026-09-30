//! Constant-gain loudness normalization through owned PCM decoding and export.
use super::{measure_loudness_file, IntegratedLoudness};
use crate::{invalid, Result};
use std::path::Path;
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug)]
pub struct NormalizeReport {
    pub source: IntegratedLoudness,
    pub gain_db: f64,
    pub peak_limited: bool,
    pub sample_frames: u64,
}
/// Normalize with one constant gain, preserving channels, rate and dynamics.
/// Silence and streams without a complete loudness window are rejected.
/// Destination publication follows owned export's no-overwrite/atomic rules.
pub fn normalize_file(
    source: &Path,
    destination: &Path,
    selected: Option<usize>,
    weights: &[f64],
    target: NormalizeTarget,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<NormalizeReport> {
    target.validate().map_err(|e| invalid(&e))?;
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err(invalid("owned loudness normalization requires WAVE output"));
    }
    let before = std::fs::metadata(source)?;
    let measured = measure_loudness_file(source, selected, weights, cancel)?;
    let desired = target.integrated_lufs
        - measured
            .integrated_lufs
            .ok_or_else(|| invalid("normalization requires non-silent audio of at least 400 ms"))?;
    let ceiling = target.sample_peak_dbfs
        - measured
            .sample_peak_dbfs
            .ok_or_else(|| invalid("normalization requires a nonzero sample peak"))?;
    let gain_db = desired.min(ceiling);
    let gain = 10f64.powf(gain_db / 20.0);
    if gain > 64.0 {
        return Err(invalid(
            "required normalization gain exceeds the owned export limit of 64",
        ));
    }
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(invalid("source changed during loudness measurement"));
    }
    let stats = crate::native_export::export_audio_pcm_selected(
        source,
        destination,
        None,
        gain,
        None,
        None,
        selected,
        cancel,
        None,
    )?;
    Ok(NormalizeReport {
        source: measured,
        gain_db,
        peak_limited: ceiling < desired,
        sample_frames: stats.sample_frames,
    })
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
