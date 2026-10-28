//! Matroska FLAC file export via owned presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut signature = [0; 4];
    if file.read_exact(&mut signature).is_err() || signature != [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(false);
    }
    let reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let preferred = reader.preferred_track_numbers(
        &reader
            .tracks
            .iter()
            .filter(|t| t.kind == 2)
            .map(|t| t.number)
            .collect::<Vec<_>>(),
    );
    Ok(reader.tracks.iter().enumerate().any(|(index, t)| {
        ((options.streams.is_empty() && preferred.contains(&t.number))
            || options.streams.contains(&index))
            && t.kind == 2
            && t.codec == "A_FLAC"
    }))
}
fn decode_options(options: &CopyOptions) -> CopyOptions {
    let mut options = options.clone();
    options.metadata_set.clear();
    options.metadata_delete.clear();
    options.stream_metadata_set.clear();
    options.stream_metadata_delete.clear();
    options
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels) = match geometry_for_decode(source, Some(transform), options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let mask = match channel_mask(source, options) {
        Ok(value) => value,
        Err(error) => return error.starts_with("controlled memory budget exceeded:"),
    };
    let target_ok = transform.channels.is_none_or(|target| {
        target == i32::from(channels)
            || (((channels <= 2 && (1..=8).contains(&target))
                || (channels <= 8 && matches!(target, 1 | 2)))
                && ((channels <= 2 && mask == 0)
                    || (mask != 0
                        && crate::owned_pcm_channels::rematrix_layout_supported(channels, mask))))
    });
    if !target_ok {
        return false;
    }
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_flac::decode_matroska_flac_pcm(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok()
        || result
            .err()
            .is_some_and(|e| e.to_string().starts_with("controlled memory budget exceeded:"))
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
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
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let (rate, channels) = geometry_for_decode(source, Some(transform), options)?;
    let mask = channel_mask(source, options)?;
    let info = crate::owned_wave_metadata::matroska_source_info_with_interval(
        source,
        prefix(transform, rate),
        options,
    )?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            crate::owned_matroska_flac::decode_matroska_flac_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    geometry_for_decode(source, None, options)
}

pub(crate) fn geometry_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16)> {
    let mut reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
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
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    if track.codec != "A_FLAC" {
        return Err("selected Matroska audio stream has a different codec".into());
    }
    let decoder = crate::owned_matroska_audio::MatroskaTimelineDecoder::from_matroska(track)
        .map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    let interval = transform.and_then(|t| prefix(t, rate));
    crate::owned_matroska_audio::admit_audio_reader_with_interval(&mut reader, index, interval, options)
        .map_err(|e| e.to_string())?;
    let _ = crate::owned_wave_metadata::matroska_source_info_with_interval(source, interval, options)?;
    if transform.is_some_and(|t| t.interval.is_some()) {
        let mut probe = decode_options(options);
        probe.progress = None;
        let stats = crate::owned_matroska_flac::decode_matroska_flac_pcm(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            &mut std::io::sink(),
            interval,
            &probe,
        )
        .map_err(|e| e.to_string())?;
        crate::owned_ogg_opus_export::check_nonempty_interval_prefix(
            transform,
            stats.sample_rate,
            stats.sample_frames,
        )?;
    }
    Ok((rate, channels))
}

pub(crate) fn channel_mask(source: &Path, options: &CopyOptions) -> Result<u32> {
    let reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index = crate::owned_matroska_audio::matroska_audio_index(
        &reader,
        selected,
    )
    .map_err(|e| e.to_string())?;
    Ok(
        crate::owned_flac_decoder::FlacDecoder::from_matroska(&reader.tracks[index])
            .map_err(|e| e.to_string())?
            .channel_mask(),
    )
}
