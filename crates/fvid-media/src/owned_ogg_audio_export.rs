//! Shared controlled codec selection for Ogg file operations.
use crate::owned_ogg::Coding;
use crate::owned_ogg_opus_export::{CheckedRead, check, preflight};
use fvid_control::CopyOptions;
use fvid_media_info::AudioDecodeTransform;
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
pub(crate) fn select_backend(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, Backend), String> {
    let (stream_index, backend, _, _) = open_selected_ogg(source, options)?;
    Ok((stream_index, backend))
}
pub(crate) fn open_selected_ogg(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, Backend, crate::owned_ogg::Ogg, usize), String> {
    check(options)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let (limits, index) = preflight(&mut file, options)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(limits.file_bytes)
        .map_err(|_| "Ogg input allocation failed")?;
    CheckedRead {
        input: BufReader::new(file),
        options,
    }
    .read_to_end(&mut bytes)
    .map_err(|e| e.to_string())?;
    let ogg = crate::owned_ogg::Ogg::parse(&bytes, &limits).map_err(|e| e.to_string())?;
    drop(bytes);
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
    let source_index = selected.source_index;
    let nth = catalog
        .iter()
        .position(|t| t.source_index == source_index)
        .unwrap();
    let backend = if crate::owned_ogg_audio_catalog::needs_joint_program(&ogg, nth)
        .map_err(|e| e.to_string())?
    {
        Backend::Joint
    } else {
        match selected.coding {
            Coding::Vorbis => Backend::Vorbis,
            Coding::Opus => Backend::Opus,
            _ => return Err("unsupported Ogg audio source codec".into()),
        }
    };
    Ok((source_index, backend, ogg, index))
}
pub(crate) fn coding(source: &Path, options: &CopyOptions) -> Result<Backend, String> {
    select_backend(source, options).map(|(_, backend)| backend)
}
pub(crate) fn descriptor(
    source: &Path,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, &'static str), String> {
    descriptor_for_decode(source, None, options)
        .map(|(index, rate, channels, mask, codec, _, _)| (index, rate, channels, mask, codec))
}
pub(crate) fn descriptor_for_decode(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(usize, u32, u16, u32, &'static str, u64, usize), String> {
    let (stream_index, backend, ogg, index) = open_selected_ogg(source, options)?;
    match backend {
        Backend::Opus => {
            let (p, info) = crate::owned_ogg_opus_export::open_from_ogg_with_transform(
                ogg, index, transform, options,
            )?;
            if p.end_samples() <= 0 {
                return Err("Ogg Opus audio stream contains no audible samples".into());
            }
            let until = transform
                .and_then(|t| crate::owned_adts_export::decoded_prefix(t, 48000))
                .map(|(_, end)| {
                    u64::try_from((end.as_nanos() * 48000).div_ceil(1_000_000_000))
                        .map_err(|_| "Opus prefix duration overflow".to_string())
                })
                .transpose()?;
            let prefix_samples = p.prefix_end_samples(until, options.max_packets);
            Ok((
                stream_index,
                p.sample_rate(),
                p.channels(),
                p.output_channel_mask().unwrap_or(0),
                "opus",
                prefix_samples,
                info.len(),
            ))
        }
        Backend::Vorbis => {
            let (p, info) = crate::owned_ogg_vorbis_export::open_from_ogg_with_transform(
                ogg, index, transform, options,
            )?;
            if p.duration().is_none_or(|d| d.is_zero()) {
                return Err("Ogg Vorbis audio stream contains no audible samples".into());
            }
            let rate = p.sample_rate();
            let until = transform
                .and_then(|t| crate::owned_adts_export::decoded_prefix(t, rate))
                .map(|(_, end)| {
                    u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                        .map_err(|_| "Vorbis prefix overflow".to_string())
                })
                .transpose()?;
            let prefix_samples = p.prefix_end_samples(until, options.max_packets);
            Ok((
                stream_index,
                rate,
                p.channels(),
                p.output_channel_mask(),
                "vorbis",
                prefix_samples,
                info.len(),
            ))
        }
        Backend::Joint => {
            let (p, info) = crate::owned_ogg_joint_export::open_from_ogg_with_transform(
                ogg, index, transform, options,
            )?;
            if p.end_samples() <= 0 {
                return Err("Ogg linked audio stream contains no audible samples".into());
            }
            let rate = p.sample_rate();
            let until = transform
                .and_then(|t| crate::owned_adts_export::decoded_prefix(t, rate))
                .map(|(_, end)| {
                    u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                        .map_err(|_| "Ogg PCM prefix overflow".to_string())
                })
                .transpose()?;
            let prefix_samples = p.prefix_end_samples(until, options.max_packets);
            let codec = match p.codec() {
                "A_OGG/VORBIS_OPUS" => "vorbis_opus",
                "A_VORBIS" => "vorbis",
                _ => "opus",
            };
            Ok((
                stream_index,
                rate,
                p.channels(),
                p.output_channel_mask(),
                codec,
                prefix_samples,
                info.len(),
            ))
        }
    }
}
