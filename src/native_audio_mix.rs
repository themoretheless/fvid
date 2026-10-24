//! Owned AAC/WAVE decoding and blockwise weighted float PCM mixing.
mod concat;
pub use concat::{
    concat_audio, concat_audio_with_options, eligible as concat_eligible,
    eligible_with_options as concat_eligible_with_options,
};
pub use crate::media_info::{MixAudioOptions, MixAudioStats, MixDuration};
use crate::{Result, invalid};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn strip_output_controls(options: &fvid_media::CopyOptions) -> fvid_media::CopyOptions {
    let mut clean = options.clone();
    clean.streams.clear();
    clean.metadata_set.clear();
    clean.metadata_delete.clear();
    clean.stream_metadata_set.clear();
    clean.stream_metadata_delete.clear();
    clean
}

pub(crate) fn validate_output_controls(
    options: &fvid_media::CopyOptions,
    matroska: Option<bool>,
) -> Result<()> {
    if !(options.streams.is_empty() || options.streams == [0]) {
        return Err(invalid("audio output has only stream 0"));
    }
    if options.metadata_set.len() + options.metadata_delete.len() > 64 {
        return Err(invalid("at most 64 container metadata mutations"));
    }
    if options.stream_metadata_set.len() + options.stream_metadata_delete.len() > 64 {
        return Err(invalid("at most 64 stream metadata mutations"));
    }
    for key in &options.metadata_delete {
        if key.is_empty() || key.len() > 128 || key.contains('\0') {
            return Err(invalid("invalid container metadata key"));
        }
    }
    for (key, value) in &options.metadata_set {
        if key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
        {
            return Err(invalid("invalid container metadata"));
        }
    }
    for (index, key) in &options.stream_metadata_delete {
        if *index != 0
            || key.is_empty()
            || key.len() > 128
            || key.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
        {
            return Err(invalid("invalid stream metadata deletion"));
        }
    }
    for (index, key, value) in &options.stream_metadata_set {
        if *index != 0
            || key.is_empty()
            || key.len() > 128
            || value.len() > 1024
            || key.contains('\0')
            || value.contains('\0')
            || matches!(
                key.to_ascii_lowercase().as_str(),
                "rotate" | "stereo_mode" | "alpha_mode"
            )
            || (key.eq_ignore_ascii_case("language") && value.len() > 128)
        {
            return Err(invalid("invalid stream metadata"));
        }
    }
    if matroska == Some(false) {
        if !options.stream_metadata_set.is_empty() || !options.stream_metadata_delete.is_empty() {
            return Err(invalid("WAVE output does not support stream metadata mutations"));
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )
        .map_err(|e| invalid(&e))?;
    }
    Ok(())
}

fn is_single_stream_fvid_media_audio_with_options(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    let single_audio = match crate::media::probe(source) {
        Ok(info) => info.streams.iter().filter(|s| s.media_type == "audio").count() == 1,
        Err(_) => fvid_media::owned_container_loudness::recognizes_standalone_audio(source)
            .map_err(|e| invalid(&e))?,
    };
    if !single_audio {
        return Ok(false);
    }
    let probe_opts = strip_output_controls(options);
    if fvid_media::plan_decode_audio(source, &Default::default(), &probe_opts).is_err() {
        return Ok(false);
    }
    Ok(fvid_media::owned_audio_plan::source_geometry_with_mask(source, &probe_opts)
        .is_ok_and(|(_, ch, _)| (1..=64).contains(&ch)))
}

fn validate_and_inspect_source_with_options(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<(u32, u16, u32)> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    validate_output_controls(options, None)?;
    if options.max_controlled_bytes.is_none() && crate::native_pcm::is_wave(source)? {
        crate::native_plan::decode_audio(source, &Default::default()).map_err(|e| invalid(&e))?;
        let mut input = File::open(source)?;
        let info = crate::native_pcm::inspect(&mut input, options.cancel.as_ref())?;
        let frame = usize::from(info.frame_bytes());
        if frame == 0 || (65536usize.min(options.max_packet_bytes) / frame * frame) == 0 {
            return Err(invalid(
                "packet byte limit is smaller than one PCM sample frame",
            ));
        }
        Ok((info.sample_rate, info.channels, info.channel_mask))
    } else if options.max_controlled_bytes.is_none() && crate::native_media::is_owned_audio_source(source)? {
        crate::native_plan::decode_audio(source, &Default::default()).map_err(|e| invalid(&e))?;
        let info = crate::native_media::audio_source_info_selected(source, None)?;
        if options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes {
            use std::io::{Seek, SeekFrom};
            let mut input = BufReader::new(File::open(source)?);
            let mut prefix = [0u8; 8];
            input.read_exact(&mut prefix)?;
            input.seek(SeekFrom::Start(0))?;
            if crate::container::mp4::recognizes_prefix(&prefix) {
                let mut limits = crate::container::mp4::Limits::default();
                limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes);
                let reader = crate::container::mp4::Mp4Reader::open(input, limits)?;
                if let Some(track) = reader.tracks().get(info.stream_index) {
                    let exceeds = match &track.samples {
                        crate::container::mp4::SampleIndex::Expanded(samples) => samples
                            .iter()
                            .take(
                                options
                                    .max_packets
                                    .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
                                    .unwrap_or(usize::MAX),
                            )
                            .any(|s| (s.size as usize) > options.max_packet_bytes),
                        crate::container::mp4::SampleIndex::Uniform(uniform) => {
                            uniform.count > 0
                                && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                        }
                    };
                    if exceeds {
                        return Err(invalid("MP4 packet exceeds budget"));
                    }
                }
            } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                let mut reader = crate::container::webm::WebmReader::open(
                    input,
                    crate::container::webm::Limits::default(),
                )?;
                reader.scan_all()?;
                if let Some(track) = reader.tracks.get(info.stream_index) {
                    let track_number = track.number;
                    let mut track_packets = 0u64;
                    for (packet_index, packet) in reader.packets.iter().enumerate() {
                        if packet.track == track_number {
                            if options.max_packets.is_some_and(|max| track_packets >= max) {
                                break;
                            }
                            track_packets += 1;
                            if reader.packet_payload_size(packet_index)? > options.max_packet_bytes
                            {
                                return Err(invalid("WebM packet exceeds budget"));
                            }
                        }
                    }
                }
            } else if crate::container::adts::header(&prefix).is_some() {
                let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
                    input,
                    options.max_packet_bytes,
                )?;
                let mut packets = 0u64;
                while options.max_packets.is_none_or(|max| packets < max)
                    && reader.next_packet()?.is_some()
                {
                    packets += 1;
                }
            }
        }
        let mask = info
            .channel_mask
            .unwrap_or(crate::native_export::default_pcm_mask(info.channels)?);
        Ok((info.sample_rate, info.channels, mask))
    } else {
        let mut default_probe = fvid_media::CopyOptions::default();
        default_probe.cancel = options.cancel.clone();
        if !is_single_stream_fvid_media_audio_with_options(source, &default_probe)? {
            return Err(invalid("source is not a single-stream owned audio input"));
        }
        let probe_opts = strip_output_controls(options);
        fvid_media::plan_decode_audio(source, &Default::default(), &probe_opts)
            .map_err(|e| invalid(&e))?;
        fvid_media::owned_audio_plan::source_geometry_with_mask(source, &probe_opts)
            .map_err(|e| invalid(&e))
    }
}

pub(crate) fn decode_source_to_f32le_with_options(
    source: &Path,
    destination: &Path,
    scratch_dir: &Path,
    index: usize,
    options: &fvid_media::CopyOptions,
) -> Result<crate::native_media::AudioDecodeStats> {
    let cancel = options.cancel.as_ref();
    let clean_opts = strip_output_controls(options);
    if options.max_controlled_bytes.is_none()
        && (crate::native_pcm::is_wave(source)? || crate::native_media::is_owned_audio_source(source)?)
    {
        let mut export_opts = clean_opts;
        export_opts.progress = None;
        crate::native_export::export_audio_pcm_transformed_with_controls(
            source,
            destination,
            None,
            1.0,
            None,
            None,
            &export_opts,
        )
    } else {
        let temp_wav = scratch_dir.join(format!("decoded-{index}.wav"));
        let mut decode_opts = clean_opts;
        decode_opts.progress = None;
        let decoded =
            fvid_media::decode_audio(source, &temp_wav, &decode_opts).map_err(|e| invalid(&e))?;
        let mut stats = crate::native_export::export_audio_pcm_selected(
            &temp_wav, destination, None, 1.0, None, None, None, cancel, None,
        )?;
        let _ = std::fs::remove_file(&temp_wav);
        stats.decoded_frames = decoded.decoded_frames;
        Ok(stats)
    }
}

#[allow(dead_code)]
pub(crate) fn decode_source_to_f32le(
    source: &Path,
    destination: &Path,
    scratch_dir: &Path,
    index: usize,
    cancel: Option<&crate::media_control::CancelFlag>,
) -> Result<crate::native_media::AudioDecodeStats> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    decode_source_to_f32le_with_options(source, destination, scratch_dir, index, &options)
}

pub fn eligible_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    if options.max_packet_bytes == 0 || options.max_packets == Some(0) {
        return Ok(false);
    }
    if fvid_media::owned_budget::check_rss_budget(options).is_err() {
        return Ok(false);
    }
    if validate_output_controls(options, None).is_err() {
        return Ok(false);
    }
    let clean_opts = strip_output_controls(options);
    for source in sources {
        if crate::native_pcm::is_wave(source)? {
            let mut input = File::open(source)?;
            let Ok(info) = crate::native_pcm::inspect(&mut input, options.cancel.as_ref()) else {
                return Ok(false);
            };
            if info.validate_decode().is_err() {
                return Ok(false);
            }
            let frame = usize::from(info.frame_bytes());
            if frame == 0 || (65536usize.min(options.max_packet_bytes) / frame * frame) == 0 {
                return Ok(false);
            }
            let probe_opts = clean_opts.clone();
            if options.max_controlled_bytes.is_some()
                && fvid_media::plan_decode_audio(source, &Default::default(), &probe_opts).is_err()
            {
                return Ok(false);
            }
        } else if crate::native_media::is_owned_audio_source(source)? {
            let Ok(info) = crate::native_media::audio_source_info_selected(source, None) else {
                return Ok(false);
            };
            if crate::media::probe(source)
                .is_ok_and(|probe| probe.streams.iter().filter(|s| s.media_type == "audio").count() != 1)
            {
                return Ok(false);
            }
            if options.max_controlled_bytes.is_none()
                && options.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
            {
                use std::io::{Seek, SeekFrom};
                let mut input = BufReader::new(File::open(source)?);
                let mut prefix = [0u8; 8];
                if input.read_exact(&mut prefix).is_err() || input.seek(SeekFrom::Start(0)).is_err()
                {
                    return Ok(false);
                }
                if crate::container::mp4::recognizes_prefix(&prefix) {
                    let mut limits = crate::container::mp4::Limits::default();
                    limits.packet_bytes = limits.packet_bytes.min(options.max_packet_bytes);
                    let Ok(reader) = crate::container::mp4::Mp4Reader::open(input, limits) else {
                        return Ok(false);
                    };
                    if let Some(track) = reader.tracks().get(info.stream_index) {
                        let exceeds = match &track.samples {
                            crate::container::mp4::SampleIndex::Expanded(samples) => samples
                                .iter()
                                .take(
                                    options
                                        .max_packets
                                        .map(|m| usize::try_from(m).unwrap_or(usize::MAX))
                                        .unwrap_or(usize::MAX),
                                )
                                .any(|s| (s.size as usize) > options.max_packet_bytes),
                            crate::container::mp4::SampleIndex::Uniform(uniform) => {
                                uniform.count > 0
                                    && (uniform.bytes_per_frame as usize) > options.max_packet_bytes
                            }
                        };
                        if exceeds {
                            return Ok(false);
                        }
                    }
                } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                    let Ok(mut reader) = crate::container::webm::WebmReader::open(
                        input,
                        crate::container::webm::Limits::default(),
                    ) else {
                        return Ok(false);
                    };
                    if reader.scan_all().is_err() {
                        return Ok(false);
                    }
                    if let Some(track) = reader.tracks.get(info.stream_index) {
                        let track_number = track.number;
                        let mut track_packets = 0u64;
                        for (packet_index, packet) in reader.packets.iter().enumerate() {
                            if packet.track == track_number {
                                if options.max_packets.is_some_and(|max| track_packets >= max) {
                                    break;
                                }
                                track_packets += 1;
                                let Ok(size) = reader.packet_payload_size(packet_index) else {
                                    return Ok(false);
                                };
                                if size > options.max_packet_bytes {
                                    return Ok(false);
                                }
                            }
                        }
                    }
                } else if crate::container::adts::header(&prefix).is_some() {
                    let Ok(mut reader) =
                        crate::container::adts::StreamReader::open_with_packet_limit(
                            input,
                            options.max_packet_bytes,
                        )
                    else {
                        return Ok(false);
                    };
                    let mut packets = 0u64;
                    while options.max_packets.is_none_or(|max| packets < max) {
                        match reader.next_packet() {
                            Ok(Some(_)) => packets += 1,
                            Ok(None) => break,
                            Err(_) => return Ok(false),
                        }
                    }
                }
            }
            let probe_opts = clean_opts.clone();
            if options.max_controlled_bytes.is_some()
                && fvid_media::plan_decode_audio(source, &Default::default(), &probe_opts).is_err()
            {
                return Ok(false);
            }
        } else if !is_single_stream_fvid_media_audio_with_options(source, &clean_opts)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn eligible(sources: &[PathBuf]) -> Result<bool> {
    eligible_with_options(sources, &Default::default())
}

fn weights(inputs: usize, options: &MixAudioOptions) -> Result<Vec<f32>> {
    if !(2..=16).contains(&inputs) {
        return Err(invalid("mix-audio requires 2..=16 inputs"));
    }
    if options.weights.len() > inputs || options.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
    {
        return Err(invalid(
            "mix-audio requires finite nonnegative weights, at most one per input",
        ));
    }
    let mut weights = options.weights.clone();
    weights.resize(inputs, *weights.last().unwrap_or(&1.0));
    let sum: f32 = weights.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(invalid("mix-audio weight sum must be finite and > 0"));
    }
    Ok(weights)
}

/// Plan and validate all source geometries without decoding packet contents.
pub fn plan(
    sources: &[PathBuf],
    options: &MixAudioOptions,
) -> Result<crate::media_info::MediaPlan> {
    plan_with_controls(sources, options, &Default::default())
}

pub fn plan_with_controls(
    sources: &[PathBuf],
    options: &MixAudioOptions,
    controls: &fvid_media::CopyOptions,
) -> Result<crate::media_info::MediaPlan> {
    weights(sources.len(), options)?;
    let mut geometry = None;
    for source in sources {
        let (rate, channels, _) = validate_and_inspect_source_with_options(source, controls)?;
        let shape = (rate, channels);
        if geometry.is_some_and(|g| g != shape) {
            return Err(invalid(
                "mix-audio inputs must share sample rate and channel count",
            ));
        }
        geometry = Some(shape);
    }
    let mut notes = vec!["PCM spools bound mixing memory independently of input duration; temporary disk space is required".into()];
    if controls.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || controls.max_packets.is_some()
        || controls.max_controlled_bytes.is_some()
        || controls.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; memory limit: {:?}; RSS limit: {:?}",
            controls.max_packet_bytes,
            controls.max_packets,
            controls.max_controlled_bytes,
            controls.max_rss_bytes
        ));
    }
    let mut steps = vec![
        crate::media_info::PlanStep {
            action: "decode".into(),
            detail: "FVid AAC/WAVE decoders to temporary float PCM spools".into(),
        },
        crate::media_info::PlanStep {
            action: "mix".into(),
            detail: format!(
                "FVid weighted float mixing; shortest duration; normalize={}",
                options.normalize
            ),
        },
    ];
    if !controls.metadata_set.is_empty()
        || !controls.metadata_delete.is_empty()
        || !controls.stream_metadata_set.is_empty()
        || !controls.stream_metadata_delete.is_empty()
    {
        steps.push(crate::media_info::PlanStep {
            action: "metadata".into(),
            detail: format!(
                "container set={}, delete={}; stream set={}, delete={}",
                controls.metadata_set.len(),
                controls.metadata_delete.len(),
                controls.stream_metadata_set.len(),
                controls.stream_metadata_delete.len()
            ),
        });
    }
    steps.push(crate::media_info::PlanStep {
        action: "write".into(),
        detail: "atomic float32 WAVE or Matroska publication without overwriting an existing destination"
            .into(),
    });
    Ok(crate::media_info::MediaPlan {
        command: "mix-audio".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams: Vec::new(),
        graph: None,
        steps,
        notes,
    })
}

pub fn mix_audio(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
) -> Result<MixAudioStats> {
    mix_audio_with_controls(sources, destination, options, &Default::default())
}

pub fn mix_audio_with_controls(
    sources: &[PathBuf],
    destination: &Path,
    options: &MixAudioOptions,
    controls: &fvid_media::CopyOptions,
) -> Result<MixAudioStats> {
    let weights = weights(sources.len(), options)?;
    let matroska = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => false,
        Some("mka" | "mkv") => true,
        _ => return Err(invalid("mix-audio requires .wav, .mka or .mkv output")),
    };
    if destination.try_exists()? {
        return Err(invalid("mix-audio destination already exists"));
    }
    validate_output_controls(controls, Some(matroska))?;
    plan_with_controls(sources, options, controls)?;
    let info_chunks = if matroska {
        Vec::new()
    } else {
        let source_info =
            crate::native_export::pcm_matroska::extract_source_wave_info(&sources[0], controls)?;
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &source_info,
            &controls.metadata_delete,
            &controls.metadata_set,
        )
        .map_err(|e| invalid(&e))?
    };
    let source_matroska_metadata = if matroska {
        Some(
            crate::native_export::pcm_matroska::extract_source_pcm_metadata_with_packet_limit(
                &sources[0],
                controls.streams.first().copied(),
                true,
                Some(controls.max_packet_bytes),
                controls.max_packets,
                controls.cancel.as_ref(),
            )?,
        )
    } else {
        None
    };
    let scratch = reserve_scratch(destination)?;
    let (mut inputs, descriptions) = decode_sources_with_options(sources, &scratch, controls)?;
    let channels = descriptions[0].channels;
    let rate = descriptions[0].sample_rate;
    if descriptions
        .iter()
        .any(|s| s.channels != channels || s.sample_rate != rate)
    {
        return Err(invalid("decoded mix-audio geometry changed"));
    }
    let frames = descriptions.iter().map(|s| s.sample_frames).min().unwrap();
    let stats = crate::native_media::AudioDecodeStats {
        sample_frames: frames,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let masks = sources
        .iter()
        .map(|source| validate_and_inspect_source_with_options(source, controls).map(|(_, _, mask)| mask))
        .collect::<Result<Vec<_>>>()?;
    let mask = if masks.iter().all(|m| *m == masks[0]) {
        masks[0]
    } else {
        0
    };
    let mut header = if matroska {
        Vec::new()
    } else {
        crate::native_export::float_wav_header_with_mask(&stats, mask)?
    };
    if !matroska && !info_chunks.is_empty() {
        let riff_size = u32::from_le_bytes(header[4..8].try_into().unwrap())
            .checked_add(
                u32::try_from(info_chunks.len())
                    .map_err(|_| invalid("WAV exceeds RIFF size limit"))?,
            )
            .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
        header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    }
    let output_path = scratch.0.join("mix.wav");
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?,
    );
    output.write_all(&header)?;
    let mut sink = crate::native_export::pcm_matroska::Output::new_with_source_metadata(
        &mut output,
        matroska,
        rate,
        channels,
        source_matroska_metadata,
        Some(controls),
    )?;
    let total: f32 = weights.iter().sum();
    let scales: Vec<f32> = weights
        .iter()
        .map(|&w| if options.normalize { w / total } else { w })
        .collect();
    let block_samples = 4096 * usize::from(channels);
    let mut encoded = vec![0u8; block_samples * 4];
    let mut mixed = vec![0f32; block_samples];
    let mut remaining = frames;
    let mut event = crate::media_control::ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    while remaining > 0 {
        if controls.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(controls).map_err(|e| invalid(&e))?;
        let count = remaining.min(4096) as usize * usize::from(channels);
        mixed[..count].fill(0.0);
        for (input, scale) in inputs.iter_mut().zip(&scales) {
            input.read_exact(&mut encoded[..count * 4])?;
            for (dst, src) in mixed[..count]
                .iter_mut()
                .zip(encoded[..count * 4].as_chunks::<4>().0.iter())
            {
                let sample = f32::from_le_bytes(*src);
                *dst = (f64::from(*dst) + f64::from(sample) * f64::from(*scale)) as f32;
            }
        }
        for (sample, bytes) in mixed[..count]
            .iter()
            .zip(encoded[..count * 4].as_chunks_mut::<4>().0.iter_mut())
        {
            bytes.copy_from_slice(&sample.to_le_bytes());
        }
        sink.write_all(&encoded[..count * 4])?;
        event.packets += 1;
        event.payload_bytes += (count * 4) as u64;
        if let Some(hook) = &controls.progress {
            hook.emit(event);
        }
        remaining -= count as u64 / u64::from(channels);
    }
    if sink.finish()?.is_some_and(|count| count != frames) {
        return Err(invalid("PCM muxed sample count mismatch"));
    }
    if !matroska && !info_chunks.is_empty() {
        output.write_all(&info_chunks)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    std::fs::hard_link(&output_path, destination)?;
    event.done = true;
    if let Some(hook) = &controls.progress {
        hook.emit(event);
    }
    Ok(MixAudioStats {
        backend: "fvid",
        sample_frames: frames,
        sample_rate: rate as i32,
        channels: i32::from(channels),
        sample_format: "flt".into(),
        inputs: sources.len(),
        duration: options.duration.as_str(),
        normalize: options.normalize,
        weights,
    })
}

fn reserve_scratch(destination: &Path) -> Result<Scratch> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let scratch = (0..100)
        .find_map(|_| {
            let path = parent.join(format!(
                ".fvid-mix-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => Some(Ok(Scratch(path))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve mix-audio scratch directory"))??;

    Ok(scratch)
}

fn decode_sources_with_options(
    sources: &[PathBuf],
    scratch: &Scratch,
    options: &fvid_media::CopyOptions,
) -> Result<(
    Vec<BufReader<File>>,
    Vec<crate::native_media::AudioDecodeStats>,
)> {
    let mut inputs = Vec::new();
    let mut descriptions = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let path = scratch.0.join(format!("{i}.f32le"));
        let stats = decode_source_to_f32le_with_options(source, &path, &scratch.0, i, options)?;
        inputs.push(BufReader::new(File::open(path)?));
        descriptions.push(stats);
    }

    Ok((inputs, descriptions))
}

fn merge_geometry_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<(u32, u16)> {
    if sources.len() != 2 {
        return Err(invalid("merge-audio requires exactly two inputs"));
    }
    let mut rate = None;
    let mut channels = 0u16;
    for source in sources {
        let (input_rate, input_channels, _) = validate_and_inspect_source_with_options(source, options)?;
        if rate.is_some_and(|r| r != input_rate) {
            return Err(invalid("merge-audio inputs must share sample rate"));
        }
        rate = Some(input_rate);
        channels = channels
            .checked_add(input_channels)
            .ok_or_else(|| invalid("merge-audio channel count overflow"))?;
    }
    if !(1..=64).contains(&channels) {
        return Err(invalid("merge-audio output requires 1..=64 channels"));
    }
    Ok((rate.unwrap(), channels))
}

pub fn plan_merge(sources: &[PathBuf]) -> Result<crate::media_info::MediaPlan> {
    plan_merge_with_controls(sources, &Default::default())
}

pub fn plan_merge_with_controls(
    sources: &[PathBuf],
    controls: &fvid_media::CopyOptions,
) -> Result<crate::media_info::MediaPlan> {
    let (rate, channels) = merge_geometry_with_options(sources, controls)?;
    let mut notes = vec![
        "Channel order is first input then second input, preserving the existing merge-audio API contract".into(),
        "PCM spools require temporary disk space; mixing memory is bounded independently of duration".into(),
    ];
    if controls.max_packet_bytes != fvid_media::CopyOptions::default().max_packet_bytes
        || controls.max_packets.is_some()
        || controls.max_controlled_bytes.is_some()
        || controls.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; memory limit: {:?}; RSS limit: {:?}",
            controls.max_packet_bytes,
            controls.max_packets,
            controls.max_controlled_bytes,
            controls.max_rss_bytes
        ));
    }
    let mut steps = vec![
        crate::media_info::PlanStep {
            action: "decode".into(),
            detail: "FVid AAC/WAVE decoders to temporary float PCM spools".into(),
        },
        crate::media_info::PlanStep {
            action: "merge".into(),
            detail: format!(
                "concatenate input channel vectors at {rate} Hz into {channels} channels, shortest duration"
            ),
        },
    ];
    if !controls.metadata_set.is_empty()
        || !controls.metadata_delete.is_empty()
        || !controls.stream_metadata_set.is_empty()
        || !controls.stream_metadata_delete.is_empty()
    {
        steps.push(crate::media_info::PlanStep {
            action: "metadata".into(),
            detail: format!(
                "container set={}, delete={}; stream set={}, delete={}",
                controls.metadata_set.len(),
                controls.metadata_delete.len(),
                controls.stream_metadata_set.len(),
                controls.stream_metadata_delete.len()
            ),
        });
    }
    steps.push(crate::media_info::PlanStep {
        action: "write".into(),
        detail: "atomic float32 WAVE or Matroska publication without replacing existing files"
            .into(),
    });
    Ok(crate::media_info::MediaPlan {
        command: "merge-audio".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams: Vec::new(),
        graph: None,
        steps,
        notes,
    })
}

/// Concatenate the channel vectors of exactly two inputs at each sample time.
/// No speaker-position remapping or implicit resampling is performed.
pub fn merge_audio(
    sources: &[PathBuf],
    destination: &Path,
) -> Result<crate::media_info::MergeAudioStats> {
    merge_audio_with_controls(sources, destination, &Default::default())
}

pub fn merge_audio_with_controls(
    sources: &[PathBuf],
    destination: &Path,
    controls: &fvid_media::CopyOptions,
) -> Result<crate::media_info::MergeAudioStats> {
    let (rate, channels) = merge_geometry_with_options(sources, controls)?;
    let matroska = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => false,
        Some("mka" | "mkv") => true,
        _ => return Err(invalid("merge-audio requires .wav, .mka or .mkv output")),
    };
    if destination.try_exists()? {
        return Err(invalid("merge-audio destination already exists"));
    }
    validate_output_controls(controls, Some(matroska))?;
    let info_chunks = if matroska {
        Vec::new()
    } else {
        let source_info =
            crate::native_export::pcm_matroska::extract_source_wave_info(&sources[0], controls)?;
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &source_info,
            &controls.metadata_delete,
            &controls.metadata_set,
        )
        .map_err(|e| invalid(&e))?
    };
    let source_matroska_metadata = if matroska {
        Some(
            crate::native_export::pcm_matroska::extract_source_pcm_metadata_with_packet_limit(
                &sources[0],
                controls.streams.first().copied(),
                true,
                Some(controls.max_packet_bytes),
                controls.max_packets,
                controls.cancel.as_ref(),
            )?,
        )
    } else {
        None
    };
    let scratch = reserve_scratch(destination)?;
    let (mut inputs, descriptions) = decode_sources_with_options(sources, &scratch, controls)?;
    if descriptions.iter().any(|s| s.sample_rate != rate)
        || descriptions
            .iter()
            .map(|s| u32::from(s.channels))
            .sum::<u32>()
            != u32::from(channels)
    {
        return Err(invalid("decoded merge-audio geometry changed"));
    }
    let frames = descriptions.iter().map(|s| s.sample_frames).min().unwrap();
    let mut header = if matroska {
        Vec::new()
    } else {
        merged_wav_header(rate, channels, frames)?
    };
    if !matroska && !info_chunks.is_empty() {
        let riff_size = u32::from_le_bytes(header[4..8].try_into().unwrap())
            .checked_add(
                u32::try_from(info_chunks.len())
                    .map_err(|_| invalid("WAV exceeds RIFF size limit"))?,
            )
            .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
        header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    }
    let output_path = scratch.0.join("merge.wav");
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)?,
    );
    output.write_all(&header)?;
    let mut sink = crate::native_export::pcm_matroska::Output::new_with_source_metadata(
        &mut output,
        matroska,
        rate,
        channels,
        source_matroska_metadata,
        Some(controls),
    )?;
    let mut buffers = descriptions
        .iter()
        .map(|s| vec![0u8; 4096 * usize::from(s.channels) * 4])
        .collect::<Vec<_>>();
    let strides = descriptions
        .iter()
        .map(|s| usize::from(s.channels) * 4)
        .collect::<Vec<_>>();
    let mut remaining = frames;
    let mut event = crate::media_control::ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    while remaining > 0 {
        if controls.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(controls).map_err(|e| invalid(&e))?;
        let count = remaining.min(4096) as usize;
        for ((input, buffer), stride) in inputs.iter_mut().zip(&mut buffers).zip(&strides) {
            input.read_exact(&mut buffer[..count * stride])?;
        }
        for frame in 0..count {
            for (buffer, stride) in buffers.iter().zip(&strides) {
                sink.write_all(&buffer[frame * stride..(frame + 1) * stride])?;
            }
        }
        event.packets += 1;
        event.payload_bytes += (count * usize::from(channels) * 4) as u64;
        if let Some(hook) = &controls.progress {
            hook.emit(event);
        }
        remaining -= count as u64;
    }
    if sink.finish()?.is_some_and(|count| count != frames) {
        return Err(invalid("PCM muxed sample count mismatch"));
    }
    if !matroska && !info_chunks.is_empty() {
        output.write_all(&info_chunks)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    std::fs::hard_link(&output_path, destination)?;
    event.done = true;
    if let Some(hook) = &controls.progress {
        hook.emit(event);
    }
    Ok(crate::media_info::MergeAudioStats {
        backend: "fvid",
        sample_frames: frames,
        sample_rate: rate as i32,
        channels: i32::from(channels),
        sample_format: "flt".into(),
        inputs: 2,
    })
}

// The legacy merge API emits IEEE-float format 3 without a speaker mask. Keep
// that representation: assigning an invented multichannel layout would change
// the meaning of the input-order channel concatenation.
fn merged_wav_header(rate: u32, channels: u16, frames: u64) -> Result<Vec<u8>> {
    let align = channels
        .checked_mul(4)
        .ok_or_else(|| invalid("WAV alignment overflow"))?;
    let bytes = frames
        .checked_mul(u64::from(align))
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
    let byte_rate = rate
        .checked_mul(u32::from(align))
        .ok_or_else(|| invalid("WAV byte rate overflow"))?;
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    Ok(out)
}
