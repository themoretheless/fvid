//! Constant-gain loudness normalization through owned PCM decoding and export.
use super::measure_loudness_file_controlled;
pub use crate::media_info::{NormalizeReport, NormalizeTarget};
use crate::{invalid, Result};
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationPhase {
    Measure,
    Export,
}
impl NormalizationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Measure => "measure",
            Self::Export => "export",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct NormalizationProgress {
    pub phase: NormalizationPhase,
    pub packets: u64,
    pub payload_bytes: u64,
    pub phase_complete: bool,
    /// True only after output has been atomically published.
    pub done: bool,
}
#[derive(Clone)]
pub struct NormalizationProgressHook(std::sync::Arc<dyn Fn(NormalizationProgress) + Send + Sync>);
impl NormalizationProgressHook {
    pub fn new<F: Fn(NormalizationProgress) + Send + Sync + 'static>(callback: F) -> Self {
        Self(std::sync::Arc::new(callback))
    }
    fn for_phase(&self, phase: NormalizationPhase) -> crate::media_control::ProgressHook {
        let callback = self.0.clone();
        crate::media_control::ProgressHook::new(move |event| {
            callback(NormalizationProgress {
                phase,
                packets: event.packets,
                payload_bytes: event.payload_bytes,
                phase_complete: event.done,
                done: event.done && phase == NormalizationPhase::Export,
            })
        })
    }
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
    let export_hook = progress.map(|hook| hook.for_phase(NormalizationPhase::Export));
    let stats = crate::native_export::export_audio_pcm_selected(
        source,
        destination,
        None,
        gain,
        None,
        None,
        selected,
        cancel,
        export_hook.as_ref(),
    )?;
    Ok(NormalizeReport {
        source: measured,
        gain_db,
        peak_limited: ceiling < desired,
        sample_frames: stats.sample_frames,
    })
}
