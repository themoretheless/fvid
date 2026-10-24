//! Shared controlled codec selection for Ogg file operations.
use crate::owned_ogg::Coding;
use crate::owned_ogg_opus_export::{CheckedRead, check, preflight};
use fvid_control::CopyOptions;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub(crate) enum Backend {
    Vorbis,
    Opus,
    Joint,
}
pub(crate) fn coding(source: &Path, options: &CopyOptions) -> Result<Backend, String> {
    check(options)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let (limits, _) = preflight(&mut file, options)?;
    let mut bytes = Vec::new();
    CheckedRead {
        input: BufReader::new(file),
        options,
    }
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
    let ogg = crate::owned_ogg::Ogg::parse(&bytes, &limits).map_err(|e| e.to_string())?;
    let catalog = crate::owned_ogg_audio_catalog::from_ogg(&ogg).map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] if catalog.len() == 1 => &catalog[0],
        [] => return Err("select exactly one Ogg audio stream".into()),
        [index] => catalog
            .iter()
            .find(|t| t.source_index == *index)
            .ok_or("selected Ogg audio source stream is absent")?,
        _ => return Err("select exactly one Ogg audio stream".into()),
    };
    check(options)?;
    let nth = catalog
        .iter()
        .position(|t| t.source_index == selected.source_index)
        .unwrap();
    if crate::owned_ogg_audio_catalog::needs_joint_program(&ogg, nth).map_err(|e| e.to_string())? {
        return Ok(Backend::Joint);
    }
    match selected.coding {
        Coding::Vorbis => Ok(Backend::Vorbis),
        Coding::Opus => Ok(Backend::Opus),
        _ => Err("unsupported Ogg audio source codec".into()),
    }
}
pub(crate) fn descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, &'static str), String> {
    let stream_index = options.streams.first().copied().unwrap_or(0);
    match coding(source, options)? {
        Backend::Opus => {
            let p = crate::owned_ogg_opus_export::open(source, options)?;
            if p.end_samples() <= 0 {
                return Err("Ogg Opus audio stream contains no audible samples".into());
            }
            Ok((
                stream_index,
                p.sample_rate(),
                p.channels(),
                p.output_channel_mask().unwrap_or(0),
                "opus",
            ))
        }
        Backend::Vorbis => {
            let (p, _) = crate::owned_ogg_vorbis_export::open(source, options)?;
            if p.duration().is_none_or(|d| d.is_zero()) {
                return Err("Ogg Vorbis audio stream contains no audible samples".into());
            }
            Ok((
                stream_index,
                p.sample_rate(),
                p.channels(),
                p.output_channel_mask(),
                "vorbis",
            ))
        }
        Backend::Joint => {
            let (p, _) = crate::owned_ogg_joint_export::open(source, options)?;
            if p.end_samples() <= 0 {
                return Err("Ogg linked audio stream contains no audible samples".into());
            }
            let codec = match p.codec() {
                "A_OGG/VORBIS_OPUS" => "vorbis_opus",
                "A_VORBIS" => "vorbis",
                _ => "opus",
            };
            Ok((
                stream_index,
                p.sample_rate(),
                p.channels(),
                p.output_channel_mask(),
                codec,
            ))
        }
    }
}
