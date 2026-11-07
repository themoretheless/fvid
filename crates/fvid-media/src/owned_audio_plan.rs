//! Read-only operation plans for the owned WAVE, ADTS and MP4 audio writer.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeTransform, MediaPlan, PlanStep, PlanStream};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

pub(crate) fn matroska_descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<Option<(usize, u32, u16, u32, String, String)>> {
    matroska_descriptor_for_decode(source, None, options)
        .map(|opt| opt.map(|(index, rate, channels, mask, codec, precision, _, _)| (index, rate, channels, mask, codec, precision)))
}

pub(crate) fn matroska_descriptor_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<Option<(usize, u32, u16, u32, String, String, u64, Vec<u8>)>> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 4];
    if file.read_exact(&mut prefix).is_err() || prefix != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    matroska_descriptor_from_file_for_decode(source, file, transform, options).map(Some)
}

pub(crate) fn matroska_descriptor_from_file_for_decode(
    source: &Path,
    file: File,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, String, String, u64, Vec<u8>)> {
    let reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(file),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index = crate::owned_matroska_audio::matroska_audio_index(&reader, selected)
        .map_err(|e| e.to_string())?;
    let source_codec = reader.tracks[index].codec.clone();
    let (rate, channels, mask, codec, precision, frames, info) = match source_codec.as_str() {
        "A_FLAC" => {
            let (rate, channels, mask, frames, info) =
                crate::owned_matroska_flac_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            (
                rate,
                channels,
                mask,
                "flac".into(),
                "float32".into(),
                frames,
                info,
            )
        }
        "A_AAC" => {
            let (rate, channels, mask, frames, info) =
                crate::owned_matroska_aac_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            (rate, channels, mask, "aac".into(), "float32".into(), frames, info)
        }
        "A_OPUS" => {
            let (rate, channels, mask, frames, info) =
                crate::owned_matroska_opus_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            (
                rate,
                channels,
                mask,
                "opus".into(),
                "float32".into(),
                frames,
                info,
            )
        }
        "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3" => {
            let (rate, channels, frames, info) =
                crate::owned_matroska_mp3_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            (
                rate,
                channels,
                crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
                match source_codec.as_str() {
                    "A_MPEG/L1" => "mp1",
                    "A_MPEG/L2" => "mp2",
                    _ => "mp3",
                }
                .into(),
                "float32".into(),
                frames,
                info,
            )
        }
        "A_ALAC" => {
            let (rate, channels, mask, frames, info) =
                crate::owned_matroska_alac_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            (
                rate,
                channels,
                mask,
                "alac".into(),
                "float32".into(),
                frames,
                info,
            )
        }
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" => {
            let (rate, channels, frames, info) =
                crate::owned_matroska_pcm_export::geometry_from_reader_for_decode(
                    source, reader, index, transform, options,
                )?;
            let mask = if channels <= 2 {
                crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32
            } else {
                0
            };
            (
                rate,
                channels,
                mask,
                source_codec.clone(),
                "float64".into(),
                frames,
                info,
            )
        }
        _ => return Err("selected Matroska audio codec is not owned by the export path".into()),
    };
    Ok((index, rate, channels, mask, codec, precision, frames, info))
}
/// Admit only plans accepted by the owned metadata preflight.
/// This does not decode packets, create a destination, or emit progress.
pub(crate) fn supports(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match plan_decode_audio(source, transform, options) {
        Ok(_) => true,
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
#[derive(Clone)]
pub(crate) struct AudioDescriptor {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    pub(crate) mask: u32,
    pub(crate) codec: String,
    pub(crate) precision: String,
    stream_index: usize,
    pub(crate) adts: bool,
    pub(crate) mp4: bool,
    pub(crate) mp4_target: Option<(u32, u16, u32)>,
    pub(crate) is_matroska: bool,
    pub(crate) raw_encoded_audio: bool,
    pub(crate) ogg: bool,
    pub(crate) g711_wave: bool,
    pub(crate) stage2_frames: u64,
    pub(crate) stage2_meta_bytes: usize,
    pub(crate) stage2_info: Option<Vec<u8>>,
}

/// Read-only audio source geometry (`rate`, `channels`) for native loudness and planning.
pub fn source_geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    let desc = source_descriptor(source, options)?;
    Ok((desc.rate, desc.channels))
}

/// Decode work consumed by a standalone G.711/Ogg/FLAC/MPEG packet prefix.
/// Uses the codec's admitted index or block geometry without decoding PCM.
/// Returns None for formats whose container-specific caller owns the count.
pub fn standalone_packet_work(source: &Path, options: &CopyOptions) -> Result<Option<u64>> {
    crate::owned_ogg_opus_export::check(options)?;
    let (head, head_len) = read_source_head(source)?;
    let head_slice = &head[..head_len];
    let count = if head_len == 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" {
        let Some(work) = crate::owned_g711_wave_export::try_packet_work(source, options)? else {
            return Ok(None);
        };
        work
    } else if head_slice.starts_with(b"OggS") {
        use crate::owned_ogg_audio_export::Backend;
        let (_, backend, ogg, index) =
            crate::owned_ogg_audio_export::open_selected_ogg(source, options)?;
        match backend {
            Backend::Opus => u64::try_from(
                crate::owned_ogg_opus_export::open_from_ogg_with_transform(
                    ogg, index, None, options,
                )?
                .0
                .packet_count(),
            )
            .map_err(|_| "Opus packet work overflow")?,
            Backend::Vorbis => u64::try_from(
                crate::owned_ogg_vorbis_export::open_from_ogg_with_transform(
                    ogg, index, None, options,
                )?
                .0
                .packet_times()
                .len(),
            )
            .map_err(|_| "Vorbis packet work overflow")?,
            Backend::Joint => crate::owned_ogg_joint_export::open_from_ogg_with_transform(
                ogg, index, None, options,
            )?
            .0
            .packet_work()
            .map_err(|e| e.to_string())?,
        }
    } else if head_slice.starts_with(b"fLaC") {
        u64::try_from(crate::owned_raw_flac::open(source, options)?.frames.len())
            .map_err(|_| "FLAC packet work overflow")?
    } else if !(head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some())
        && crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)])
    {
        let parsed = crate::owned_raw_mp3::open(source, options)?;
        let pres = parsed.presentation_window();
        let active_frames = parsed.frames.iter().take_while(|f| f.pts < pres.end).count();
        let total = active_frames.saturating_add(usize::from(parsed.info_packet().is_some()));
        u64::try_from(total).map_err(|_| "MPEG packet work overflow")?
    } else { return Ok(None); };
    Ok(Some(options.max_packets.map_or(count, |limit| count.min(limit))))
}

pub fn read_source_head(source: &Path) -> Result<([u8; 12], usize)> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok((head, filled))
}

/// Read-only audio source geometry and speaker mask (`rate`, `channels`, `mask`).
pub fn source_geometry_with_mask(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let desc = source_descriptor(source, options)?;
    Ok((desc.rate, desc.channels, desc.mask))
}

pub(crate) fn source_descriptor(source: &Path, options: &CopyOptions) -> Result<AudioDescriptor> {
    source_descriptor_for_decode(source, options, None)
}
fn source_descriptor_for_decode(source: &Path, options: &CopyOptions,
    transform: Option<&AudioDecodeTransform>) -> Result<AudioDescriptor> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
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
    use std::io::{Seek, SeekFrom};
    let mut head_file = Some(File::open(source).map_err(|e| e.to_string())?);
    let mut head = [0u8; 12];
    let mut head_len = 0;
    while head_len < head.len() {
        match head_file.as_mut().unwrap().read(&mut head[head_len..]) {
            Ok(0) => break,
            Ok(n) => head_len += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let head_slice = &head[..head_len];
    let adts = head_len >= 7 && crate::owned_aac::adts::header(&head[..7]).is_some();
    let raw_flac = !adts && head_slice.starts_with(b"fLaC");
    let raw_encoded_audio =
        raw_flac || (!adts && crate::owned_mpeg_audio::has_signature(&head[..head_len.min(10)]));
    let mp4_desc = if head_len >= 8 && crate::owned_mp4::recognizes_prefix(&head[..8]) {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        crate::owned_mp4_audio_export::try_descriptor_with_frames_and_target_from_file_for_decode(
            source,
            file,
            transform.copied(),
            options,
        )?
    } else {
        None
    };
    let mp4 = mp4_desc.is_some();
    let matroska = if head_slice.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Some(matroska_descriptor_from_file_for_decode(
            source,
            file,
            transform.copied(),
            options,
        )?)
    } else {
        None
    };
    let is_matroska = matroska.is_some();
    let ogg = head_slice.starts_with(b"OggS");
    let g711_desc = if head_len == 12
        && &head[..4] == b"RIFF"
        && &head[8..12] == b"WAVE"
    {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        if let Some(law) =
            crate::owned_wave_inspect::g711_law(&mut file, options.cancel.as_ref())
                .map_err(|e| e.to_string())?
        {
            Some(
                crate::owned_g711_wave_export::descriptor_from_file_and_law_for_decode(
                    file,
                    law,
                    transform.copied(),
                    options,
                )?,
            )
        } else {
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            head_file = Some(file);
            None
        }
    } else {
        None
    };
    let g711_wave = g711_desc.is_some();
    let mut stream_index = 0;
    let mut stage2_frames = 1024;
    let mut stage2_meta_bytes = 0;
    let mut stage2_info = None;
    let mut mp4_target = None;
    let (rate, channels, mask, codec, precision) = if let Some((rate, channels, mask, codec, frames, info_len)) = g711_desc {
        stage2_frames = frames;
        stage2_meta_bytes = info_len;
        (rate, channels, mask, codec.to_owned(), "float32".to_owned())
    } else if ogg {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (index, rate, channels, mask, codec, frames, info_len) =
            crate::owned_ogg_audio_export::descriptor_from_file_for_decode(
                file,
                transform.copied(),
                options,
            )?;
        stream_index = index;
        stage2_frames = frames;
        stage2_meta_bytes = info_len;
        (rate, channels, mask, codec.to_owned(), "float32".to_owned())
    } else if let Some((index, rate, channels, mask, codec, precise, frames, info, target)) = mp4_desc {
        stream_index = index;
        stage2_frames = frames;
        stage2_meta_bytes = info.len();
        stage2_info = Some(info);
        mp4_target = target;
        let precision = if precise || codec.starts_with("pcm_") {
            "float64"
        } else {
            "float32"
        };
        (rate, channels, mask, codec, precision.to_owned())
    } else if let Some((index, rate, channels, mask, codec, precision, frames, info)) = matroska {
        stream_index = index;
        stage2_frames = frames;
        stage2_meta_bytes = info.len();
        stage2_info = Some(info);
        (rate, channels, mask, codec, precision)
    } else if raw_flac {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (rate, channels, mask, frames, info_len) =
            crate::owned_raw_flac::descriptor_from_file_for_decode(
                file,
                transform.copied(),
                options,
            )?;
        if let Some(max) = options.max_controlled_bytes {
            let wave_meta_bytes =
                crate::owned_wave_metadata::wave_metadata_working_bytes(info_len, options)?;
            if wave_meta_bytes > max {
                return Err("controlled memory budget exceeded: WAVE metadata limit".into());
            }
        }
        stage2_frames = frames;
        stage2_meta_bytes = info_len;
        (rate, channels, mask, "flac".to_owned(), "float32".to_owned())
    } else if raw_encoded_audio {
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let (rate, channels, raw_codec, frames, info_len) =
            crate::owned_raw_mp3::descriptor_from_file_for_decode(
                file,
                transform.copied(),
                options,
            )?;
        if let Some(max) = options.max_controlled_bytes {
            let wave_meta_bytes =
                crate::owned_wave_metadata::wave_metadata_working_bytes(info_len, options)?;
            if wave_meta_bytes > max {
                return Err("controlled memory budget exceeded: WAVE metadata limit".into());
            }
        }
        stage2_frames = frames;
        stage2_meta_bytes = info_len;
        (rate, channels, crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
         raw_codec.to_owned(), "float32".to_owned())
    } else if adts {
        if !crate::owned_audio_export::simple_options(options) {
            return Err("ADTS has only stream 0".into());
        }
        let mut file = head_file.take().unwrap();
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let reader = crate::owned_aac::adts::map_controlled_adts_packet_error(
            crate::owned_aac::adts::StreamReader::open_with_packet_limit(
                BufReader::new(file),
                options
                    .max_packet_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
            )
            .map_err(|e| e.to_string()),
            options,
        )?;
        let config = reader.configuration();
        let prefix = transform.and_then(|transform|
            crate::owned_adts_export::decoded_prefix(*transform, config.sample_rate));
        let (rate, channels, mask, frames) =
            crate::owned_aac::stream::adts_prefix_info_from_reader_for_transform(
                reader,
                prefix,
                transform.copied(),
                options,
            )
            .map_err(|e| e.to_string())?;
        stage2_frames = frames;
        (
            rate,
            channels,
            mask,
            "aac".to_owned(),
            "float32".to_owned(),
        )
    } else {
        if !crate::owned_audio_export::simple_options(options) {
            return Err("WAVE has only stream 0".into());
        }
        let mut file = match head_file.take() {
            Some(mut f) => {
                f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
                f
            }
            None => File::open(source).map_err(|e| e.to_string())?,
        };
        let info = crate::owned_wave_inspect::inspect(&mut file, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
        if info.sample_frames == 0 || !(1..=255).contains(&info.channels) {
            return Err("no decoded audio samples or invalid channel count (nonempty input required)".into());
        }
        let frame_bytes = usize::from(info.block);
        if frame_bytes == 0 {
            return Err("PCM packet limit cannot hold one sample frame".into());
        }
        let capacity =
            (4096 * frame_bytes).min(options.max_packet_bytes / frame_bytes * frame_bytes);
        if capacity == 0 {
            return Err("PCM packet limit cannot hold one sample frame".into());
        }
        let full_size = usize::try_from(info.data_bytes)
            .map_err(|_| "WAVE PCM payload exceeds address space")?;
        let limit = options
            .max_packets
            .map(|packets| u128::from(packets) * capacity as u128)
            .unwrap_or(full_size as u128);
        let size = (full_size as u128).min(limit) as usize;
        if size == 0 {
            return Err("packet limit leaves no decoded audio samples".into());
        }
        if let Some(transform) = transform {
            if let Some((from, to)) = transform.interval {
                let frames_read = size / frame_bytes;
                let out_rate =
                    u128::try_from(transform.sample_rate.unwrap_or(info.sample_rate as i32))
                        .map_err(|_| "invalid output sample rate")?;
                let output_frames = usize::try_from(
                    (frames_read as u128)
                        .checked_mul(out_rate)
                        .ok_or("audio interval overflow")?
                        .div_ceil(u128::from(info.sample_rate)),
                )
                .map_err(|_| "audio interval overflow")?;
                let boundary = |time: i64| -> Result<usize> {
                    usize::try_from(
                        (time as u128)
                            .checked_mul(out_rate)
                            .ok_or("audio interval overflow")?
                            .div_ceil(1_000_000),
                    )
                    .map(|n| n.min(output_frames))
                    .map_err(|_| "audio interval overflow".into())
                };
                let (first, last) = (boundary(from)?, boundary(to)?);
                if last <= first {
                    return Err("no decoded audio samples".into());
                }
            }
        }
        if let Some(max) = options.max_controlled_bytes {
            let metadata = crate::owned_wave_inspect::info_chunks_bytes(
                &mut file,
                &info,
                options.cancel.as_ref(),
            )
            .map_err(|e| e.to_string())?;
            let estimated = crate::owned_budget::estimate_float_wave_export_bytes(
                &info,
                size,
                metadata,
                transform.copied().unwrap_or_default(),
                options,
            )?;
            if estimated > max {
                return Err(format!(
                    "controlled memory budget exceeded: need {estimated} bytes, limit {max}"
                ));
            }
        }
        (
            info.sample_rate,
            info.channels,
            info.channel_mask,
            info.codec(),
            if info.float {
                format!("float{}", info.bits_per_sample)
            } else {
                format!(
                    "integer{}",
                    if info.bits_per_sample == 24 {
                        32
                    } else {
                        info.bits_per_sample
                    }
                )
            },
        )
    };
    Ok(AudioDescriptor {
        rate, channels, mask, codec, precision, stream_index,
        adts, mp4, mp4_target, is_matroska, raw_encoded_audio, ogg, g711_wave,
        stage2_frames, stage2_meta_bytes, stage2_info,
    })
}

pub fn plan_decode_audio(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    Ok(plan_decode_audio_with_descriptor(source, transform, options)?.0)
}

/// Read-only operation plan and `(rate, channels, mask)` geometry for native audio mixing/concat.
pub fn plan_decode_audio_with_geometry(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<(MediaPlan, (u32, u16, u32))> {
    let (plan, desc) = plan_decode_audio_with_descriptor(source, transform, options)?;
    Ok((plan, (desc.rate, desc.channels, desc.mask)))
}

pub(crate) fn plan_decode_audio_with_descriptor(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<(MediaPlan, AudioDescriptor)> {
    crate::owned_audio_export::validate_request(*transform, options)?;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    crate::owned_adts_export::check_wave_metadata_budget(options)?;
    let descriptor = source_descriptor_for_decode(source, options, Some(transform))?;
    let AudioDescriptor {
        rate, channels, mask, codec, precision, stream_index,
        adts, mp4, mp4_target, is_matroska, raw_encoded_audio, ogg, g711_wave,
        stage2_frames, stage2_meta_bytes, stage2_info: _,
    } = descriptor.clone();
    let output_channels = transform.channels.unwrap_or(i32::from(channels));
    let aac_fold=mp4_target.is_none()&&codec=="aac"&&output_channels!=i32::from(channels)&&matches!(output_channels,1|2)&&crate::owned_aac_export_routing::router_for_decode(source,rate,channels,Some(*transform),options)?.is_some();
    if mp4_target.is_none() && output_channels != i32::from(channels) && !aac_fold {
        if !((channels <= 8 && matches!(output_channels, 1 | 2))
            || (channels <= 2 && (1..=8).contains(&output_channels)))
        {
            return Err("unsupported channel conversion".into());
        }
        if !(channels <= 2 && mask == 0)
            && !(mask != 0 && crate::owned_pcm_channels::rematrix_layout_supported(channels, mask))
        {
            return Err("unsupported speaker layout".into());
        }
    }
    if adts || mp4 || is_matroska || raw_encoded_audio || ogg || g711_wave {
        let (stage2_rate, stage2_channels, _) = mp4_target.unwrap_or((rate, channels, mask));
        let stage2_bits = if precision == "float64" { 64 } else { 32 };
        let stage2_frames = if stage2_rate != rate && rate > 0 {
            (u128::from(stage2_frames) * u128::from(stage2_rate)).div_ceil(u128::from(rate)) as u64
        } else {
            stage2_frames
        }
        .max(1024);
        crate::owned_adts_export::check_spool_wave_export_budget(
            stage2_rate,
            stage2_channels,
            stage2_bits,
            stage2_frames,
            stage2_meta_bytes,
            aac_fold,
            *transform,
            options,
        )?;
    }
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: if mp4 {
            format!("owned MP4 {codec} decoder and presentation scheduler; preserve silence, repeated edits and AAC preroll; private {precision} WAVE disk spool")
        } else if is_matroska {
            format!("owned Matroska {codec} decoder and presentation scheduler; preserve CodecDelay, signed padding and gaps; private {precision} WAVE disk spool")
        } else if ogg {
            format!("owned Ogg {codec} decoder and presentation scheduler; preserve chained links, pre-skip and rational rate changes; private {precision} WAVE disk spool")
        } else if g711_wave {
            format!("owned WAVE {codec} decoder; preserve speaker mask {mask:#x}; private {precision} WAVE disk spool")
        } else if raw_encoded_audio {
            format!("owned raw {} decoder; validated sample timeline and encoded packet limits; private float32 WAVE disk spool", codec.to_uppercase())
        } else if adts {
            "owned AAC-LC/HE-AAC decoder; negotiate implicit SBR output clock; retain ADTS priming and preroll; private float32 WAVE disk spool"
                .into()
        } else {
            format!("owned WAVE reader; retain {precision} precision and speaker mask {mask:#x}")
        },
    }];
    if output_channels != i32::from(channels) {
        steps.push(PlanStep {
            action: "rematrix".into(),
            detail: format!("owned channel conversion {channels} → {output_channels}"),
        });
    }
    if let Some(gain) = transform.volume.filter(|&g| g != 1.0) {
        steps.push(PlanStep {
            action: "volume".into(),
            detail: format!("owned linear gain {gain} after rematrix and before resampling"),
        });
    }
    if let Some(output_rate) = transform.sample_rate.filter(|&r| r != rate as i32) {
        steps.push(PlanStep {
            action: "resample".into(),
            detail: format!("owned windowed-sinc resampler {rate} → {output_rate} Hz"),
        });
    }
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep{action:"trim".into(),detail:format!("output sample window [{from},{to}) µs; ceil boundaries after resampling; compressed prefix includes required preroll and resampler lookahead")});
    }
    if !options.metadata_set.is_empty() || !options.metadata_delete.is_empty() {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: "apply owned RIFF INFO container tag edits to final WAVE".into(),
        });
    }
    steps.push(PlanStep{action:"write".into(),detail:format!("owned .wav writer; preserve {precision} precision; publish without overwriting; cleanup on failure")});
    let mut notes = vec![
        "backend: owned fvid-media; no external demuxer, codec, resampler or muxer".into(),
        if adts {
            "read-only ADTS prefix negotiation checks the output clock and decoder admission without emitting progress; packet payloads and publication are revalidated during execution".into()
        } else {
            "read-only metadata preflight: packet tools, payload validity, DSP allocation admission and publication are checked during execution".into()
        },
        if adts || mp4 || is_matroska || raw_encoded_audio || ogg || g711_wave {
            "packet byte/count limits apply to selected encoded audio, including preroll; internal PCM blocks are excluded".into()
        } else {
            "packet limits apply to frame-aligned WAVE I/O blocks of at most 4096 sample frames".into()
        },
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {} packets; controlled memory limit: {} bytes; RSS limit: {} bytes",
            options.max_packet_bytes,
            options.max_packets.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options.max_controlled_bytes.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
            options.max_rss_bytes.map_or_else(|| "unbounded".to_owned(), |n| n.to_string()),
        ));
    }
    Ok((MediaPlan{command:"decode-audio".into(),input:source.into(),inputs:vec![source.into()],streams:vec![PlanStream{index:stream_index,media_type:"audio".into(),codec,disposition:"decode".into()}],steps,graph:None,notes}, descriptor))
}
