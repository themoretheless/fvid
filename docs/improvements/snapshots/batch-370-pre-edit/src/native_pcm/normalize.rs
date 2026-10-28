//! Constant-gain loudness normalization through owned PCM decoding and export.
use super::measure_loudness_file_with_options;
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
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    options.cancel = cancel.cloned();
    normalize_file_with_options(source, destination, weights, target, &options, progress)
}
/// Normalize with packet, memory, and process RSS budgets.
pub fn normalize_file_with_options(
    source: &Path,
    destination: &Path,
    weights: &[f64],
    target: NormalizeTarget,
    options: &fvid_media::CopyOptions,
    progress: Option<&NormalizationProgressHook>,
) -> Result<NormalizeReport> {
    target.validate().map_err(|e| invalid(&e))?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let ext = destination.extension().and_then(|s| s.to_str());
    if !matches!(ext, Some("wav" | "mka" | "mkv")) {
        return Err(invalid("owned loudness normalization requires .wav, .mka or .mkv output"));
    }
    let matroska_out = matches!(ext, Some("mka" | "mkv"));
    if matroska_out {
        crate::native_export::pcm_matroska::validate_matroska_pcm_metadata(Some(options))?;
    } else {
        if !options.stream_metadata_set.is_empty() || !options.stream_metadata_delete.is_empty() {
            return Err(invalid("normalize-loudness to .wav does not support stream metadata mutations"));
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )
        .map_err(|e| invalid(&e))?;
    }
    let before = std::fs::metadata(source)?;
    let measure_hook = progress.map(|hook| hook.for_phase(NormalizationPhase::Measure));
    let mut measure_options = options.clone();
    measure_options.metadata_set.clear();
    measure_options.metadata_delete.clear();
    measure_options.stream_metadata_set.clear();
    measure_options.stream_metadata_delete.clear();
    measure_options.progress = measure_hook;
    let measured = measure_loudness_file_with_options(source, weights, &measure_options)?;
    let plan = fvid_media::owned_normalize::GainPlan::new(&measured, target)
        .map_err(|e| invalid(&e))?;
    let after = std::fs::metadata(source)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(invalid("source changed during loudness measurement"));
    }
    let export_hook = progress.map(|hook| hook.for_phase(NormalizationPhase::Export));
    let selected = options.streams.first().copied();
    let use_native_export = options.max_controlled_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && (super::is_wave(source)?
            || crate::native_media::audio_source_info_selected_with_options(source, selected, options).is_ok());
    let sample_frames = if use_native_export {
        let mut export_options = options.clone();
        export_options.progress = export_hook;
        crate::native_export::export_audio_pcm_transformed_with_controls(
            source,
            destination,
            None,
            plan.linear_gain,
            None,
            None,
            &export_options,
        )?
        .sample_frames
    } else {
        let mut export_options = options.clone();
        export_options.progress = export_hook;
        let transform = fvid_media::AudioDecodeTransform {
            volume: Some(plan.linear_gain),
            ..Default::default()
        };
        crate::media::decode_fvid_media_audio_to_destination(source, destination, transform, &export_options)
            .map_err(|e| invalid(&e))?
            .sample_frames
    };
    Ok(NormalizeReport {
        source: measured,
        gain_db: plan.gain_db,
        peak_limited: plan.peak_limited,
        sample_frames,
    })
}
