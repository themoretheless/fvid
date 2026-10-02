//! Matroska PCM file export via precise float64 presentation decoding and WAVE DSP.
//! All supported integer samples fit exactly in f64; float64 never passes through f32.
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
    Ok(reader.tracks.iter().enumerate().any(|(index, t)| {
        (options.streams.is_empty() || options.streams.contains(&index))
            && t.kind == 2
            && matches!(
                t.codec.as_str(),
                "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE"
            )
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
    if options.max_controlled_bytes.is_some() {
        return false;
    }
    let Ok((rate, _)) = geometry(source, options) else {
        return false;
    };
    let mut probe = decode_options(options);
    probe.progress = None;
    let result = crate::owned_matroska_pcm::decode_matroska_pcm_f64(
        match File::open(source) {
            Ok(f) => BufReader::new(f),
            Err(_) => return false,
        },
        &mut std::io::sink(),
        prefix(transform, rate),
        &probe,
    );
    result.is_ok_and(|stats| {
        transform.channels.is_none_or(|channels| {
            channels == i32::from(stats.channels)
                || (stats.channels <= 2 && (1..=8).contains(&channels))
                || (stats.channels <= 8 && matches!(channels, 1 | 2))
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
        return Err("Matroska PCM aggregate allocation admission is not yet implemented".into());
    }
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let (rate, channels) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded_with_precision(
        rate,
        channels,
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
        64,
        options,
        |writer, options| {
            crate::owned_matroska_pcm::decode_matroska_pcm_f64(
                BufReader::new(File::open(source).map_err(|e| e.to_string())?),
                writer,
                prefix(transform, rate),
                &decode_options(options),
            )
            .map_err(|e| e.to_string())
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}

fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
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
        crate::owned_pcm_decoder::PcmDecoder::from_matroska(track).map_err(|e| e.to_string())?;
    let (rate, channels) = (decoder.sample_rate(), decoder.channels());
    Ok((rate, channels))
}
