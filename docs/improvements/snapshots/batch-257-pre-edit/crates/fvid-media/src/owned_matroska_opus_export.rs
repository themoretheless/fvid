//! Matroska Opus file export via owned presentation decoding and WAVE DSP.
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
    let reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let preferred=reader.preferred_track_numbers(&reader.tracks.iter().filter(|t|t.kind==2).map(|t|t.number).collect::<Vec<_>>());
    Ok(reader.tracks.iter().enumerate().any(|(index, t)| {
        ((options.streams.is_empty() && preferred.contains(&t.number))
            || options.streams.contains(&index))
            && t.kind == 2
            && t.codec == "A_OPUS"
    }))
}
fn decode_options(options: &CopyOptions) -> CopyOptions {
    crate::owned_adts_export::pcm_decode_options(options)
}
use crate::owned_adts_export::decoded_prefix as prefix;
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    let (rate, channels, mask) = match geometry_with_mask(source, options) {
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
    let result = crate::owned_matroska_opus::decode_matroska_opus_pcm(
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
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let (rate, channels, mask) = geometry_with_mask(source, options)?;
    let info = crate::owned_wave_metadata::matroska_source_info(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            crate::owned_matroska_opus::decode_matroska_opus_pcm(
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
    geometry_with_mask(source, options).map(|(rate, channels, _)| (rate, channels))
}

pub(crate) fn geometry_with_mask(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let mut reader = crate::owned_matroska_audio::open_audio_reader(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let index = match options.streams.as_slice() {
        [index] => *index,
        [] => {
            let preferred=reader.preferred_track_numbers(&reader.tracks.iter().filter(|t|t.kind==2).map(|t|t.number).collect::<Vec<_>>());
            let indices: Vec<_> = reader
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (t.kind == 2 && preferred.contains(&t.number)).then_some(i))
                .collect();
            if indices.len() != 1 {
                return Err("select exactly one audio stream".into());
            }
            indices[0]
        }
        _ => return Err("select exactly one audio stream".into()),
    };
    crate::owned_matroska_audio::admit_audio_reader(&mut reader, index, options)
        .map_err(|e| e.to_string())?;
    if options.max_controlled_bytes.is_none()
        && options.max_packet_bytes < crate::owned_webm::Limits::default().packet_bytes
    {
        reader.scan_all().map_err(|e| e.to_string())?;
    }
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let rate = u32::try_from(track.sample_rate).map_err(|_| "Opus rate overflow")?;
    let channels = u16::try_from(track.channels).map_err(|_| "Opus channel overflow")?;
    // Qualify geometry without allocating a complete decoder solely to probe it.
    crate::owned_opus::OpusDecoder::decode_admission_bytes(&track.codec_private, rate, channels)?;
    let head = fvid_opus::OpusHead::parse(&track.codec_private).map_err(|e| e.to_string())?;
    let mask = head.wave_channel_mask().unwrap_or(0);
    Ok((rate, channels, mask))
}
