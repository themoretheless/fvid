//! Sample-domain concat for single-stream owned audio sources.
use crate::{
    Result, invalid,
    media_control::{CancelFlag, ProgressEvent, ProgressHook},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

fn geometry_with_options(source: &Path, options: &fvid_media::CopyOptions) -> Result<(u32, u16)> {
    if options.max_controlled_bytes.is_none() && crate::native_pcm::is_wave(source)? {
        let info = crate::native_pcm::inspect(&mut File::open(source)?, options.cancel.as_ref())?;
        Ok((info.sample_rate, info.channels))
    } else if options.max_controlled_bytes.is_none() && crate::native_media::is_owned_audio_source(source)? {
        let info = crate::native_media::audio_source_info_selected(source, None)?;
        Ok((info.sample_rate, info.channels))
    } else {
        let mut probe_opts = options.clone();
        probe_opts.max_packets = None;
        fvid_media::owned_audio_plan::source_geometry(source, &probe_opts)
            .map_err(|e| invalid(&e))
    }
}

pub fn eligible(sources: &[PathBuf]) -> Result<bool> {
    eligible_with_options(sources, &Default::default())
}

pub fn eligible_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<bool> {
    if !(2..=256).contains(&sources.len()) {
        return Ok(false);
    }
    let mut expected_geometry = None;
    for source in sources {
        if !super::eligible_with_options(std::slice::from_ref(source), options)? {
            return Ok(false);
        }
        let info = crate::media::probe(source).map_err(|e| invalid(&e))?;
        if info.streams.len() != 1 || info.streams[0].media_type != "audio" {
            return Ok(false);
        }
        let current = geometry_with_options(source, options)?;
        if expected_geometry.is_some_and(|g| g != current) {
            return Ok(false);
        }
        expected_geometry = Some(current);
    }
    Ok(true)
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
    if !matches!(
        destination.extension().and_then(|s| s.to_str()),
        Some("mka" | "mkv")
    ) {
        return Err(invalid("owned PCM concat requires .mka or .mkv output"));
    }
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    if !eligible_with_options(sources, options)? {
        return Err(invalid(
            "PCM concat requires 2..=256 single audio inputs with matching sample rate and channels",
        ));
    }
    if destination.try_exists()? {
        return Err(invalid("concat destination already exists"));
    }
    let (rate, channels) = geometry_with_options(&sources[0], options)?;
    let scratch = super::reserve_scratch(destination)?;
    let temporary = scratch.0.join("concat.mka");
    let mut output = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?,
    );
    let mut sink =
        crate::native_export::pcm_matroska::Output::new(&mut output, true, rate, channels)?;
    let mut stats = crate::native_media::AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
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
                return Err(invalid("packet work limit exceeded"));
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
    if sink.finish()? != Some(stats.sample_frames) {
        return Err(invalid("concat PCM sample count mismatch"));
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
