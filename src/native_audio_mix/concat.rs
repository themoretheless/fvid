//! Sample-domain concat for single-stream owned audio sources.
use crate::{
    Result, invalid,
    media_control::{CancelFlag, ProgressEvent, ProgressHook},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

fn geometry_with_options(source: &Path, options: &fvid_media::CopyOptions) -> Result<(u32, u16)> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let mut default_probe = fvid_media::CopyOptions::default();
    default_probe.cancel = options.cancel.clone();
    default_probe.max_rss_bytes = options.max_rss_bytes;
    if options.max_controlled_bytes.is_none() {
        let mut wave_file = File::open(source)?;
        if crate::native_pcm::is_wave_reader(&mut wave_file)? {
            let info = crate::native_pcm::inspect(&mut wave_file, options.cancel.as_ref())?;
            return Ok((info.sample_rate, info.channels));
        }
        if let Ok(info) = crate::native_media::constant_audio_source_info_selected_from_file_with_interval_and_options(
            wave_file,
            None,
            None,
            options,
        ) {
            return Ok((info.sample_rate, info.channels));
        }
    }
    fvid_media::owned_audio_plan::source_geometry(source, options)
        .map_err(|e| invalid(&e))
}

fn segment_packet_count(source: &Path, options: &fvid_media::CopyOptions) -> Result<u64> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let mut raw_file = File::open(source)?;
    if crate::native_pcm::is_wave_reader(&mut raw_file)? {
        let info = crate::native_pcm::inspect(&mut raw_file, options.cancel.as_ref())?;
        let frame = usize::from(info.frame_bytes());
        if frame == 0 {
            return Err(invalid("packet byte limit is smaller than one PCM sample frame"));
        }
        let max_chunk_bytes = if options.max_controlled_bytes.is_some() {
            4096 * frame
        } else {
            65536
        };
        let chunk_frames = (max_chunk_bytes.min(options.max_packet_bytes) / frame) as u64;
        if chunk_frames == 0 {
            return Err(invalid("packet byte limit is smaller than one PCM sample frame"));
        }
        return Ok(info.sample_frames.div_ceil(chunk_frames));
    }
    let mut input = std::io::BufReader::new(raw_file);
    let mut prefix = [0u8; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    if crate::container::mp4::recognizes_prefix(&prefix) {
        let reader = crate::container::mp4::Mp4Reader::open_indexed(
            input,
            crate::container::mp4::Limits {
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                packet_bytes: options
                    .max_packet_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )?;
        if let Some(mem_limit) = options.max_controlled_bytes
            && reader.estimated_index_payload_bytes()? > mem_limit
        {
            return Err(invalid("controlled memory budget exceeded"));
        }
        let stream_index = crate::native_media::mp4_audio_index(&reader, None)?;
        if let Some(track) = reader.tracks().get(stream_index) {
            track.ensure_constant_description()?;
            if track.edits.is_empty() {
                return Ok(track.samples.len() as u64);
            }
            // Indexed sample count is not decode work for an edited source:
            // edits can stop early, replay media, or retain decoder preroll.
            // Count through the same bounded decoder as execution, discarding
            // PCM instead of allocating a spool or publishing progress.
            let mut file = reader.into_inner().into_inner();
            file.seek(SeekFrom::Start(0))?;
            let mut count_options = super::strip_output_controls(options);
            count_options.progress = None;
            return fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                file,
                &mut std::io::sink(),
                None,
                &count_options,
            )
            .map(|stats| stats.decoded_frames)
            .map_err(|error| invalid(&error.to_string()));
        }
        return Ok(u64::MAX);
    }
    if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let memory_packets = options
            .max_controlled_bytes
            .map(|limit| limit / std::mem::size_of::<crate::container::webm::Packet>());
        let mut limits = crate::container::webm::Limits::default();
        // Count packet headers with the global parser/memory ceiling. The
        // source inspector checks byte admission only on the reached prefix.
        limits.packet_bytes = limits.packet_bytes.min(options.max_controlled_bytes.unwrap_or(usize::MAX));
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets.is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader = crate::container::webm::WebmReader::open(input, limits)
            .map_err(map_packet_limit)?;
        reader.scan_all().map_err(map_packet_limit)?;
        if let Some(mem_limit) = options.max_controlled_bytes
            && reader.estimated_index_payload_bytes()? > mem_limit
        {
            return Err(invalid("controlled memory budget exceeded"));
        }
        let index = crate::native_media::matroska_audio_header_index(&reader, None)?;
        if let Some(track) = reader.tracks.get(index) {
            let track_number = track.number;
            return Ok(reader
                .packets
                .iter()
                .filter(|p| p.track == track_number)
                .count() as u64);
        }
        return Ok(u64::MAX);
    }
    if crate::container::adts::header(&prefix).is_some() {
        let mut reader = crate::container::adts::StreamReader::open_with_packet_limit(
            input,
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )?;
        let mut count = 0u64;
        while options.max_packets.is_none_or(|max| count < max) {
            if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(invalid("media operation cancelled"));
            }
            fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
            if reader.next_packet()?.is_none() {
                break;
            }
            count += 1;
        }
        return Ok(count);
    }
    let file = input.into_inner();
    fvid_media::owned_audio_plan::standalone_packet_work_from_file(file, options)
        .map(|count| count.unwrap_or(u64::MAX))
        .map_err(|error| invalid(&error))
}

pub fn validate_concat_segments_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    validate_concat_segments_internal(sources, options, None).map(|_| ())
}

fn validate_concat_segments_internal(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
    matroska: Option<bool>,
) -> Result<(
    u32,
    u16,
    u32,
    Option<Vec<u8>>,
    Option<crate::native_export::pcm_matroska::MatroskaPcmMetadata>,
)> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    if !(2..=256).contains(&sources.len()) {
        return Err(invalid(
            "PCM concat requires 2..=256 single audio inputs with matching sample rate and channels",
        ));
    }
    super::validate_output_controls(options, None)?;
    let mut remaining_packets = options.max_packets;
    let mut masks = Vec::new();
    let mut active_geometries = Vec::new();
    let mut packet_counts = Vec::new();
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        if remaining_packets == Some(0) {
            break;
        }
        let mut segment_opts = super::strip_output_controls(options);
        segment_opts.max_packets = remaining_packets;
        let (rate, channels, mask) =
            super::validate_and_inspect_source_with_options(source, &segment_opts)?;
        masks.push(mask);
        active_geometries.push((rate, channels));
        if let Some(rem) = &mut remaining_packets {
            let count = segment_packet_count(source, &segment_opts)?;
            packet_counts.push(count);
            *rem = rem.saturating_sub(count);
        }
    }
    let mut default_probe = fvid_media::CopyOptions::default();
    default_probe.cancel = options.cancel.clone();
    default_probe.max_rss_bytes = options.max_rss_bytes;
    default_probe.max_controlled_bytes = options.max_controlled_bytes;
    default_probe.max_packet_bytes = options.max_packet_bytes;
    let mut expected_geometry = None;
    let mut rem_geom = options.max_packets;
    for (idx, source) in sources.iter().enumerate() {
        if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        let mut geom_probe = default_probe.clone();
        geom_probe.max_packets = if rem_geom == Some(0) { Some(1) } else { rem_geom };
        if rem_geom == Some(0) {
            // Unreached segments still need compatible stream headers, but the
            // active encoded-packet cap must not be applied to their payloads.
            geom_probe.max_packet_bytes = fvid_media::CopyOptions::default().max_packet_bytes;
        }
        let mut eligible_probe = geom_probe.clone();
        eligible_probe.max_controlled_bytes = None;
        if active_geometries.get(idx).is_none()
            && !super::eligible_with_options(std::slice::from_ref(source), &eligible_probe)?
        {
            return Err(invalid(
                "PCM concat requires 2..=256 single audio inputs with matching sample rate and channels",
            ));
        }
        crate::native_plan::audio_stream_index_from_metadata(source, None, &geom_probe)
            .map_err(|e| invalid(&e))?;
        let current = if let Some(&geom) = active_geometries.get(idx) {
            geom
        } else {
            geometry_with_options(source, &geom_probe)?
        };
        if rem_geom != Some(0) {
            if expected_geometry.is_some_and(|g| g != current) {
                return Err(invalid(
                    "PCM concat requires 2..=256 single audio inputs with matching sample rate and channels",
                ));
            }
            expected_geometry = Some(current);
        }
        if let Some(rem) = &mut rem_geom
            && *rem > 0
        {
            if let Some(&count) = packet_counts.get(idx) {
                *rem = rem.saturating_sub(count);
            } else if let Ok(count) = segment_packet_count(source, &geom_probe) {
                *rem = rem.saturating_sub(count);
            }
        }
    }
    let (rate, channels) = match expected_geometry {
        Some(geom) => geom,
        None => geometry_with_options(&sources[0], options)?,
    };
    let (cached_wave_info, cached_matroska_meta) =
        crate::native_export::pcm_matroska::check_output_memory_budget_with_wave_and_metadata(
            &sources[0],
            channels,
            81920,
            options,
            matroska,
        )?;
    let mask = if !masks.is_empty() && masks.iter().all(|m| *m == masks[0]) {
        masks[0]
    } else {
        0
    };
    Ok((rate, channels, mask, cached_wave_info, cached_matroska_meta))
}

pub fn eligible(sources: &[PathBuf]) -> Result<bool> {
    eligible_with_options(sources, &Default::default())
}

pub fn eligible_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    Ok(validate_concat_segments_with_options(sources, options).is_ok())
}

/// Decode each source independently, retaining its audible presentation samples.
/// Output is float32 Matroska, with no metadata or compressed-payload retention.
pub fn concat_audio(
    sources: &[PathBuf],
    destination: &Path,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    concat_audio_with_options(sources, destination, &options)
}

pub fn concat_audio_with_options(
    sources: &[PathBuf],
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<crate::native_media::AudioDecodeStats> {
    let cancel = options.cancel.as_ref();
    let progress = options.progress.as_ref();
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let matroska = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => false,
        Some("mka" | "mkv") => true,
        _ => return Err(invalid("owned PCM concat requires .wav, .mka or .mkv output")),
    };
    if destination.symlink_metadata().is_ok() {
        return Err(invalid("concat destination already exists"));
    }
    super::validate_output_controls(options, Some(matroska))?;
    let (rate, channels, validated_mask, cached_wave_info, cached_matroska_meta) =
        validate_concat_segments_internal(sources, options, Some(matroska))?;
    let info_chunks = if matroska {
        Vec::new()
    } else {
        let source_info = match cached_wave_info {
            Some(info) => info,
            None => crate::native_export::pcm_matroska::extract_source_wave_info(&sources[0], options)?,
        };
        let edited = fvid_media::owned_wave_metadata::edit_info_chunks(
            &source_info,
            &options.metadata_delete,
            &options.metadata_set,
        )
        .map_err(|e| invalid(&e))?;
        if options
            .max_controlled_bytes
            .is_some_and(|max| edited.len() > max)
        {
            return Err(invalid(
                "controlled memory budget exceeded: WAVE metadata limit",
            ));
        }
        edited
    };
    let source_matroska_metadata = if matroska {
        match cached_matroska_meta {
            Some(meta) => Some(meta),
            None => Some(
                crate::native_export::pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
                    &sources[0],
                    options.streams.first().copied(),
                    true,
                    None,
                    Some(options.max_packet_bytes),
                    options.max_packets,
                    options.max_controlled_bytes,
                    cancel,
                )?,
            ),
        }
    } else {
        None
    };
    let mask = if matroska { 0 } else { validated_mask };
    let scratch = super::reserve_scratch(destination)?;
    let temporary = scratch.0.join(if matroska { "concat.mka" } else { "concat.wav" });
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?,
    );
    let mut stats = crate::native_media::AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    if !matroska {
        let placeholder = crate::native_export::float_wav_header_with_mask(&stats, mask)?;
        output.write_all(&placeholder)?;
    }
    let mut sink = crate::native_export::pcm_matroska::Output::new_with_source_metadata(
        &mut output,
        matroska,
        rate,
        channels,
        source_matroska_metadata,
        Some(options),
    )?;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    let mut buffer = [0u8; 16384];
    for (i, source) in sources.iter().enumerate() {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        let mut segment_options = options.clone();
        if let Some(limit) = options.max_packets {
            let remaining = limit.saturating_sub(stats.decoded_frames);
            if remaining == 0 {
                break;
            }
            segment_options.max_packets = Some(remaining);
        }
        let spool = scratch.0.join("segment.f32le");
        let decoded = super::decode_source_to_f32le_with_options(
            source,
            &spool,
            &scratch.0,
            i,
            &segment_options,
        )?;
        if (decoded.sample_rate, decoded.channels) != (rate, channels) {
            return Err(invalid("decoded concat geometry changed"));
        }
        let mut input = File::open(&spool)?;
        loop {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                return Err(invalid("media operation cancelled"));
            }
            fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            sink.write_all(&buffer[..n])?;
            event.payload_bytes = event
                .payload_bytes
                .checked_add(n as u64)
                .ok_or_else(|| invalid("concat byte count overflow"))?;
        }
        drop(input);
        std::fs::remove_file(spool)?;
        stats.sample_frames = stats
            .sample_frames
            .checked_add(decoded.sample_frames)
            .ok_or_else(|| invalid("concat sample count overflow"))?;
        stats.decoded_frames = stats
            .decoded_frames
            .checked_add(decoded.decoded_frames)
            .ok_or_else(|| invalid("concat decoded frame count overflow"))?;
        if options.max_packets.is_some_and(|limit| stats.decoded_frames > limit) {
            return Err(invalid("packet work limit exceeded"));
        }
        event.packets = stats.decoded_frames;
        if let Some(hook) = progress {
            hook.emit(event);
        }
    }
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if sink.finish()?.is_some_and(|count| count != stats.sample_frames) {
        return Err(invalid("concat PCM sample count mismatch"));
    }
    if !matroska {
        if !info_chunks.is_empty() {
            output.write_all(&info_chunks)?;
        }
        let mut header = crate::native_export::float_wav_header_with_mask(&stats, mask)?;
        if !info_chunks.is_empty() {
            let riff_size = u32::from_le_bytes(header[4..8].try_into().unwrap())
                .checked_add(
                    u32::try_from(info_chunks.len())
                        .map_err(|_| invalid("WAV exceeds RIFF size limit"))?,
                )
                .ok_or_else(|| invalid("WAV exceeds RIFF size limit"))?;
            header[4..8].copy_from_slice(&riff_size.to_le_bytes());
        }
        output.seek(SeekFrom::Start(0))?;
        output.write_all(&header)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    std::fs::hard_link(&temporary, destination)?;
    event.done = true;
    if let Some(hook) = progress {
        hook.emit(event);
    }
    Ok(stats)
}
