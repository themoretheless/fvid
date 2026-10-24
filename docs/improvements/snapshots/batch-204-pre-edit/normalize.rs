//! Constant-gain loudness normalization through owned PCM decoding and export.
use super::measure_loudness_file_controlled;
pub use crate::media_info::{NormalizeReport, NormalizeTarget};
use crate::{invalid, Result};
use std::path::Path;
pub use fvid_media::owned_normalize::{NormalizationPhase, NormalizationProgress, NormalizationProgressHook};
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
    normalize_file_controlled(source, destination, selected, weights, target, cancel, None)
}
/// Report measurement/export separately; cancellation never produces success.
pub fn normalize_file_controlled(
    source: &Path,
    destination: &Path,
    selected: Option<usize>,
    weights: &[f64],
    target: NormalizeTarget,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&NormalizationProgressHook>,
) -> Result<NormalizeReport> {
    target.validate().map_err(|e| invalid(&e))?;
    if !matches!(destination.extension().and_then(|s| s.to_str()),Some("wav" | "mka" | "mkv")) {
        return Err(invalid("owned loudness normalization requires .wav, .mka or .mkv output"));
    }
    let before = std::fs::metadata(source)?;
    let measure_hook = progress.map(|hook| hook.for_phase(NormalizationPhase::Measure));
    let measured =
        measure_loudness_file_controlled(source, selected, weights, cancel, measure_hook.as_ref())?;
    let plan = fvid_media::owned_normalize::GainPlan::new(&measured, target)
        .map_err(|e| invalid(&e))?;
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(invalid("source changed during loudness measurement"));
    }
    let export_hook = progress.map(|hook| hook.for_phase(NormalizationPhase::Export));
    let stats = crate::native_export::export_audio_pcm_selected(
        source,
        destination,
        None,
        plan.linear_gain,
        None,
        None,
        selected,
        cancel,
        export_hook.as_ref(),
    )?;
    Ok(NormalizeReport {
        source: measured,
        gain_db: plan.gain_db,
        peak_limited: plan.peak_limited,
        sample_frames: stats.sample_frames,
    })
}
