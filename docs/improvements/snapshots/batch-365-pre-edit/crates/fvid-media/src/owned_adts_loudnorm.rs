//! AAC-LC/HE-AAC normalization via owned decoding, private spool and the owned DSP.
use fvid_control::{CopyOptions, ProgressHook};
use fvid_media_info::{LoudnormStats, MediaPlan, PlanStep};
use std::path::Path;
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    crate::owned_audio_export::simple_options(options)
        && options.metadata_set.len() + options.metadata_delete.len() <= 64
        && !options
            .metadata_set
            .iter()
            .any(|(_, value)| value.contains('\0'))
}
pub(crate) fn supports(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    options: &CopyOptions,
) -> bool {
    policies(options)
        && destination.extension().and_then(|s| s.to_str()) == Some("wav")
        && crate::owned_loudnorm::validate_request(args).is_ok()
        && crate::owned_adts_loudness::supports(
            source,
            &crate::owned_adts_export::pcm_decode_options(options),
        )
}
/// Metadata-only qualification for frontend dispatch; packet tools are checked during execution.
pub fn supports_plan(source: &Path, args: Option<&str>, options: &CopyOptions) -> bool {
    policies(options)
        && crate::owned_loudnorm::validate_request(args).is_ok()
        && crate::owned_adts_loudness::supports_plan(
            source,
            &crate::owned_adts_export::pcm_decode_options(options),
        )
}
pub fn apply(
    source: &Path,
    destination: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<LoudnormStats> {
    if !policies(options) {
        return Err("owned ADTS loudnorm requires stream 0 and valid WAVE INFO metadata edits".into());
    }
    let resolved = crate::owned_loudnorm::validate_request(args)?;
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("wav") {
        return Err("owned ADTS loudnorm requires .wav output".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    crate::owned_adts_export::check_wave_metadata_budget(options)?;
    let decode_options = crate::owned_adts_export::pcm_decode_options(options);
    // Read-only configuration/policy preflight; actual packet failures propagate
    // once execution starts, without retry through the legacy backend.
    crate::owned_adts_loudness::validate_header_configuration(source, &decode_options)?;
    let spool = crate::owned_adts_export::decode_to_wave(source, None, &decode_options)?;
    if !crate::owned_wave_loudness::qualified_rate(spool.stats.sample_rate) {
        return Err(
            "owned AAC true-peak measurement requires a qualified standard PCM rate".into(),
        );
    }
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
    let stats = if dual_pass {
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
    Ok(stats)
}
pub fn plan_loudnorm(
    source: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !policies(options) {
        return Err("owned ADTS loudnorm requires stream 0 and valid WAVE INFO metadata edits".into());
    }
    let resolved = crate::owned_loudnorm::validate_request(args)?;
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_adts_export::check_wave_metadata_budget(options)?;
    let decode_options = crate::owned_adts_export::pcm_decode_options(options);
    let mut plan = crate::owned_adts_loudness::plan_loudness(source, &decode_options)?;
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
    if crate::owned_loudnorm::report_requested(Some(&resolved))? {
        plan.steps.push(PlanStep{action:"analyze-output".into(),detail:"owned output loudness/true-peak measurement before publication; print JSON/summary after successful publication".into()});
    }
    if !options.metadata_set.is_empty() || !options.metadata_delete.is_empty() {
        plan.steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply owned RIFF INFO container tag edits to final WAVE".into(),
        });
    }
    plan.notes.push("normalization backend: fvid; owned AAC decode and WAVE normalization; dynamic PCM is not claimed equivalent to libavfilter".into());
    plan.steps.push(PlanStep{action:"write".into(),detail:"owned float32 .wav writer; no overwrite; completion after publication; remove private spool on every outcome".into()});
    Ok(plan)
}
