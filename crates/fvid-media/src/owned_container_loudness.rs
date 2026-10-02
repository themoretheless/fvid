//! Owned container audio decoding followed by the streaming PCM loudness meter.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() {
        return Ok(false);
    }
    Ok(prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] || crate::owned_mp4::recognizes_prefix(&prefix))
}
fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32, String)> {
    if options.streams.len() > 1
        || options.max_controlled_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("owned container loudness requires one audio stream, no metadata edits and no aggregate admission policy".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !recognizes(source)? {
        return Err("source is not an owned audio container".into());
    }
    let (_, rate, channels, mask, codec) =
        if crate::owned_mp4_audio_export::recognizes(source, options)? {
            crate::owned_mp4_audio_export::descriptor(source, options)?
        } else {
            let (index, rate, channels, mask, codec, _) =
                crate::owned_audio_plan::matroska_descriptor(source, options)?
                    .ok_or("source is not a supported Matroska audio container")?;
            (index, rate, channels, mask, codec)
        };
    if mask == 0 || mask & !0x7ff != 0 {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(rate) {
        return Err("owned true-peak measurement requires a qualified standard PCM rate".into());
    }
    Ok((rate, channels, mask, codec))
}
pub(crate) fn supports_plan(source: &Path, options: &CopyOptions) -> bool {
    geometry(source, options).is_ok()
}
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    geometry(source, options).is_ok()
        && crate::owned_audio_export::supports(
            source,
            Path::new("owned.wav"),
            Default::default(),
            options,
        )
}
pub(crate) fn decode_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    let (rate, channels, mask, codec) = geometry(source, options)?;
    let mp4 = crate::owned_mp4_audio_export::recognizes(source, options)?;
    let precise = codec.starts_with("A_PCM/");
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        if precise { 64 } else { 32 },
        options,
        |writer, options| {
            let file = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
            let result = if mp4 {
                crate::owned_mp4_audio::decode_mp4_audio_pcm(file, writer, None, options)
            } else if codec == "aac" {
                crate::owned_matroska_aac::decode_matroska_aac_pcm(file, writer, None, options)
            } else if codec == "alac" {
                crate::owned_matroska_alac::decode_matroska_alac_pcm(file, writer, None, options)
            } else {
                crate::owned_matroska_pcm::decode_matroska_pcm_f64(file, writer, None, options)
            };
            result.map_err(|e| e.to_string())
        },
    )?;
    Ok(spool)
}
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    let spool = decode_to_wave(source, options)?;
    let mut pcm_options = options.clone();
    pcm_options.streams = vec![0];
    pcm_options.max_packets = None;
    pcm_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    pcm_options.progress = None;
    let mut stats = crate::owned_wave_loudness::measure_loudness(&spool.wave, &pcm_options)?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    stats.backend = "owned container audio loudness";
    if let Some(hook) = &options.progress {
        hook.emit(ProgressEvent {
            done: true,
            ..spool.progress
        });
    }
    Ok(stats)
}
pub fn plan_loudness(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    geometry(source, options)?;
    let mut plan =
        crate::owned_audio_plan::plan_decode_audio(source, &Default::default(), options)?;
    plan.command = "loudness".into();
    plan.streams[0].disposition = "analyze".into();
    plan.steps.retain(|s| s.action != "write");
    plan.steps.push(PlanStep { action:"analyze".into(),detail:"owned streaming K-weighting, integrated gates, LRA and true-peak FIR; no published output".into() });
    plan.notes.push("private decoded WAVE is removed on success or failure; completion is emitted once after analysis".into());
    Ok(plan)
}
