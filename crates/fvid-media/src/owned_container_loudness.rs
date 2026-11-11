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
    let (file, head, head_len) = crate::owned_audio_plan::open_source_head(source)?;
    recognizes_standalone_from_file_and_head(file, &head, head_len, options)
}

pub fn recognizes_standalone_from_head(
    source: &Path,
    head: &[u8; 12],
    head_len: usize,
    options: &CopyOptions,
) -> Result<bool> {
    if head_len < 4 {
        return Ok(false);
    }
    if &head[..4] == b"OggS" || &head[..4] == b"fLaC" {
        return Ok(true);
    }
    if matches!(&head[..4], b"RIFF" | b"RIFX" | b"RF64") {
        return crate::owned_g711_wave_export::recognizes(source, options);
    }
    if head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some() {
        return Ok(false);
    }
    Ok(crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)]))
}

pub fn recognizes_standalone_from_file_and_head(
    mut file: File,
    head: &[u8; 12],
    head_len: usize,
    options: &CopyOptions,
) -> Result<bool> {
    if head_len < 4 {
        return Ok(false);
    }
    if &head[..4] == b"OggS" || &head[..4] == b"fLaC" {
        return Ok(true);
    }
    if matches!(&head[..4], b"RIFF" | b"RIFX" | b"RF64") {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        return crate::owned_g711_wave_export::recognizes_from_file(file, options);
    }
    if head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some() {
        return Ok(false);
    }
    Ok(crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)]))
}

pub(crate) fn recognizes_from_head(
    source: &Path,
    head: &[u8; 12],
    head_len: usize,
    options: &CopyOptions,
) -> Result<bool> {
    if head_len < 4 {
        return Ok(false);
    }
    if head[..4] == [0x1a, 0x45, 0xdf, 0xa3]
        || (head_len >= 8 && crate::owned_mp4::recognizes_prefix(&head[..8]))
    {
        return Ok(true);
    }
    recognizes_standalone_from_head(source, head, head_len, options)
}


pub fn geometry_with_frames_unqualified(
    source: &Path,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, String, u64, Option<Vec<u8>>)> {
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
    use std::io::{Seek, SeekFrom};
    let mut head_file = File::open(source).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    let mut head_len = 0;
    while head_len < head.len() {
        match head_file.read(&mut head[head_len..]) {
            Ok(0) => break,
            Ok(n) => head_len += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    if head_len < 4 {
        return Err("source is not an owned audio container".into());
    }
    let head_slice = &head[..head_len];
    let (_, rate, channels, mask, codec, frames, info) =
        if head_len >= 8 && crate::owned_mp4::recognizes_prefix(&head[..8]) {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc =
                crate::owned_mp4_audio_export::try_source_descriptor_from_file_for_decode(
                    source, head_file, None, options,
                )?
                .ok_or("source is not a supported Matroska audio container")?;
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                desc.channel_mask,
                desc.codec,
                desc.estimated_frames,
                desc.info_chunk,
            )
        } else if matches!(&head[..4], b"RIFF" | b"RIFX" | b"RF64") {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc = crate::owned_g711_wave_export::try_descriptor_from_file_for_decode(
                head_file, None, options,
            )?
            .ok_or("source is not an owned audio container")?;
            if desc.estimated_frames == 0 {
                return Err("loudness requires at least one audio frame".into());
            }
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                desc.channel_mask,
                desc.codec,
                desc.estimated_frames,
                None,
            )
        } else if head_slice.starts_with(b"OggS") {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc = crate::owned_ogg_audio_export::descriptor_from_file_for_decode(
                head_file, None, options,
            )?;
            let effective_mask = if desc.channel_mask == 0 && desc.channels <= 2 {
                crate::owned_pcm_channels::standard_mask(desc.channels).unwrap_or(0) as u32
            } else {
                desc.channel_mask
            };
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                effective_mask,
                desc.codec,
                desc.estimated_frames,
                None,
            )
        } else if head_slice.starts_with(b"fLaC") {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc =
                crate::owned_raw_flac::descriptor_from_file_for_decode(head_file, None, options)?;
            let effective_mask = if desc.channel_mask == 0 && desc.channels <= 2 {
                crate::owned_pcm_channels::standard_mask(desc.channels).unwrap_or(0) as u32
            } else {
                desc.channel_mask
            };
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                effective_mask,
                desc.codec,
                desc.estimated_frames,
                None,
            )
        } else if !(head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some())
            && crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)])
        {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc =
                crate::owned_raw_mp3::descriptor_from_file_for_decode(head_file, None, options)?;
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                desc.channel_mask,
                desc.codec,
                desc.estimated_frames,
                None,
            )
        } else if head[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            head_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| e.to_string())?;
            let desc = crate::owned_audio_plan::matroska_descriptor_from_file_for_decode(
                source, head_file, None, options,
            )?;
            (
                desc.stream_index,
                desc.sample_rate,
                desc.channels,
                desc.channel_mask,
                desc.codec,
                desc.estimated_frames,
                desc.info_chunk,
            )
        } else {
            return Err("source is not an owned audio container".into());
        };
    Ok((rate, channels, mask, codec, frames, info))
}
pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32, String)> {
    let (rate, channels, mask, codec, _, _) = geometry_with_frames(source, options)?;
    Ok((rate, channels, mask, codec))
}
fn geometry_with_frames(
    source: &Path,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, String, u64, Option<Vec<u8>>)> {
    let (rate, channels, mask, codec, frames, info) =
        geometry_with_frames_unqualified(source, options)?;
    if mask == 0 || mask & !0x7ff != 0 || mask.count_ones() != u32::from(channels) {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(rate) {
        return Err("owned true-peak measurement requires a qualified standard PCM rate".into());
    }
    Ok((rate, channels, mask, codec, frames, info))
}
pub(crate) fn supports_plan(source: &Path, options: &CopyOptions) -> bool {
    match geometry(source, options) {
        Ok(_) => true,
        // Resource admission failures belong to the owned execution route.
        // Returning false would hide the refusal as an unsupported format.
        Err(error) => error.starts_with("controlled memory budget exceeded:")
            || matches!(error.as_str(), "MP4 packet exceeds budget" | "WebM packet exceeds budget"),
    }
}
#[cfg(test)]
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
    let (rate, channels, mask, codec, _, info) = geometry_with_frames(source, options)?;
    decode_to_wave_from_geometry_with_info(source, rate, channels, mask, &codec, info, options)
}
pub fn decode_to_wave_unqualified(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    let (rate, channels, mask, codec, _, info) = geometry_with_frames_unqualified(source, options)?;
    decode_to_wave_from_geometry_with_info(source, rate, channels, mask, &codec, info, options)
}
pub fn decode_adts_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    crate::owned_adts_export::decode_to_wave(source, None, options)
}
pub fn decode_to_wave_from_geometry_with_info(
    source: &Path,
    rate: u32,
    channels: u16,
    mask: u32,
    codec: &str,
    precomputed_info: Option<Vec<u8>>,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    use std::io::{Seek, SeekFrom};
    let (mut head_file, head, head_len) = crate::owned_audio_plan::open_source_head(source)?;
    let head_slice = &head[..head_len];
    if head_len == 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        head_file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        if let Some(spool) =
            crate::owned_g711_wave_export::try_decode_to_wave_from_file(head_file, options)?
        {
            return Ok(spool);
        }
        return Err("source is not an owned audio container".into());
    }
    if head_slice.starts_with(b"OggS") {
        head_file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let (_, backend, ogg, index) =
            crate::owned_ogg_audio_export::open_selected_ogg_from_file(head_file, options)?;
        return match backend {
            crate::owned_ogg_audio_export::Backend::Opus => {
                crate::owned_ogg_opus_export::decode_to_wave_from_ogg(ogg, index, options)
            }
            crate::owned_ogg_audio_export::Backend::Vorbis => {
                crate::owned_ogg_vorbis_export::decode_to_wave_from_ogg(ogg, index, options)
            }
            crate::owned_ogg_audio_export::Backend::Joint => {
                crate::owned_ogg_joint_export::decode_to_wave_from_ogg(ogg, index, options)
            }
        };
    }
    if head_slice.starts_with(b"fLaC") {
        head_file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        return crate::owned_raw_flac::decode_to_wave_from_file(
            head_file, rate, channels, mask, options,
        );
    }
    if !(head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some())
        && crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)])
    {
        head_file
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        return crate::owned_raw_mp3::decode_to_wave_from_file(
            head_file, rate, channels, mask, options,
        );
    }
    let mp4 = head_len >= 8 && crate::owned_mp4::recognizes_prefix(&head[..8]);
    let info = match precomputed_info {
        Some(info) => info,
        None if mp4 => crate::owned_wave_metadata::mp4_source_info(source, options)?,
        None => crate::owned_wave_metadata::matroska_source_info(source, options)?,
    };
    head_file
        .seek(SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    let precise = codec.starts_with("A_PCM/") || codec.starts_with("pcm_");
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        mask,
        if precise { 64 } else { 32 },
        options,
        move |writer, options| {
            let file = BufReader::new(head_file);
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
pub(crate) fn check_meter_budget(
    rate: u32,
    channels: u16,
    codec: &str,
    sample_frames: u64,
    options: &CopyOptions,
) -> Result<()> {
    if let Some(limit) = options.max_controlled_bytes {
        let precise = codec.starts_with("A_PCM/") || codec.starts_with("pcm_");
        let ch = u128::from(channels.max(1));
        let rate_u64 = u64::from(rate);
        let rate_u128 = u128::from(rate);
        let frames = sample_frames.max(1024);
        let hop = (rate_u64 / 10).max(1);
        let integrated_blocks = if frames < rate_u64 * 2 / 5 {
            0
        } else {
            1 + (frames - rate_u64 * 2 / 5) / hop
        };
        let short_blocks = if frames < rate_u64 * 3 {
            0
        } else {
            1 + (frames - rate_u64 * 3) / hop
        };
        let frame = ch * if precise { 8 } else { 4 };
        let capacity = 4096 * frame;
        let estimated = (rate_u128 * 3 + rate_u128 * 2 / 5) * 8
            + capacity
            + 4096 * ch * 8
            + ch * 512
            + u128::from(integrated_blocks + short_blocks) * 512
            + 16384;
        if estimated > limit as u128 {
            return Err(format!(
                "controlled memory budget exceeded: need {estimated} bytes"
            ));
        }
    }
    Ok(())
}
pub fn measure_loudness(source: &Path, options: &CopyOptions) -> Result<LoudnessStats> {
    let (rate, channels, mask, codec, sample_frames, info) = geometry_with_frames(source, options)?;
    check_meter_budget(rate, channels, &codec, sample_frames, options)?;
    let spool =
        decode_to_wave_from_geometry_with_info(source, rate, channels, mask, &codec, info, options)?;
    let mut pcm_options = options.clone();
    pcm_options.streams = vec![0];
    pcm_options.max_packets = None;
    pcm_options.max_packet_bytes = CopyOptions::default().max_packet_bytes;
    pcm_options.progress = None;
    let mut stats = crate::owned_wave_loudness::measure_wave_loudness(&spool.wave, &pcm_options)?;
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
    let (mut plan, desc) =
        crate::owned_audio_plan::plan_decode_audio_with_descriptor(source, &Default::default(), options)?;
    if desc.adts || (!desc.mp4 && !desc.is_matroska && !desc.raw_encoded_audio && !desc.ogg && !desc.g711_wave) {
        return Err("source is not an owned audio container".into());
    }
    if desc.g711_wave && desc.stage2_frames == 0 {
        return Err("loudness requires at least one audio frame".into());
    }
    let effective_mask = if desc.mask == 0 && desc.channels <= 2 {
        crate::owned_pcm_channels::standard_mask(desc.channels).unwrap_or(0) as u32
    } else {
        desc.mask
    };
    if effective_mask == 0
        || effective_mask & !0x7ff != 0
        || effective_mask.count_ones() != u32::from(desc.channels)
    {
        return Err("owned loudness speaker positions are not yet qualified".into());
    }
    if !crate::owned_wave_loudness::qualified_rate(desc.rate) {
        return Err("owned true-peak measurement requires a qualified standard PCM rate".into());
    }
    check_meter_budget(
        desc.rate,
        desc.channels,
        &desc.codec,
        desc.stage2_frames,
        options,
    )?;
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
