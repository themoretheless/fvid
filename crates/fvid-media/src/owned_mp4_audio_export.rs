//! Owned MP4 AAC/ALAC file export through presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
fn open(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_mp4::Mp4Reader<BufReader<File>>> {
    crate::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            packet_bytes: options.max_packet_bytes,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn recognizes(source: &Path, options: &CopyOptions) -> Result<bool> {
    let mut prefix = [0; 8];
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    if file.read_exact(&mut prefix).is_err() || !crate::owned_mp4::recognizes_prefix(&prefix) {
        return Ok(false);
    }
    let reader = open(source, options)?;
    Ok(reader.tracks().iter().enumerate().any(|(index, t)| {
        (options.streams.is_empty() || options.streams.contains(&index))
            && t.handler == *b"soun"
            && matches!(&t.codec, b"mp4a" | b"alac")
    }))
}
fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let reader = open(source, options)?;
    if !reader.refused().is_empty() {
        return Err("MP4 stream selection requires every track to be represented".into());
    }
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index =
        crate::owned_mp4_audio::mp4_audio_index(&reader, selected).map_err(|e| e.to_string())?;
    let track = &reader.tracks()[index];
    let decoder =
        crate::owned_mp4_audio::Mp4TimelineDecoder::new(track).map_err(|e| e.to_string())?;
    if track.timescale == 0
        || track.sample_rate != decoder.sample_rate()
        || track.channels != decoder.channels()
    {
        return Err("MP4 audio export requires valid clock and matching audio geometry".into());
    }
    Ok((
        decoder.sample_rate(),
        decoder.channels(),
        decoder.channel_mask(),
    ))
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    if options.max_controlled_bytes.is_some() {
        return false;
    }
    let Ok((rate, channels, mask)) = geometry(source, options) else {
        return false;
    };
    let output = transform.channels.unwrap_or(i32::from(channels));
    if output != i32::from(channels)
        && (!((channels <= 2 && (1..=8).contains(&output))
            || (channels <= 8 && matches!(output, 1 | 2)))
            || !crate::owned_pcm_channels::standard_mask(channels)
                .is_some_and(|standard| standard == u64::from(mask)))
    {
        return false;
    }
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut probe = options.clone();
    probe.progress = None;
    probe.metadata_set.clear();
    probe.metadata_delete.clear();
    probe.stream_metadata_set.clear();
    probe.stream_metadata_delete.clear();
    crate::owned_mp4_audio::decode_mp4_audio_pcm(
        BufReader::new(file),
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    )
    .is_ok()
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() {
        return Err("MP4 audio aggregate allocation admission is not yet implemented".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let (rate, channels, mask) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            crate::owned_mp4_audio::decode_mp4_audio_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                options,
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
