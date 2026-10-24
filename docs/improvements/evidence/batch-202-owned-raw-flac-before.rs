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
    crate::owned_budget::check_rss_budget(options)
}
pub(crate) fn recognizes(source: &Path) -> Result<bool> {
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut head = [0; 10];
    let count = file.read(&mut head).map_err(|e| e.to_string())?;
    Ok(head[..count].starts_with(b"fLaC"))
}
pub(crate) fn open(source: &Path, options: &CopyOptions) -> Result<Flac> {
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
    let parsed = Flac::parse_owned_controlled(
        bytes,
        &Limits {
            file_bytes: size,
            packets: usize::MAX,
        },
        options.max_controlled_bytes,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    Ok(parsed)
}
pub(crate) fn descriptor(source: &Path, options: &CopyOptions) -> Result<(u32, u16, u32)> {
    let p = open(source, options)?;
    Ok((p.sample_rate, p.channels, p.channel_mask))
}
pub(crate) fn geometry(source: &Path, options: &CopyOptions) -> Result<(u32, u16)> {
    let parsed = open(source, options)?;
    Ok((parsed.sample_rate, parsed.channels))
}
pub(crate) fn supports(
    source: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> bool {
    match descriptor(source, options) {
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
    let parsed = open(source, options)?;
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
pub(crate) fn apply(
    source: &Path,
    destination: &Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<ExportStats> {
    if destination.symlink_metadata().is_ok() {
        return Err("output already exists".into());
    }
    check(options)?;
    let (rate, channels, mask) = descriptor(source, options)?;
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
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
