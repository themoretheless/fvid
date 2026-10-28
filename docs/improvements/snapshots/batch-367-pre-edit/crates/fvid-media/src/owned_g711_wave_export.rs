//! Native G.711 RIFF/WAVE decode with controlled streaming and common WAVE DSP.
use crate::{
    owned_ogg_opus_export::{admit, check},
    owned_pcm_decoder::{PcmDecoder, PcmFormat},
    owned_wave_inspect::WaveInfo,
};
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{AudioDecodeStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;
struct Input {
    file: File,
    shape: WaveInfo,
    decoder: PcmDecoder,
    mask: u32,
}
pub(crate) fn recognizes(path: &Path, options: &CopyOptions) -> Result<bool> {
    if !crate::owned_wave_inspect::is_wave(path).map_err(|e| e.to_string())? {
        return Ok(false);
    }
    crate::owned_wave_inspect::g711_law(
        &mut File::open(path).map_err(|e| e.to_string())?,
        options.cancel.as_ref(),
    )
    .map(|law| law.is_some())
    .map_err(|e| e.to_string())
}
fn open(path: &Path, options: &CopyOptions) -> Result<(Input, Vec<u8>)> {
    if !crate::owned_audio_export::simple_options(options) {
        return Err("WAVE has only stream 0".into());
    }
    check(options)?;
    admit(256 * 1024, options)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let law = crate::owned_wave_inspect::g711_law(&mut file, options.cancel.as_ref())
        .map_err(|e| e.to_string())?
        .ok_or("WAVE does not contain G.711")?;
    let shape = crate::owned_wave_inspect::inspect_g711(&mut file, options.cancel.as_ref(), law)
        .map_err(|e| e.to_string())?;
    if !(1..=255).contains(&shape.channels) {
        return Err("owned WAVE output supports 1..255 channels".into());
    }
    if options.max_packet_bytes < usize::from(shape.channels) {
        return Err("G.711 channel frame exceeds byte limit".into());
    }
    let metadata =
        crate::owned_wave_inspect::info_chunks_bytes(&mut file, &shape, options.cancel.as_ref())
            .map_err(|e| e.to_string())?;
    admit(
        usize::from(shape.channels)
            .checked_mul(2048 * 9)
            .and_then(|n| metadata.checked_mul(2).and_then(|m| n.checked_add(m)))
            .and_then(|n| n.checked_add(256 * 1024))
            .ok_or("G.711 decode reserve overflow")?,
        options,
    )?;
    let info = crate::owned_wave_inspect::info_chunks(&mut file, &shape, options.cancel.as_ref())
        .map_err(|e| e.to_string())?;
    if options
        .max_controlled_bytes
        .is_some_and(|max| info.len() > max)
    {
        return Err("controlled memory budget exceeded: WAVE metadata limit".into());
    }
    let format = if law == fvid_codecs::g711::Law::ALaw {
        PcmFormat::ALaw
    } else {
        PcmFormat::MuLaw
    };
    let decoder =
        PcmDecoder::new(format, shape.sample_rate, shape.channels).map_err(|e| e.to_string())?;
    let mask = if shape.channel_mask != 0 {
        shape.channel_mask
    } else {
        match shape.channels {
            1 => 4,
            2 => 3,
            _ => 0,
        }
    };
    file.seek(SeekFrom::Start(shape.data_offset))
        .map_err(|e| e.to_string())?;
    Ok((
        Input {
            file,
            shape,
            decoder,
            mask,
        },
        info,
    ))
}
pub(crate) fn descriptor(
    path: &Path,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, &'static str, u64)> {
    descriptor_with_transform(path, None, options)
}
pub(crate) fn descriptor_with_transform(
    path: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32, &'static str, u64)> {
    let (input, _) = open(path, options)?;
    if input.shape.sample_frames == 0 {
        return Err("no decoded audio samples or invalid channel count (nonempty input required)".into());
    }
    let channels = usize::from(input.shape.channels);
    let window = 2048.min(options.max_packet_bytes / channels);
    if window == 0 {
        return Err("G.711 channel frame exceeds byte limit".into());
    }
    let rate = input.shape.sample_rate;
    let until = transform
        .and_then(|t| crate::owned_adts_export::decoded_prefix(t, rate))
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                .map_err(|_| "G.711 PCM prefix overflow".to_string())
        })
        .transpose()?;
    let end = until.map_or(input.shape.sample_frames, |n| {
        n.min(input.shape.sample_frames)
    });
    let frames_read = options.max_packets.map_or(end, |pkts| {
        end.min(pkts.saturating_mul(window as u64))
    });
    if frames_read == 0 {
        return Err("no decoded audio samples or invalid channel count (nonempty input required)".into());
    }
    if let Some(transform) = transform {
        if let Some((from, to)) = transform.interval {
            let out_rate = u128::try_from(transform.sample_rate.unwrap_or(rate as i32))
                .map_err(|_| "invalid output sample rate")?;
            let output_frames = usize::try_from(
                u128::from(frames_read)
                    .checked_mul(out_rate)
                    .ok_or("audio interval overflow")?
                    .div_ceil(u128::from(rate)),
            )
            .map_err(|_| "audio interval overflow")?;
            let boundary = |time: i64| -> Result<usize> {
                usize::try_from(
                    (time as u128)
                        .checked_mul(out_rate)
                        .ok_or("audio interval overflow")?
                        .div_ceil(1_000_000),
                )
                .map(|n| n.min(output_frames))
                .map_err(|_| "audio interval overflow".into())
            };
            let (first, last) = (boundary(from)?, boundary(to)?);
            if last <= first {
                return Err("no decoded audio samples".into());
            }
        }
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let law = crate::owned_wave_inspect::g711_law(&mut file, options.cancel.as_ref())
        .map_err(|e| e.to_string())?
        .ok_or("WAVE does not contain G.711")?;
    let codec = if law == fvid_codecs::g711::Law::ALaw {
        "pcm_alaw"
    } else {
        "pcm_mulaw"
    };
    Ok((
        input.shape.sample_rate,
        input.shape.channels,
        input.mask,
        codec,
        frames_read,
    ))
}
pub(crate) fn supports(
    path: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match descriptor_with_transform(path, Some(transform), options) {
        Ok((_rate, channels, mask, _codec, _frames)) => {
            let target = transform
                .channels
                .unwrap_or(i32::from(channels));
            target == i32::from(channels)
                || (matches!(target, 1 | 2)
                    && crate::owned_pcm_channels::rematrix_layout_supported(
                        channels,
                        mask,
                    ))
                || (channels <= 2 && (1..=8).contains(&target))
        }
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
fn decode(
    input: &mut Input,
    writer: &mut impl Write,
    until: Option<u64>,
    options: &CopyOptions,
) -> Result<crate::owned_matroska_audio::AudioDecodeStats> {
    let channels = usize::from(input.shape.channels);
    let window = 2048.min(options.max_packet_bytes / channels);
    if window == 0 {
        return Err("G.711 channel frame exceeds byte limit".into());
    }
    let end = until.map_or(input.shape.sample_frames, |n| {
        n.min(input.shape.sample_frames)
    });
    let mut bytes = vec![0; window * channels];
    let mut frames = 0;
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    while frames < end && !options.max_packets.is_some_and(|n| event.packets >= n) {
        check(options)?;
        let count = (end - frames).min(window as u64) as usize;
        let data = &mut bytes[..count * channels];
        input.file.read_exact(data).map_err(|e| e.to_string())?;
        let samples = input.decoder.decode_pcm(data).map_err(|e| e.to_string())?;
        event.packets += 1;
        event.payload_bytes += data.len() as u64;
        if let Some(hook) = &options.progress {
            hook.emit(event);
        }
        check(options)?;
        let output: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        writer.write_all(&output).map_err(|e| e.to_string())?;
        frames += count as u64;
    }
    Ok(crate::owned_matroska_audio::AudioDecodeStats {
        decoded_frames: event.packets,
        sample_frames: frames,
        sample_rate: input.shape.sample_rate,
        channels: input.shape.channels,
    })
}
pub(crate) fn decode_to_wave(
    source: &Path,
    options: &CopyOptions,
) -> Result<crate::owned_adts_export::DecodedSpool> {
    check(options)?;
    let (mut input, info) = open(source, options)?;
    let rate = input.shape.sample_rate;
    let channels = input.shape.channels;
    let mask = input.mask;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut input, writer, None, options),
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    Ok(spool)
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("destination already exists".into());
    }
    let (mut input, info) = open(source, options)?;
    let rate = input.shape.sample_rate;
    let channels = input.shape.channels;
    let mask = input.mask;
    let until = crate::owned_adts_export::decoded_prefix(transform, rate)
        .map(|(_, end)| {
            u64::try_from((end.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
                .map_err(|_| "G.711 PCM prefix overflow")
        })
        .transpose()?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        move |writer, options| decode(&mut input, writer, until, options),
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
