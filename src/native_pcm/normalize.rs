//! Constant-gain loudness normalization through owned PCM decoding and export.
use super::measure_loudness_file;
pub use crate::media_info::{NormalizeReport, NormalizeTarget};
use crate::{invalid, Result};
use std::path::Path;
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
