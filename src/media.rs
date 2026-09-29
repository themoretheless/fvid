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
    validate_native_copy_options(options)?;
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

fn validate_native_copy_options(options: &CopyOptions) -> Result<()> {
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
        return Err("native media operation does not yet support stream selection, custom budgets or metadata mutations".into());
    }
    Ok(())
}

/// Plan AAC decoding using owned container metadata and decoder configuration.
/// Packet contents and timeline consistency are checked during execution.
pub fn plan_decode_audio(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !crate::native_media::is_aac_source(source).map_err(|e| e.to_string())? {
        return fvid_media::plan_decode_audio(source, transform, options);
    }
    validate_native_copy_options(options)?;
    if transform.interval.is_some_and(|(from, to)| from < 0 || to <= from) {
        return Err("decode-audio interval requires 0 <= from < to".into());
    }
    if transform.sample_rate.is_some_and(|rate| !(8000..=384000).contains(&rate)) {
        return Err("sample rate must be within 8000..=384000".into());
    }
    if transform.volume.is_some_and(|gain| !gain.is_finite() || !(0.0..=64.0).contains(&gain)) {
        return Err("volume must be a finite linear gain within 0..=64".into());
    }
    let info = crate::native_media::aac_source_info(source).map_err(|e| e.to_string())?;
    let channels = transform.channels.unwrap_or(i32::from(info.channels));
    if channels != i32::from(info.channels) && !matches!(channels, 1 | 2) {
        return Err("native AAC channel conversion supports mono or stereo output".into());
    }
    let rate = transform.sample_rate.unwrap_or(info.sample_rate as i32);
    let mut steps = vec![PlanStep {
        action: "decode".into(), detail: "FVid owned AAC-LC decoder to interleaved float PCM".into(),
    }];
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep { action: "trim".into(),
            detail: format!("presentation window [{from}, {to}) µs; round up to source sample grid; decode pre-roll") });
    }
    if channels != i32::from(info.channels) {
        steps.push(PlanStep { action: "rematrix".into(),
            detail: format!("FVid channel conversion {} → {channels}; center/surround -3 dB, omit LFE when downmixing", info.channels) });
    }
    if let Some(gain) = transform.volume.filter(|&gain| gain != 1.0) {
        steps.push(PlanStep { action: "volume".into(), detail: format!("unclipped linear gain {gain} after channel conversion") });
    }
    if rate != info.sample_rate as i32 {
        steps.push(PlanStep { action: "resample".into(),
            detail: format!("FVid windowed-sinc resampler {} Hz → {rate} Hz", info.sample_rate) });
    }
    steps.push(PlanStep { action: "write".into(), detail: "atomic float32 PCM export to .f32le or .wav; never overwrite an existing destination".into() });
    Ok(MediaPlan {
        command: "decode-audio".into(), input: source.to_path_buf(), inputs: vec![source.to_path_buf()],
        streams: vec![PlanStream { index: info.stream_index, media_type: "audio".into(),
            codec: "aac".into(), disposition: "decode".into() }],
        steps, graph: None,
        notes: vec![
            "backend: fvid; no external codec or resampler".into(),
            "MP4 edits and Matroska trim metadata apply; ADTS retains encoder priming".into(),
            "metadata-only plan: packet contents and timeline consistency are verified during execution".into(),
        ],
    })
}

/// Native ADTS-to-MP4 muxing and MP4 fast-start relocation. Other container
/// conversions retain the legacy adapter until their owned muxers are available.
/// For opaque MP4 relocation, packets=0 means uncounted; payload_bytes counts
/// mdat bytes. fvid_payload_copies counts additional per-packet payload clones,
/// not file I/O buffering (MP4 relocation does not create packet objects).
pub fn remux(
    source: &std::path::Path,
    destination: &std::path::Path,
    options: &CopyOptions,
) -> Result<CopyStats> {
    use std::io::Read;
    if !matches!(destination.extension().and_then(|s| s.to_str()), Some("mp4" | "m4a")) {
        return fvid_media::remux(source, destination, options);
    }
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let adts = crate::container::adts::header(&prefix).is_some();
    let mp4 = &prefix[4..8] == b"ftyp";
    if !adts && !mp4 { return fvid_media::remux(source, destination, options); }
    validate_native_copy_options(options)?;
    let stats = if adts {
        crate::native_export::remux_adts_aac_stats(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    } else {
        crate::native_export::remux_mp4_stats(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    }.map_err(|e| e.to_string())?;
    Ok(CopyStats { packets: stats.packets, payload_bytes: stats.payload_bytes,
        segments: 1, backend: "fvid", fvid_payload_copies: 0 })
}
