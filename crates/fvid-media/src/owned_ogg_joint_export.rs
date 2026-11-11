//! Controlled native file export for Ogg programs that change audio codec.
use crate::{
    owned_ogg_audio_program::Program,
    owned_ogg_opus_export::{admit, check},
};
use fvid_control::CopyOptions;
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{io::Write, path::Path};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn open_from_ogg_with_transform(
    ogg: crate::owned_ogg::Ogg,
    index: usize,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(Program, Vec<u8>)> {
    check(options)?;
    let catalog = crate::owned_ogg_audio_catalog::from_ogg(&ogg).map_err(|e| e.to_string())?;
    let nth = match options.streams.as_slice() {
        [] if catalog.len() == 1 => 0,
        [] => return Err("select exactly one Ogg audio stream".into()),
        [index] => catalog
            .iter()
            .position(|t| t.source_index == *index)
            .ok_or("selected Ogg audio source stream is absent")?,
        _ => return Err("select exactly one Ogg audio stream".into()),
    };
    let reserve =
        crate::owned_ogg_audio_program::decoder_reserve(&ogg, nth).map_err(|e| e.to_string())?;
    let first = &ogg.chains()[catalog[nth].source_index];
    let comment = &first
        .packets
        .get(1)
        .ok_or("Ogg comment header missing")?
        .data;
    let tagged = if first.coding == crate::owned_ogg::Coding::Vorbis {
        let payload = comment
            .strip_prefix(b"\x03vorbis")
            .ok_or("Vorbis comment signature missing")?;
        let mut bytes = b"OpusTags".to_vec();
        bytes.extend_from_slice(payload);
        bytes
    } else {
        comment.clone()
    };
    let tags = fvid_opus::OpusTags::parse(&tagged).map_err(|e| e.to_string())?;
    let info = crate::owned_ogg_opus_export::info_for_tags(&tags)?;
    let wave_meta_bytes =
        crate::owned_wave_metadata::wave_metadata_working_bytes(info.len(), options)?;
    if options
        .max_controlled_bytes
        .is_some_and(|max| wave_meta_bytes > max)
    {
        return Err("controlled memory budget exceeded: WAVE metadata limit".into());
    }
    admit(
        index
            .checked_add(reserve.checked_mul(2).ok_or("Ogg audio reserve overflow")?)
            .and_then(|n| n.checked_add(wave_meta_bytes))
            .and_then(|n| n.checked_add(512 * 1024))
            .ok_or("Ogg audio reserve overflow")?,
        options,
    )?;
    let program = Program::from_ogg(ogg, nth).map_err(|e| e.to_string())?;
    let rate = program.sample_rate();
    let until = transform
        .and_then(|t| crate::owned_adts_export::decoded_prefix(t, rate))
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                .map_err(|_| "Ogg PCM prefix overflow".to_string())
        })
        .transpose()?;
    if program.max_packet_size_in_prefix(until, options.max_packets) > options.max_packet_bytes {
        return Err("Ogg audio compressed packet exceeds byte limit".into());
    }
    let prefix_samples = program.prefix_end_samples(until, options.max_packets);
    crate::owned_ogg_opus_export::check_nonempty_interval_prefix(transform, rate, prefix_samples)?;
    check(options)?;
    Ok((program, info))
}
#[cfg(test)]
pub(crate) fn supports_from_ogg(
    ogg: crate::owned_ogg::Ogg,
    index: usize,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match open_from_ogg_with_transform(ogg, index, Some(transform), options) {
        Ok((p, _)) => supports_program(&p, transform),
        Err(e) => e.starts_with("controlled memory budget exceeded:"),
    }
}
#[cfg(test)]
fn supports_program(p: &Program, transform: AudioDecodeTransform) -> bool {
    let target = transform.channels.unwrap_or(i32::from(p.channels()));
    target == i32::from(p.channels())
        || (((p.channels() <= 8 && matches!(target, 1 | 2))
            || (p.channels() <= 2 && (1..=8).contains(&target)))
            && ((p.channels() <= 2 && p.output_channel_mask() == 0)
                || (p.output_channel_mask() != 0
                    && crate::owned_pcm_channels::rematrix_layout_supported(
                        p.channels(),
                        p.output_channel_mask(),
                    ))))
}
fn decode(
    program: &mut Program,
    writer: &mut impl Write,
    until: Option<u64>,
    options: &CopyOptions,
) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
    let width = usize::from(program.channels()) * 4;
    let silence = crate::owned_frame::buffer(1024 * width)?;
    let mut frames = 0;
    while !until.is_some_and(|end| frames >= end) {
        let Some(mut pcm) = program.next_pcm_with(options).map_err(|e| e.to_string())? else {
            break;
        };
        check(options)?;
        let gap_end = until.map_or(pcm.pts, |end| end.min(pcm.pts));
        while frames < gap_end {
            check(options)?;
            let n = (gap_end - frames).min(1024) as usize;
            writer
                .write_all(&silence[..n * width])
                .map_err(|e| e.to_string())?;
            frames += n as u64;
        }
        let n = (pcm.data.len() / width)
            .min(until.map_or(usize::MAX, |end| end.saturating_sub(pcm.pts) as usize));
        pcm.data.truncate(n * width);
        writer.write_all(&pcm.data).map_err(|e| e.to_string())?;
        frames += n as u64;
    }
    Ok(crate::owned_matroska_audio::AudioDecodeStats {
        sample_frames: frames,
        decoded_frames: program.decoded_packets(),
        sample_rate: program.sample_rate(),
        channels: program.channels(),
    })
}
pub(crate) fn decode_to_wave_from_ogg(
    ogg: crate::owned_ogg::Ogg,
    index: usize,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    check(options)?;
    let (program, info) = open_from_ogg_with_transform(ogg, index, None, options)?;
    decode_program_to_wave(program, info, options)
}
fn decode_program_to_wave(
    mut program: Program,
    info: Vec<u8>,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    let rate = program.sample_rate();
    let channels = program.channels();
    let mask = program.output_channel_mask();
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut program, writer, None, options),
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    Ok(spool)
}
pub(crate) fn apply_from_ogg(
    ogg: crate::owned_ogg::Ogg,
    index: usize,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    check(options)?;
    if destination.symlink_metadata().is_ok() {
        return Err("destination already exists".into());
    }
    let (program, info) = open_from_ogg_with_transform(ogg, index, Some(transform), options)?;
    apply_program(program, info, destination, transform, options)
}
fn apply_program(
    mut program: Program,
    info: Vec<u8>,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    let rate = program.sample_rate();
    let channels = program.channels();
    let mask = program.output_channel_mask();
    let until = crate::owned_adts_export::decoded_prefix(transform, rate)
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                .map_err(|_| "Ogg PCM prefix overflow")
        })
        .transpose()?;
    let prefix_samples = program.prefix_end_samples(until, options.max_packets);
    crate::owned_adts_export::check_spool_wave_export_budget(
        rate,
        channels,
        32,
        prefix_samples.max(1024),
        info.len(),
        false,
        transform,
        options,
    )?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut program, writer, until, options),
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
