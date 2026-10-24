//! Owned raw MPEG Layer I/II/III file presentation decoding and atomic WAVE DSP.
pub use crate::owned_aac::stream::AudioDecodeStats;
use crate::owned_mpeg_audio::{LAYER_I, LAYER_II, LAYER_III, Limits, Mp3};
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
    Ok(crate::owned_mpeg_audio::has_signature(&head[..count]))
}
pub(crate) fn open(source: &Path, options: &CopyOptions) -> Result<Mp3> {
    check(options)?;
    if !crate::owned_audio_export::simple_options(options) {
        return Err("raw MP3 has only stream 0".into());
    }
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let size = usize::try_from(file.metadata().map_err(|e| e.to_string())?.len())
        .map_err(|_| "MPEG input size overflow")?;
    let estimated = size
        .checked_add(fvid_codecs::mp3::Decoder::decode_admission_bytes())
        .ok_or("MPEG allocation estimate overflow")?;
    if options
        .max_controlled_bytes
        .is_some_and(|max| estimated > max)
    {
        return Err("controlled memory budget exceeded: MPEG input/decoder".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| "MPEG input allocation failed")?;
    // Read bounded chunks so cancellation/RSS checks remain responsive.
    let mut chunk = [0; 65536];
    loop {
        check(options)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if n > size.saturating_sub(bytes.len()) {
            return Err("MPEG input changed while reading".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let parsed = Mp3::parse_owned_controlled(
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
pub(crate) fn descriptor(source: &Path, options: &CopyOptions) -> Result<(u32, u16, &'static str)> {
    let parsed = open(source, options)?;
    Ok((
        parsed.sample_rate,
        parsed.channels,
        match parsed.tag {
            LAYER_I => "mp1",
            LAYER_II => "mp2",
            _ => "mp3",
        },
    ))
}
enum MpegDecoder {
    One(Box<fvid_codecs::mp1::Decoder>),
    Two(Box<fvid_codecs::mp2::Decoder>),
    Three(Box<fvid_codecs::mp3::Decoder>),
}
impl MpegDecoder {
    fn new(parsed: &Mp3) -> Result<Self> {
        Ok(match parsed.tag {
            LAYER_I => Self::One(Box::new(
                fvid_codecs::mp1::Decoder::new(parsed.sample_rate, parsed.channels)
                    .map_err(|e| e.to_string())?,
            )),
            LAYER_II => Self::Two(Box::new(
                fvid_codecs::mp2::Decoder::new(parsed.sample_rate, parsed.channels)
                    .map_err(|e| e.to_string())?,
            )),
            _ => Self::Three(Default::default()),
        })
    }
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>> {
        match self {
            Self::One(d) => d.decode(packet).map_err(|e| e.to_string()),
            Self::Two(d) => d.decode(packet).map_err(|e| e.to_string()),
            Self::Three(d) => d
                .decode(packet)
                .map(|pcm| pcm.samples)
                .map_err(|e| e.to_string()),
        }
    }
    fn prime_info(&mut self, packet: &[u8]) -> Result<()> {
        match self {
            Self::Three(d) => d.prime_info(packet).map_err(|e| e.to_string()),
            _ => Err("Info history requires Layer III".into()),
        }
    }
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
    match geometry(source, options) {
        Ok((_rate, channels)) => {
            let target = transform.channels.unwrap_or(i32::from(channels));
            let mask = crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32;
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
/// Decode an audible sample window. Every preceding encoded frame is retained
/// for reservoir/overlap history; packet limits include the Info prefix.
/// Caller-owned output can be partial on failure; completion belongs to export.
/// Compatibility entry point restricted to Layer III. Other layers use the
/// generic owned MPEG entry point below.
pub fn decode_raw_mp3_pcm(
    source: &Path,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_raw_checked(source, output, interval, options, Some(LAYER_III))
}
/// Owned bare MPEG Layers I, II and III presentation decoding.
pub fn decode_raw_mpeg_pcm(
    source: &Path,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_raw_checked(source, output, interval, options, None)
}
fn decode_raw_checked(
    source: &Path,
    output: &mut impl Write,
    interval: Option<(Duration, Duration)>,
    options: &CopyOptions,
    required: Option<&str>,
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
    if required.is_some_and(|tag| parsed.tag != tag) {
        return Err("selected bare MPEG audio stream is not Layer III".into());
    }
    let rate = parsed.sample_rate;
    let channels = parsed.channels;
    let boundary = |at: Duration| -> Result<u64> {
        u64::try_from((at.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000))
            .map_err(|_| "MPEG interval overflow".into())
    };
    let window = parsed.presentation_window();
    let (first, last) = match interval {
        Some((a, b)) => (boundary(a)?, boundary(b)?),
        None => (0, parsed.presentation_samples()),
    };
    let from = window.start.saturating_add(first);
    let to = window.start.saturating_add(last).min(window.end);
    let mut stats = AudioDecodeStats {
        sample_frames: 0,
        decoded_frames: 0,
        sample_rate: rate,
        channels,
    };
    let mut decoder = MpegDecoder::new(&parsed)?;
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
            return Err("MPEG packet bytes exceed limit".into());
        }
        event.packets += 1;
        event.payload_bytes += data.len() as u64;
        if let Some(hook) = &options.progress {
            hook.emit(*event);
        }
        check(options)?;
        Ok(true)
    };
    if let Some(info) = parsed.info_packet() {
        if !count_packet(info, &mut event)? {
            return Err("MP3 packet limit contains no samples".into());
        }
        decoder.prime_info(info)?;
    }
    for (index, frame) in parsed.frames.iter().enumerate() {
        if frame.pts >= to {
            break;
        }
        let packet = parsed.packet(index);
        if !count_packet(packet, &mut event)? {
            break;
        }
        let pcm = decoder.decode(packet)?;
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
        return Err("MPEG audio interval contains no samples".into());
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
    let (rate, channels) = geometry(source, options)?;
    let spool = crate::owned_adts_export::spool_decoded(
        rate,
        channels,
        crate::owned_pcm_channels::standard_mask(channels).unwrap_or(0) as u32,
        options,
        |writer, options| {
            decode_raw_mpeg_pcm(
                source,
                writer,
                crate::owned_adts_export::decoded_prefix(transform, rate),
                &crate::owned_adts_export::pcm_decode_options(options),
            )
        },
    )?;
    crate::owned_adts_export::export_spool(spool, destination, transform, options)
}
