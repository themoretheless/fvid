//! Media API migration: plain video decode and AAC PCM export use FVid's native pipeline.
//! Remaining exports still use the legacy adapter and retain its dependencies.
pub use fvid_media::*;

pub fn decode_video(source: &std::path::Path) -> Result<DecodeStats> {
    decode_video_interval(source, None)
}

/// Native half-open presentation interval; pre-roll reference frames are not counted.
pub fn decode_video_interval(
    source: &std::path::Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<DecodeStats> {
    let stats =
        crate::native_media::decode_video_interval(source, interval).map_err(|e| e.to_string())?;
    Ok(DecodeStats {
        backend: stats.backend,
        video_frames: stats.video_frames,
        width: stats.width,
        height: stats.height,
        pixel_format: stats.pixel_format,
        decode_errors: stats.decode_errors,
    })
}

/// Export AAC through the owned decoder and PCM writer.
/// Other codecs retain the legacy adapter until their migration is complete.
pub fn decode_audio(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(source, destination, AudioDecodeTransform::default(), options)
}

pub fn decode_audio_interval(
    source: &std::path::Path,
    destination: &std::path::Path,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    decode_audio_transformed(source, destination, AudioDecodeTransform {
        interval, ..Default::default()
    }, options)
}

/// AAC exports use native float PCM, including source edit-list trimming.
/// Explicit options without a native implementation are rejected, never ignored.
pub fn decode_audio_transformed(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !crate::native_media::is_aac_source(source).map_err(|e| e.to_string())? {
        return fvid_media::decode_audio_transformed(source, destination, transform, options);
    }
    if !options.streams.is_empty()
        || options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err("native AAC export does not yet support stream selection, custom budgets or metadata mutations".into());
    }
    let interval = transform.interval.map(|(from, to)| {
        if from < 0 || to <= from {
            return Err("decode-audio interval requires 0 <= from < to".to_owned());
        }
        Ok((std::time::Duration::from_micros(from as u64),
            std::time::Duration::from_micros(to as u64)))
    }).transpose()?;
    let channels = transform.channels.map(|n| u16::try_from(n)
        .map_err(|_| "invalid channel count".to_owned())).transpose()?;
    let sample_rate = transform.sample_rate.map(|n| u32::try_from(n)
        .map_err(|_| "invalid sample rate".to_owned())).transpose()?;
    let stats = crate::native_export::export_aac_pcm_controlled(
        source, destination, interval, transform.volume.unwrap_or(1.0),
        channels, sample_rate, options.cancel.as_ref(), options.progress.as_ref(),
    ).map_err(|e| e.to_string())?;
    Ok(AudioDecodeStats {
        sample_frames: stats.sample_frames,
        decoded_frames: stats.decoded_frames,
        sample_rate: stats.sample_rate as i32,
        channels: i32::from(stats.channels),
        sample_format: "flt".into(),
        planar_interleave_bytes: 0,
        decode_errors: 0,
    })
}
