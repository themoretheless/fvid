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

fn geometry(source: &Path) -> Result<(u32, u16)> {
    if crate::native_pcm::is_wave(source)? {
        let info = crate::native_pcm::inspect(&mut File::open(source)?, None)?;
        Ok((info.sample_rate, info.channels))
    } else {
        let info = crate::native_media::audio_source_info_selected(source, None)?;
        Ok((info.sample_rate, info.channels))
    }
}

pub fn eligible(sources: &[PathBuf]) -> Result<bool> {
    if !(2..=256).contains(&sources.len()) {
        return Ok(false);
    }
    let mut expected_geometry = None;
    for source in sources {
        if !super::eligible(std::slice::from_ref(source))? {
            return Ok(false);
        }
        let info = crate::native_probe::probe(source).map_err(|e| invalid(&e))?;
        if info.streams.len() != 1 || info.streams[0].media_type != "audio" {
            return Ok(false);
        }
        let current = geometry(source)?;
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
    if !matches!(
        destination.extension().and_then(|s| s.to_str()),
        Some("mka" | "mkv")
    ) {
        return Err(invalid("owned PCM concat requires .mka or .mkv output"));
    }
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if !eligible(sources)? {
        return Err(invalid(
            "PCM concat requires 2..=256 single audio inputs with matching sample rate and channels",
        ));
    }
    if destination.try_exists()? {
        return Err(invalid("concat destination already exists"));
    }
    let (rate, channels) = geometry(&sources[0])?;
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
    for source in sources {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        let spool = scratch.0.join("segment.f32le");
        let decoded = crate::native_export::export_audio_pcm_selected(
            source, &spool, None, 1.0, None, None, None, cancel, None,
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
