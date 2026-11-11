//! Matroska MP3 file export via owned presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn decode_options(options: &CopyOptions) -> CopyOptions {
    let mut options = options.clone();
    options.metadata_set.clear();
    options.metadata_delete.clear();
    options.stream_metadata_set.clear();
    options.stream_metadata_delete.clear();
    options
}
use crate::owned_adts_export::decoded_prefix as prefix;
#[cfg(test)]
pub(crate) fn supports_from_reader<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, _, _, reader) =
        match geometry_and_reader_for_decode(source, reader, index, Some(transform), options) {
            Ok(value) => value,
            Err(error) => return error.starts_with("controlled memory budget exceeded:"),
        };
    let target_ok = transform.channels.is_none_or(|target| {
        target == i32::from(channels)
            || (channels <= 2 && (1..=8).contains(&target))
            || (channels <= 8 && matches!(target, 1 | 2))
    });
    if !target_ok {
        return false;
    }
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_audio::decode_matroska_audio_reader_pcm(
        reader,
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok()
        || result
            .err()
            .is_some_and(|e| e.to_string().starts_with("controlled memory budget exceeded:"))
}

pub(crate) fn apply_from_reader<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
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
    let (rate, channels, frames, info, reader) =
        geometry_and_reader_for_decode(source, reader, index, Some(transform), options)?;
    crate::owned_adts_export::check_spool_wave_export_budget(
        rate,
        channels,
        32,
        frames.max(1024),
        info.len(),
        false,
        transform,
        options,
    )?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
        options,
        move |writer, options| {
            crate::owned_matroska_audio::decode_matroska_audio_reader_pcm(
                reader,
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





pub(crate) fn geometry_from_reader_for_decode<R: Read + std::io::Seek>(
    source: &Path,
    reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u64, Vec<u8>)> {
    let (rate, channels, estimated_frames, info, _) =
        geometry_and_reader_for_decode(source, reader, index, transform, options)?;
    Ok((rate, channels, estimated_frames, info))
}

fn geometry_and_reader_for_decode<R: Read + std::io::Seek>(
    source: &Path,
    mut reader: crate::owned_webm::WebmReader<R>,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u64, Vec<u8>, crate::owned_webm::WebmReader<R>)> {
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    if !matches!(track.codec.as_str(), "A_MPEG/L1" | "A_MPEG/L2" | "A_MPEG/L3") {
        return Err("selected Matroska audio stream has a different codec".into());
    }
    let decoder = crate::owned_matroska_audio::MatroskaTimelineDecoder::from_matroska(track)
        .map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    let interval = transform.and_then(|t| prefix(t, rate));
    crate::owned_matroska_audio::admit_audio_reader_with_interval(&mut reader, index, interval, options)
        .map_err(|e| e.to_string())?;
    let mut estimated_frames = crate::owned_matroska_audio::matroska_estimated_frames(
        &mut reader,
        index,
        rate,
        interval,
        options.max_packets,
    );
    let info = crate::owned_wave_metadata::matroska_source_info_from_reader(
        &mut reader, index, interval, options,
    )?;
    if transform.is_some_and(|t| t.interval.is_some()) {
        let mut probe = decode_options(options);
        probe.progress = None;
        let stats = crate::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
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
        estimated_frames = stats.sample_frames;
    }
    Ok((rate, channels, estimated_frames, info, reader))
}
