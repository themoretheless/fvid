//! Own AAC-LC/HE-AAC decoding followed by the existing streaming PCM loudness meter.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
pub(crate) fn validate_configuration(source: &Path, options: &CopyOptions) -> Result<u32> {
    if !policies(options) {
        return Err("owned ADTS loudness requires stream 0 and no metadata edits".into());
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
    let config = reader.configuration();
    crate::owned_aac::stream::check_adts_decode_admission(reader.audio_specific_config(), options)
        .map_err(|e| e.to_string())?;
    let rate = config.sample_rate;
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
    match validate_configuration(source, options) {
        Ok(_) => {
            crate::owned_audio_plan::plan_decode_audio(source, &Default::default(), options).is_ok()
        }
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    match validate_configuration(source, options) {
        Ok(_) => crate::owned_adts_export::supports(source, Default::default(), options),
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    validate_configuration(source, options)?;
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
    stats.backend = "owned ADTS AAC loudness";
    if let Some(hook) = &options.progress {
        hook.emit(ProgressEvent {
            done: true,
            ..spool.progress
        });
    }
    Ok(stats)
}
pub fn plan_loudness(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    validate_configuration(source, options)?;
    let mut plan =
        crate::owned_audio_plan::plan_decode_audio(source, &Default::default(), options)?;
    plan.command = "loudness".into();
    plan.streams[0].disposition = "analyze".into();
    plan.steps.retain(|s| s.action != "write");
    plan.steps.push(PlanStep{action:"analyze".into(),detail:"owned streaming K-weighting, integrated loudness gates, LRA and true-peak FIR; no published output".into()});
    plan.notes.push("private decoded WAVE is removed on success or failure; measurement completion is emitted once after analysis".into());
    Ok(plan)
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn aac_loudness_and_normalization_keep_owned_budget_policy() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/aac-stereo.aac");
        let small = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        let admitted = CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        assert!(supports(&source, &small));
        assert!(supports_plan(&source, &small));
        assert!(
            crate::measure_loudness(&source, &small)
                .unwrap_err()
                .contains("controlled memory budget exceeded")
        );
        let expected = crate::measure_loudness(&source, &Default::default()).unwrap();
        let actual = crate::measure_loudness(&source, &admitted).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        crate::plan_loudness(&source, &admitted).unwrap();
        for dual in [false, true] {
            let output = std::env::temp_dir().join(format!(
                "fvid-aac-loudnorm-budget-{dual}-{}.wav",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            assert!(crate::owned_adts_loudnorm::supports(
                &source, &output, None, &small
            ));
            let apply = |options: &CopyOptions| {
                if dual {
                    crate::apply_loudnorm_dual(&source, &output, None, options)
                } else {
                    crate::apply_loudnorm(&source, &output, None, options)
                }
            };
            assert!(
                apply(&small)
                    .unwrap_err()
                    .contains("controlled memory budget exceeded")
            );
            assert!(!output.exists());
            apply(&admitted).unwrap();
            let with_budget = std::fs::read(&output).unwrap();
            std::fs::remove_file(&output).unwrap();
            apply(&CopyOptions::default()).unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), with_budget);
            std::fs::remove_file(&output).unwrap();
        }
    }
}
