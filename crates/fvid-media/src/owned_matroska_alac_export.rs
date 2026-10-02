//! Matroska ALAC file export via owned presentation decoding and WAVE DSP.
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
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
    Ok(reader
        .tracks
        .iter()
        .any(|t| t.kind == 2 && t.codec == "A_ALAC"))
}
fn decode_options(options: &CopyOptions) -> CopyOptions {
    let mut options = options.clone();
    options.metadata_set.clear();
    options.metadata_delete.clear();
    options.stream_metadata_set.clear();
    options.stream_metadata_delete.clear();
    options
}
fn prefix(transform: AudioDecodeTransform) -> Option<(std::time::Duration, std::time::Duration)> {
    transform.interval.map(|(_, to)| {
        (
            std::time::Duration::ZERO,
            std::time::Duration::from_micros(to as u64),
        )
    })
}
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    if options.max_controlled_bytes.is_some() {
        return false;
    }
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_alac::decode_matroska_alac_pcm(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        &mut std::io::sink(),
        prefix(transform),
        &probe,
    );
    result.is_ok_and(|stats| {
        transform.channels.is_none_or(|channels| {
            channels == i32::from(stats.channels) || (1..=8).contains(&channels)
        })
    })
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if options.max_controlled_bytes.is_some() {
        return Err("Matroska ALAC aggregate allocation admission is not yet implemented".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    let index = match options.streams.as_slice() {
        [index] => *index,
        [] => {
            let indices: Vec<_> = reader
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (t.kind == 2).then_some(i))
                .collect();
            if indices.len() != 1 {
                return Err("select exactly one audio stream".into());
            }
            indices[0]
        }
        _ => return Err("select exactly one audio stream".into()),
    };
    let track = reader
        .tracks
        .get(index)
        .ok_or("selected audio stream is absent")?;
    let decoder =
        crate::owned_alac::AlacDecoder::from_matroska(track).map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    drop(reader);
    drop(decoder);
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
        options,
        |writer, options| {
            crate::owned_matroska_alac::decode_matroska_alac_pcm(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
