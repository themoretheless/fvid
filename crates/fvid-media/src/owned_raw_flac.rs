//! Owned bare FLAC presentation decoding and atomic WAVE DSP.
pub use crate::owned_aac::stream::AudioDecodeStats;
use crate::owned_flac::{Flac, Limits};
use fvid_control::{CopyOptions, ProgressEvent};
use fvid_media_info::{AudioDecodeStats as ExportStats, AudioDecodeTransform};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    time::Duration,
};
type Result<T> = std::result::Result<T, String>;
fn check(options: &CopyOptions) -> Result<()> {
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
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut head = [0; 10];
    let count = file.read(&mut head).map_err(|e| e.to_string())?;
    Ok(head[..count].starts_with(b"fLaC"))
}
pub(crate) fn open(source: &Path, options: &CopyOptions) -> Result<Flac> {
    open_with_transform(source, None, options)
}
pub(crate) fn open_with_transform(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<Flac> {
    open_with_interval(
        source,
        transform.map(|transform| (transform, None)),
        options,
    )
}
fn open_with_interval(
    source: &Path,
    window: Option<(AudioDecodeTransform, Option<(Duration, Duration)>)>,
    options: &CopyOptions,
) -> Result<Flac> {
    check(options)?;
    if !crate::owned_audio_export::simple_options(options) {
        return Err("raw FLAC has only stream 0".into());
    }
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let size = usize::try_from(file.metadata().map_err(|e| e.to_string())?.len())
        .map_err(|_| "FLAC input size overflow")?;
    let estimated = size
        .checked_add(crate::owned_flac_decoder::FlacDecoder::DECODE_ADMISSION_BYTES)
        .ok_or("FLAC allocation estimate overflow")?;
    if options
        .max_controlled_bytes
        .is_some_and(|max| estimated > max)
    {
        return Err("controlled memory budget exceeded: FLAC input/decoder".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "FLAC input allocation failed")?;
    // Read bounded chunks so cancellation/RSS checks remain responsive.
    let mut chunk = [0; 65536];
    loop {
        check(options)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if n > size.saturating_sub(bytes.len()) {
            return Err("FLAC input changed while reading".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let prefix_bounded = options.max_packets.is_some()
        || window.is_some_and(|(transform, interval)| {
            interval.is_some() || transform.interval.is_some()
        });
    let parse_packet_bytes = if prefix_bounded {
        usize::MAX
    } else {
        options.max_packet_bytes
    };
    let parsed = Flac::parse_owned_controlled(
        bytes,
        &Limits {
            file_bytes: size,
            packets: usize::MAX,
        },
        options.max_controlled_bytes,
        parse_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let until = match window {
        Some((_transform, Some((_from, to)))) => Some(to),
        Some((transform, None)) => crate::owned_adts_export::decoded_prefix(
            transform,
            parsed.sample_rate,
        )
        .map(|(_from, to)| to),
        None => None,
    };
    if parsed.max_packet_size_in_prefix(until, options.max_packets) > options.max_packet_bytes {
        return Err("FLAC packet bytes exceed limit".into());
    }
    if let Some(max) = options.max_controlled_bytes {
        let to = match until {
            Some(at) => u64::try_from(
                (at.as_nanos() * u128::from(parsed.sample_rate)).div_ceil(1_000_000_000),
            )
            .unwrap_or(u64::MAX)
            .min(parsed.samples()),
            None => parsed.samples(),
        };
        let mut packets = 0u64;
        let mut max_frame_samples = 0usize;
        for frame in &parsed.frames {
            if frame.pts >= to {
                break;
            }
            if options.max_packets.is_some_and(|limit| packets >= limit) {
                break;
            }
            packets += 1;
            max_frame_samples = max_frame_samples.max(frame.samples as usize);
        }
        let pcm_bytes = max_frame_samples
            .checked_mul(usize::from(parsed.channels))
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or("FLAC float allocation overflow")?;
        let index_bytes = parsed
            .frames
            .capacity()
            .checked_mul(std::mem::size_of::<crate::owned_flac::Frame>())
            .ok_or("FLAC index allocation overflow")?;
        let info = crate::owned_flac_layout::wave_info_from_extra(parsed.metadata_blocks())?;
        let wave_meta_bytes =
            crate::owned_wave_metadata::wave_metadata_working_bytes(info.len(), options)?;
        let total = size
            .checked_add(crate::owned_flac_decoder::FlacDecoder::DECODE_ADMISSION_BYTES)
            .and_then(|n| n.checked_add(index_bytes))
            .and_then(|n| n.checked_add(pcm_bytes))
            .and_then(|n| n.checked_add(wave_meta_bytes))
            .ok_or("FLAC allocation estimate overflow")?;
        if total > max {
            return Err("controlled memory budget exceeded: FLAC decoded PCM".into());
        }
    }
    if let Some((transform, None)) = window {
        let to = match until {
            Some(at) => u64::try_from(
                (at.as_nanos() * u128::from(parsed.sample_rate)).div_ceil(1_000_000_000),
            )
            .unwrap_or(u64::MAX)
            .min(parsed.samples()),
            None => parsed.samples(),
        };
        crate::owned_ogg_opus_export::check_nonempty_interval_prefix(
            Some(transform),
            parsed.sample_rate,
            to,
        )?;
    }
    Ok(parsed)
}
pub(crate) fn descriptor(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    descriptor_with_transform(source, None, options)
}
pub(crate) fn descriptor_with_transform(
    source: &Path,
    transform: Option<AudioDecodeTransform>,
    options: &CopyOptions,
) -> Result<(u32, u16, u32)> {
    let p = open_with_transform(source, transform, options)?;
    if p.samples() == 0 {
        return Err("FLAC stream contains no audible samples".into());
    }
    Ok((p.sample_rate, p.channels, p.channel_mask))
}
pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    let parsed = open(source, options)?;
    if parsed.samples() == 0 {
        return Err("FLAC stream contains no audible samples".into());
    }
    Ok((parsed.sample_rate, parsed.channels))
}
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match descriptor_with_transform(source, Some(transform), options) {
        Ok((_rate, channels, mask)) => {
            let target = transform.channels.unwrap_or(i32::from(channels));
            target == i32::from(channels)
                || (((channels <= 8 && matches!(target, 1 | 2))
                    || (channels <= 2 && (1..=8).contains(&target)))
                    && ((channels <= 2 && mask == 0)
                        || (mask != 0
                            && crate::owned_pcm_channels::rematrix_layout_supported(
                                channels, mask,
                            ))))
        }
        Err(error) => error.starts_with("controlled memory budget exceeded:"),
    }
}
/// Decode a sample window with ceil-rounded interval boundaries.
/// Encoded packet limits include preceding scanned frames. Caller-owned PCM
/// can be partial on error; atomic publication belongs to the file export.
pub fn decode_raw_flac_pcm(
    source: &Path,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_raw_checked(source, output, interval, options)
}
fn decode_raw_checked(
    source: &Path,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    check(options)?;
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("raw PCM stream cannot apply metadata mutations".into());
    }
    if interval.is_some_and(|(a, b)| a >= b) {
        return Err("audio interval requires from < to".into());
    }
    let file_bytes = usize::try_from(std::fs::metadata(source).map_err(|e| e.to_string())?.len())
        .map_err(|_| "FLAC input size overflow")?;
    let parsed = open_with_interval(
        source,
        Some((AudioDecodeTransform::default(), interval)),
        options,
    )?;
    let rate = parsed.sample_rate;
    let channels = parsed.channels;
    let boundary = |at: Duration| -> Result<u64> {
        u64::try_from((at.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
            .map_err(|_| "FLAC interval overflow".into())
    };

    let (first, last) = match interval {
        Some((a, b)) => (boundary(a)?, boundary(b)?),
        None => (0, parsed.samples()),
    };
    let from = first;
    let to = last.min(parsed.samples());
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let config = fvid_codecs::flac::Config {
        sample_rate: rate,
        channels,
        bits: parsed.bits_per_sample as u8,
    };
    let mut event = ProgressEvent {
        packets: 0,
        payload_bytes: 0,
        done: false,
    };
    if let Some(hook) = &options.progress {
        hook.emit(event);
    }
    check(options)?;
    let count_packet = |data: &[u8], event: &mut ProgressEvent| -> Result<bool> {
        check(options)?;
        if options.max_packets.is_some_and(|max| event.packets >= max) {
            return Ok(false);
        }
        if data.len() > options.max_packet_bytes {
            return Err("FLAC packet bytes exceed limit".into());
        }
        event.packets += 1;
        event.payload_bytes += data.len() as u64;
        if let Some(hook) = &options.progress {
            hook.emit(*event);
        }
        check(options)?;
        Ok(true)
    };
    for (index, frame) in parsed.frames.iter().enumerate() {
        if frame.pts >= to {
            break;
        }
        let packet = parsed.packet(index);
        if !count_packet(packet, &mut event)? {
            break;
        }
        let decoded = fvid_codecs::flac::decode(packet, config).map_err(|e| e.to_string())?;
        if let Some(max) = options.max_controlled_bytes {
            let pcm_bytes = decoded
                .samples
                .len()
                .checked_mul(std::mem::size_of::<f32>())
                .ok_or("FLAC float allocation overflow")?;
            let index_bytes = parsed
                .frames
                .capacity()
                .checked_mul(std::mem::size_of::<crate::owned_flac::Frame>())
                .ok_or("FLAC index allocation overflow")?;
            let total = file_bytes
                .checked_add(crate::owned_flac_decoder::FlacDecoder::DECODE_ADMISSION_BYTES)
                .and_then(|n| n.checked_add(index_bytes))
                .and_then(|n| n.checked_add(pcm_bytes))
                .ok_or("FLAC allocation estimate overflow")?;
            if total > max {
                return Err("controlled memory budget exceeded: FLAC decoded PCM".into());
            }
        }
        let scale = (1u64 << (config.bits - 1)) as f64;
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(decoded.samples.len())
            .map_err(|_| "FLAC float allocation failed")?;
        pcm.extend(
            decoded
                .samples
                .into_iter()
                .map(|v| (f64::from(v) / scale) as f32),
        );
        stats.decoded_frames += 1;
        let begin = from.saturating_sub(frame.pts).min(u64::from(frame.samples)) as usize;
        let end = to.saturating_sub(frame.pts).min(u64::from(frame.samples)) as usize;
        if end > begin {
            for sample in &pcm[begin * usize::from(channels)..end * usize::from(channels)] {
                output
                    .write_all(&sample.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
            stats.sample_frames += (end - begin) as u64;
        }
    }
    if stats.sample_frames == 0 {
        return Err("FLAC audio interval contains no samples".into());
    }
    Ok(stats)
}
pub(crate) fn source_info(source: &Path, options: &CopyOptions) -> Result<Vec<u8>> {
    let mut inspect_opts = crate::owned_adts_export::pcm_decode_options(options);
    inspect_opts.max_packets = None;
    inspect_opts.max_packet_bytes = usize::MAX;
    let parsed = open(source, &inspect_opts)?;
    let info = crate::owned_flac_layout::wave_info_from_extra(parsed.metadata_blocks())?;
    if let Some(max) = options.max_controlled_bytes {
        let wave_meta_bytes =
            crate::owned_wave_metadata::wave_metadata_working_bytes(info.len(), options)?;
        if wave_meta_bytes > max {
            return Err("controlled memory budget exceeded: WAVE metadata limit".into());
        }
    }
    Ok(info)
}
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<ExportStats> {
    check(options)?;
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    let (rate, channels, mask) = descriptor_with_transform(source, Some(transform), options)?;
    let info = source_info(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        mask,
        options,
        |writer, options| {
            decode_raw_flac_pcm(
                source,
                writer,
                crate::owned_adts_export::decoded_prefix(transform, rate),
                &crate::owned_adts_export::pcm_decode_options(options),
            )
        },
    )?;
    check(options)?;
    crate::owned_ogg_opus_export::append_info(&spool.wave, &info)?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
