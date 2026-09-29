//! Media API migration: plain video decode and AAC PCM export use FVid's native pipeline.
//! Remaining exports still use the legacy adapter and retain its dependencies.
pub use fvid_media::*;
pub use crate::native_export::{export_y4m, export_y4m_interval, export_y4m_transformed};

pub fn decode_video(source: &std::path::Path) -> Result<DecodeStats> {
    decode_video_interval(source, None)
}

/// Native half-open presentation interval; pre-roll reference frames are not counted.
pub fn decode_video_interval(
    source: &std::path::Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<DecodeStats> {
    crate::native_media::decode_video_interval(source, interval).map_err(|e| e.to_string())
}


/// Geometry-only decoding uses owned codecs and sample planes. Remaining filter
/// operations retain their existing adapter until their native migration.
pub fn decode_video_transformed(source: &std::path::Path, transform: DecodeTransform) -> Result<DecodeStats> {
    if !crate::native_media::supports_video_request(&transform) {
        return fvid_media::decode_video_transformed(source, transform);
    }
    crate::native_media::decode_video_request(source, &transform).map_err(|e|e.to_string())
}

/// Export AAC or packed WAVE PCM through the owned audio pipeline.
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

/// AAC and WAVE exports use native float PCM, including source edit-list trimming.
/// Explicit options without a native implementation are rejected, never ignored.
pub fn decode_audio_transformed(
    source: &std::path::Path,
    destination: &std::path::Path,
    transform: AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<AudioDecodeStats> {
    if !crate::native_media::is_aac_source(source).map_err(|e| e.to_string())? && !crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        return fvid_media::decode_audio_transformed(source, destination, transform, options);
    }
    validate_native_copy_options(options, true)?;
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
    let stats = crate::native_export::export_audio_pcm_selected(
        source, destination, interval, transform.volume.unwrap_or(1.0),
        channels, sample_rate, options.streams.first().copied(), options.cancel.as_ref(), options.progress.as_ref(),
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

fn validate_native_copy_options(options: &CopyOptions, audio_selection: bool) -> Result<()> {
    if (if audio_selection { options.streams.len() > 1 } else { !options.streams.is_empty() })
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

/// Plan AAC or WAVE decoding using owned metadata and decoder configuration.
/// Packet contents and timeline consistency are checked during execution.
pub fn plan_decode_audio(
    source: &std::path::Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !crate::native_media::is_aac_source(source).map_err(|e| e.to_string())? && !crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        return fvid_media::plan_decode_audio(source, transform, options);
    }
    validate_native_copy_options(options, true)?;
    crate::native_plan::decode_audio_selected(source, transform, options.streams.first().copied())
}

/// Native ADTS-to-MP4/Matroska muxing and MP4 fast-start relocation. Other container
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
    if !matches!(destination.extension().and_then(|s| s.to_str()), Some("mp4" | "m4a" | "mka" | "mkv")) {
        return fvid_media::remux(source, destination, options);
    }
    let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let adts = crate::container::adts::header(&prefix).is_some();
    let mp4 = &prefix[4..8] == b"ftyp";
    let matroska = matches!(destination.extension().and_then(|s|s.to_str()),Some("mka"|"mkv"));
    let mp4_matroska = mp4 && matroska && crate::native_export::is_native_mp4_matroska(source).map_err(|e| e.to_string())?;
    if !adts && !mp4_matroska && (matroska || !mp4) { return fvid_media::remux(source, destination, options); }
    // Preserve unmigrated metadata mutations/stream options on the legacy path.
    if mp4_matroska && validate_native_copy_options(options, false).is_err() {
        return fvid_media::remux(source, destination, options);
    }
    validate_native_copy_options(options, false)?;
    let stats = if mp4_matroska {
        crate::native_export::remux_mp4_matroska(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    } else if adts {
        crate::native_export::remux_adts_aac_stats(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    } else {
        crate::native_export::remux_mp4_stats(source, destination,
            options.cancel.as_ref(), options.progress.as_ref())
    }.map_err(|e| e.to_string())?;
    Ok(CopyStats { packets: stats.packets, payload_bytes: stats.payload_bytes,
        segments: 1, backend: "fvid", fvid_payload_copies: 0 })
}

/// Own parsers describe supported containers; unmigrated formats retain the
/// legacy adapter. A native parsing error never falls back to another parser.
pub fn probe(source: &std::path::Path) -> Result<MediaInfo> { probe_as(source, None) }
pub fn probe_as(source: &std::path::Path, format: Option<&str>) -> Result<MediaInfo> {
    match crate::native_probe::try_probe_as(source, format)? {
        Some(info) => Ok(info),
        None => fvid_media::probe_as(source, format),
    }
}

/// Sample-exact RIFF PCM uses the owned streaming slicer and publisher.
pub fn trim_pcm(source:&std::path::Path,destination:&std::path::Path,from:i64,to:i64,options:&CopyOptions)->Result<PcmTrimStats> {
    if !crate::native_pcm::is_wave(source).map_err(|e|e.to_string())? {
        return fvid_media::trim_pcm(source,destination,from,to,options);
    }
    validate_native_copy_options(options,true)?;
    if options.streams.first().is_some_and(|&n|n!=0) {return Err("WAVE has only stream 0".into());}
    crate::native_pcm::trim_wave(source,destination,from,to,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())
}

/// Plan owned RIFF PCM slicing; other containers retain the legacy adapter.
pub fn plan_trim_pcm(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return fvid_media::plan_trim_pcm(source, from, to, options);
    }
    validate_native_copy_options(options, true)?;
    crate::native_plan::trim_pcm(source, from, to, options.streams.first().copied())
}

/// General trim of a PCM WAVE file shares the exact native sample-slicing path.
pub fn trim(
    source: &std::path::Path,
    destination: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<CopyStats> {
    if destination.extension().and_then(|s|s.to_str())==Some("y4m") {
        validate_native_copy_options(options,true)?;
        let stats=crate::native_export::trim_y4m(source,destination,from,to,options.streams.first().copied(),options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:1,backend:"fvid",fvid_payload_copies:0});
    }
    if crate::native_media::is_aac_source(source).map_err(|e|e.to_string())?
        && destination.extension().and_then(|s|s.to_str())==Some("wav") {
        validate_native_copy_options(options,true)?;
        let stats=crate::native_export::trim_aac_wave(source,destination,from,to,options.streams.first().copied(),options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.decoded_frames,payload_bytes:stats.sample_frames*u64::from(stats.channels)*4,segments:1,backend:"fvid",fvid_payload_copies:0});
    }
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return fvid_media::trim(source, destination, from, to, options);
    }
    let stats = trim_pcm(source, destination, from, to, options)?;
    Ok(CopyStats {
        packets: stats.packets,
        payload_bytes: stats.payload_bytes,
        segments: 1,
        backend: "fvid",
        fvid_payload_copies: stats.fvid_payload_copies,
    })
}
pub fn plan_trim(
    source: &std::path::Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if crate::native_media::is_aac_source(source).map_err(|e|e.to_string())? {
        validate_native_copy_options(options,true)?;
        return crate::native_plan::trim_aac(source,from,to,options.streams.first().copied());
    }
    if !crate::native_pcm::is_wave(source).map_err(|e| e.to_string())? {
        return fvid_media::plan_trim(source, from, to, options);
    }
    let mut plan = plan_trim_pcm(source, from, to, options)?;
    plan.command = "trim".into();
    Ok(plan)
}

/// Plan owned ADTS/MP4 remuxing with the same option restrictions as execution.
pub fn plan_remux(source: &std::path::Path, options: &CopyOptions) -> Result<MediaPlan> {
    match crate::native_plan::remux(source)? {
        Some(plan) => { validate_native_copy_options(options, false)?; Ok(plan) }
        None => fvid_media::plan_remux(source, options),
    }
}

/// Concatenate compatible packed WAVE PCM through the owned streaming writer.
pub fn concat(sources: &[std::path::PathBuf], destination: &std::path::Path, options: &CopyOptions) -> Result<CopyStats> {
    if destination.extension().and_then(|s|s.to_str())==Some("y4m") {
        validate_native_copy_options(options,true)?;
        let stats=crate::native_export::concat_y4m(sources,destination,options.streams.first().copied(),options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0});
    }
    if sources.first().map(|p|crate::native_export::is_adts_source(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        validate_native_copy_options(options,true)?;
        if options.streams.first().is_some_and(|&s|s!=0) {return Err("ADTS has only stream 0".into());}
        let stats=crate::native_export::concat_adts_aac(sources,destination,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
        return Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0});
    }
    if !sources.first().map(|p|crate::native_pcm::is_wave(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        return fvid_media::concat(sources,destination,options);
    }
    validate_native_copy_options(options,true)?;
    if options.streams.first().is_some_and(|&s|s!=0) {return Err("WAVE has only stream 0".into());}
    let stats=crate::native_pcm::concat_wave(sources,destination,options.cancel.as_ref(),options.progress.as_ref()).map_err(|e|e.to_string())?;
    Ok(CopyStats {packets:stats.packets,payload_bytes:stats.payload_bytes,segments:sources.len(),backend:"fvid",fvid_payload_copies:0})
}

pub fn plan_concat(sources: &[std::path::PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    if sources.first().map(|p|crate::native_export::is_adts_source(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        validate_native_copy_options(options,true)?;
        if options.streams.first().is_some_and(|&s|s!=0) {return Err("ADTS has only stream 0".into());}
        return crate::native_plan::concat_adts(sources);
    }
    if !sources.first().map(|p|crate::native_pcm::is_wave(p)).transpose().map_err(|e|e.to_string())?.unwrap_or(false) {
        return fvid_media::plan_concat(sources,options);
    }
    validate_native_copy_options(options,true)?;
    if options.streams.first().is_some_and(|&s|s!=0) {return Err("WAVE has only stream 0".into());}
    crate::native_plan::concat_wave(sources)
}

/// Plan decoded video concatenation with an explicit Y4M output contract.
pub fn plan_concat_y4m(sources: &[std::path::PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    validate_native_copy_options(options,true)?;
    crate::native_plan::concat_y4m(sources,options.streams.first().copied())
}

/// Plan an explicitly requested decoded Y4M video trim.
pub fn plan_trim_y4m(source: &std::path::Path, from: i64, to: i64, options: &CopyOptions) -> Result<MediaPlan> {
    validate_native_copy_options(options,true)?;
    crate::native_plan::trim_y4m(source,from,to,options.streams.first().copied())
}
