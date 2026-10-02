//! AAC-LC normalization via owned decoding, private spool and the owned DSP.
use fvid_control::{CopyOptions, ProgressHook};
use fvid_media_info::{LoudnormStats, MediaPlan, PlanStep};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> bool {
    destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && crate::owned_loudnorm::validate_request(args).is_ok()
        && crate::owned_adts_loudness::supports(source, options)
}
pub(crate) fn supports_plan(source: &Path, args: Option<&str>, options: &CopyOptions) -> bool {
    crate::owned_loudnorm::validate_request(args).is_ok()
        && crate::owned_adts_loudness::supports_plan(source, options)
}
pub fn apply(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    let resolved = crate::owned_loudnorm::validate_request(args)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned ADTS loudnorm requires .wav output".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    // Read-only configuration/policy preflight; actual packet failures propagate
    // once execution starts, without retry through the legacy backend.
    crate::owned_adts_loudness::plan_loudness(source, options)?;
    let spool = crate::owned_adts_export::decode_to_wave(source, None, options)?;
    let mut pcm_options = options.clone();
    pcm_options.max_packets = None;
    pcm_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    let base = spool.progress;
    pcm_options.progress = options.progress.clone().map(|hook| {
        ProgressHook::new(move |mut event| {
            event.packets += base.packets;
            event.payload_bytes += base.payload_bytes;
            hook.emit(event);
        })
    });
    let mut stats = if dual_pass {
        crate::owned_loudnorm::apply_loudnorm_dual(
            &spool.wave,
            destination,
            Some(&resolved),
            &pcm_options,
        )?
    } else {
        crate::owned_loudnorm::apply_loudnorm(
            &spool.wave,
            destination,
            Some(&resolved),
            &pcm_options,
        )?
    };
    stats.backend = "owned ADTS AAC-LC loudnorm";
    Ok(stats)
}
pub fn plan_loudnorm(
    source: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    let resolved = crate::owned_loudnorm::validate_request(args)?;
    let mut plan = crate::owned_adts_loudness::plan_loudness(source, options)?;
    plan.command = "loudnorm".into();
    plan.notes
        .retain(|note| !note.contains("measurement completion"));
    plan.streams[0].disposition = "decode-normalize".into();
    if !dual_pass && !crate::owned_loudnorm::report_requested(Some(&resolved))? {
        plan.steps.retain(|s| s.action != "analyze");
    }
    plan.steps.push(PlanStep {
        action: "filter".into(),
        detail: format!(
            "owned loudnorm {resolved}; {}",
            if dual_pass {
                "measure first; eligible measured linear gain, otherwise dynamic normalization"
            } else {
                "requested measured linear gain when eligible, otherwise dynamic normalization"
            }
        ),
    });
    plan.steps.push(PlanStep{action:"write".into(),detail:"owned float32 .wav writer; no overwrite; completion after publication; remove private spool on every outcome".into()});
    Ok(plan)
}
