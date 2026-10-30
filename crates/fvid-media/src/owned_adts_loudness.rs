//! Own AAC-LC/HE-AAC decoding followed by the existing streaming PCM loudness meter.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep, PlanStream};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn policies(options: &CopyOptions) -> bool {
    (options.streams.is_empty() || options.streams == [0])
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
}
pub(crate) fn validate_header_configuration(source: &Path, options: &CopyOptions) -> Result<u32> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    if !policies(options) {
        return Err("owned ADTS loudness requires stream 0 and no metadata edits".into());
    }
    let reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )
    .map_err(|e| e.to_string())?;
    let config = reader.configuration();
    crate::owned_aac::stream::check_adts_decode_admission(reader.audio_specific_config(), options)
        .map_err(|e| e.to_string())?;
    let decoder = crate::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
        .map_err(|e| e.to_string())?;
    if decoder.channel_mask() & !0x7ff != 0 {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(config.sample_rate)
        && !crate::owned_wave_loudness::qualified_rate(config.sample_rate.saturating_mul(2))
    {
        return Err(
            "owned AAC true-peak measurement requires a qualified standard PCM rate".into(),
        );
    }
    Ok(config.sample_rate)
}
fn validate_packet_budget(source: &Path, options: &CopyOptions) -> Result<()> {
    let packet_limit = options
        .max_packet_bytes
        .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
    if packet_limit >= CopyOptions::default().max_packet_bytes {
        return Ok(());
    }
    let mut reader = crate::owned_aac::adts::StreamReader::open_with_packet_limit(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        packet_limit,
    )
    .map_err(|e| e.to_string())?;
    let mut packets = 0u64;
    while !options.max_packets.is_some_and(|limit| packets >= limit) {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err("media operation cancelled".into());
        }
        crate::owned_budget::check_rss_budget(options)?;
        if reader.next_packet().map_err(|e| e.to_string())?.is_none() {
            break;
        }
        packets += 1;
    }
    Ok(())
}
pub(crate) fn validate_configuration(source: &Path, options: &CopyOptions) -> Result<u32> {
    let header_rate = validate_header_configuration(source, options)?;
    validate_packet_budget(source, options)?;
    if crate::owned_wave_loudness::qualified_rate(header_rate) {
        return Ok(header_rate);
    }
    let (rate, _channels, mask) = crate::owned_aac::stream::adts_prefix_info(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        None,
        options,
    )
    .map_err(|e| e.to_string())?;
    if mask & !0x7ff != 0 {
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
        Ok(_) => true,
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
    validate_header_configuration(source, options)?;
    let spool = crate::owned_adts_export::decode_to_wave(source, None, options)?;
    if !crate::owned_wave_loudness::qualified_rate(spool.stats.sample_rate) {
        return Err(
            "owned AAC true-peak measurement requires a qualified standard PCM rate".into(),
        );
    }
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
    let mut notes = vec![
        "backend: owned fvid-media; no external demuxer, codec, resampler or muxer".into(),
        "read-only metadata preflight: packet tools, payload validity, DSP allocation admission and publication are checked during execution".into(),
        "packet byte/count limits apply to selected encoded audio, including preroll; internal PCM blocks are excluded".into(),
        "private decoded WAVE is removed on success or failure; measurement completion is emitted once after analysis".into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {} packets; controlled memory limit: {} bytes; RSS limit: {} bytes",
            options.max_packet_bytes,
            options
                .max_packets
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options
                .max_controlled_bytes
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options
                .max_rss_bytes
                .map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
        ));
    }
    Ok(MediaPlan {
        command: "loudness".into(),
        input: source.into(),
        inputs: vec![source.into()],
        streams: vec![PlanStream {
            index: 0,
            media_type: "audio".into(),
            codec: "aac".into(),
            disposition: "analyze".into(),
        }],
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "owned AAC-LC/HE-AAC decoder; negotiate implicit SBR output clock; retain ADTS priming and preroll; private float32 WAVE disk spool".into(),
            },
            PlanStep {
                action: "analyze".into(),
                detail: "owned streaming K-weighting, integrated loudness gates, LRA and true-peak FIR; no published output".into(),
            },
        ],
        graph: None,
        notes,
    })
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
        assert!(!supports(&source, &small));
        assert!(!supports_plan(&source, &small));
        assert_eq!(
            crate::measure_loudness(&source, &small).unwrap_err(),
            "ADTS packet exceeds budget"
        );
        let small = CopyOptions {
            max_controlled_bytes: Some(8192),
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
