//! Own AAC-LC decoding followed by the existing streaming PCM loudness meter.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.max_controlled_bytes.is_none()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
fn rate(source: &Path, options: &CopyOptions) -> Result<u32> {
    if !policies(options) {
        return Err("owned ADTS loudness requires stream 0, no metadata edits and no aggregate admission policy".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let rate = reader.configuration().sample_rate;
    let decoder = crate::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
        .map_err(|e| e.to_string())?;
    if decoder.channel_mask() & !0x7ff != 0 {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(rate) {
        return Err(
            "owned AAC true-peak measurement requires a qualified standard PCM rate".into(),
        );
    }
    Ok(rate)
}
pub(crate) fn supports_plan(source: &Path, options: &CopyOptions) -> bool {
    rate(source, options).is_ok()
        && crate::owned_audio_plan::plan_decode_audio(source, &Default::default(), options).is_ok()
}
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    rate(source, options).is_ok()
        && crate::owned_adts_export::supports(source, Default::default(), options)
}
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    rate(source, options)?;
    let spool = crate::owned_adts_export::decode_to_wave(source, None, options)?;
    let mut pcm_options = options.clone();
    pcm_options.max_packets = None;
    pcm_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    pcm_options.progress = None;
    let mut stats = crate::owned_wave_loudness::measure_loudness(&spool.wave, &pcm_options)?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    stats.backend = "owned ADTS AAC-LC loudness";
    if let Some(hook) = &options.progress {
        hook.emit(ProgressEvent {
            done: true,
            ..spool.progress
        });
    }
    Ok(stats)
}
pub fn plan_loudness(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    rate(source, options)?;
    let mut plan =
        crate::owned_audio_plan::plan_decode_audio(source, &Default::default(), options)?;
    plan.command = "loudness".into();
    plan.streams[0].disposition = "analyze".into();
    plan.steps.retain(|s| s.action != "write");
    plan.steps.push(PlanStep{action:"analyze".into(),detail:"owned streaming K-weighting, integrated loudness gates, LRA and true-peak FIR; no published output".into()});
    plan.notes.push("private decoded WAVE is removed on success or failure; measurement completion is emitted once after analysis".into());
    Ok(plan)
}
