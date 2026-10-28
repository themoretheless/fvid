//! Owned container audio decoding followed by the streaming PCM loudness meter.
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{LoudnessStats, MediaPlan, PlanStep};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub fn recognizes_standalone_audio(source: &Path) -> Result<bool> {
    recognizes_standalone_audio_with_options(source, &Default::default())
}
pub fn recognizes_standalone_audio_with_options(
    source: &Path,
    options: &CopyOptions,
) -> Result<bool> {
    let mut prefix = [0; 4];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() {
        return Ok(false);
    }
    if &prefix == b"OggS" || &prefix == b"fLaC" {
        return Ok(true);
    }
    if matches!(&prefix, b"RIFF" | b"RIFX" | b"RF64") {
        return crate::owned_g711_wave_export::recognizes(source, options);
    }
    if crate::owned_adts_export::recognizes(source)? {
        return Ok(false);
    }
    crate::owned_raw_mp3::recognizes(source)
}
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    recognizes_with_options(source, &Default::default())
}
pub(crate) fn recognizes_with_options(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut prefix = [0; 10];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let count = file.read(&mut prefix).map_err(|e| e.to_string())?;
    if count < 4 {
        return Ok(false);
    }
    if prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3]
        || (count >= 8 && crate::owned_mp4::recognizes_prefix(&prefix[..8]))
    {
        return Ok(true);
    }
    recognizes_standalone_audio_with_options(source, options)
}
fn geometry_unqualified(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32, String)> {
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
    if options.streams.len() > 1
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(
            "owned container loudness requires one audio stream and no metadata edits".into(),
        );
    }
    if !recognizes_with_options(source, options)? {
        return Err("source is not an owned audio container".into());
    }
    let (_, rate, channels, mask, codec) =
        if crate::owned_mp4_audio_export::recognizes(source, options)? {
            crate::owned_mp4_audio_export::descriptor(source, options)?
        } else if crate::owned_g711_wave_export::recognizes(source, options)? {
            let (rate, channels, mask, codec, frames) =
                crate::owned_g711_wave_export::descriptor(source, options)?;
            if frames == 0 {
                return Err("loudness requires at least one audio frame".into());
            }
            (0, rate, channels, mask, codec.into())
        } else if crate::owned_ogg_opus_export::recognizes(source)? {
            let (index, rate, channels, mask, codec) =
                crate::owned_ogg_audio_export::descriptor(source, options)?;
            let effective_mask = if mask == 0 && channels <= 2 {
                crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
            } else {
                mask
            };
            (index, rate, channels, effective_mask, codec.into())
        } else if crate::owned_raw_flac::recognizes(source)? {
            let (rate, channels, mask) = crate::owned_raw_flac::descriptor(source, options)?;
            let effective_mask = if mask == 0 && channels <= 2 {
                crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
            } else {
                mask
            };
            (0, rate, channels, effective_mask, "flac".into())
        } else if crate::owned_raw_mp3::recognizes(source)? {
            let (rate, channels, codec) = crate::owned_raw_mp3::descriptor(source, options)?;
            let mask = crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32;
            (0, rate, channels, mask, codec.into())
        } else {
            let (index, rate, channels, mask, codec, _) =
                crate::owned_audio_plan::matroska_descriptor(source, options)?
                    .ok_or("source is not a supported Matroska audio container")?;
            (index, rate, channels, mask, codec)
        };
    Ok((rate, channels, mask, codec))
}
fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32, String)> {
    let (rate, channels, mask, codec) = geometry_unqualified(source, options)?;
    if mask == 0 || mask & !0x7ff != 0 {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(rate) {
        return Err("owned true-peak measurement requires a qualified standard PCM rate".into());
    }
    Ok((rate, channels, mask, codec))
}
pub(crate) fn supports_plan(source: &Path, options: &CopyOptions) -> bool {
    match geometry(source, options) {
        Ok(_) => true,
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    match geometry(source, options) {
        Ok(_) => crate::owned_audio_export::supports(
            source,
            Path::new("owned.wav"),
            Default::default(),
            options,
        ),
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
pub fn decode_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    let (rate, channels, mask, codec) = geometry(source, options)?;
    decode_to_wave_from_geometry(source, rate, channels, mask, &codec, options)
}
pub fn decode_to_wave_unqualified(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    let (rate, channels, mask, codec) = geometry_unqualified(source, options)?;
    decode_to_wave_from_geometry(source, rate, channels, mask, &codec, options)
}
pub fn decode_adts_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    crate::owned_adts_export::decode_to_wave(source, None, options)
}
fn decode_to_wave_from_geometry(
    source: &Path,
    rate: u32,
    channels: u16,
    mask: u32,
    codec: &str,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    if crate::owned_g711_wave_export::recognizes(source, options)? {
        return crate::owned_g711_wave_export::decode_to_wave(source, options);
    }
    if crate::owned_ogg_opus_export::recognizes(source)? {
        return match crate::owned_ogg_audio_export::coding(source, options)? {
            crate::owned_ogg_audio_export::Backend::Opus => {
                crate::owned_ogg_opus_export::decode_to_wave(source, options)
            }
            crate::owned_ogg_audio_export::Backend::Vorbis => {
                crate::owned_ogg_vorbis_export::decode_to_wave(source, options)
            }
            crate::owned_ogg_audio_export::Backend::Joint => {
                crate::owned_ogg_joint_export::decode_to_wave(source, options)
            }
        };
    }
    if crate::owned_raw_flac::recognizes(source)? {
        let info = crate::owned_raw_flac::source_info(source, options)?;
        let spool = crate::owned_adts_export::spool_decoded(
            rate,
            channels,
            mask,
            options,
            |writer, options| crate::owned_raw_flac::decode_raw_flac_pcm(source, writer, None, options),
        )?;
        crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
        return Ok(spool);
    }
    if crate::owned_raw_mp3::recognizes(source)? {
        let info = crate::owned_raw_mp3::source_info(source, options)?;
        let spool = crate::owned_adts_export::spool_decoded(
            rate,
            channels,
            mask,
            options,
            |writer, options| crate::owned_raw_mp3::decode_raw_mpeg_pcm(source, writer, None, options),
        )?;
        crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
        return Ok(spool);
    }
    let mp4 = crate::owned_mp4_audio_export::recognizes(source, options)?;
    let info = if mp4 {
        crate::owned_wave_metadata::mp4_source_info(source, options)?
    } else {
        crate::owned_wave_metadata::matroska_source_info(source, options)?
    };
    let precise = codec.starts_with("A_PCM/") || codec.starts_with("pcm_");
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        if precise { 64 } else { 32 },
        options,
        |writer, options| {
            let file = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
            let result = if mp4 && precise {
                crate::owned_mp4_audio::decode_mp4_pcm_f64(file, writer, None, options)
            } else if mp4 {
                crate::owned_mp4_audio::decode_mp4_audio_pcm(file, writer, None, options)
            } else if codec == "aac" {
                crate::owned_matroska_aac::decode_matroska_aac_pcm(file, writer, None, options)
            } else if codec == "opus" {
                crate::owned_matroska_opus::decode_matroska_opus_pcm(file, writer, None, options)
            } else if codec == "alac" {
                crate::owned_matroska_alac::decode_matroska_alac_pcm(file, writer, None, options)
            } else if codec == "flac" {
                crate::owned_matroska_flac::decode_matroska_flac_pcm(file, writer, None, options)
            } else if matches!(codec, "mp1" | "mp2" | "mp3") {
                crate::owned_matroska_mpeg::decode_matroska_mpeg_pcm(file, writer, None, options)
            } else {
                crate::owned_matroska_pcm::decode_matroska_pcm_f64(file, writer, None, options)
            };
            result.map_err(|e| e.to_string())
        },
    )?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
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

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn matroska_aac_loudness_and_normalization_keep_owned_budget_policy() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/audio/aac-stereo.mka");
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
                "fvid-matroska-loudnorm-budget-{dual}-{}.wav",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            assert!(crate::owned_container_loudnorm::supports(
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
            let bytes = std::fs::read(&output).unwrap();
            std::fs::remove_file(&output).unwrap();
            apply(&CopyOptions::default()).unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), bytes);
            std::fs::remove_file(&output).unwrap();
        }
    }
}
