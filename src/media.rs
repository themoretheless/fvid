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


/// Geometry-only decoding uses owned codecs and sample planes. Remaining filter
/// operations retain their existing adapter until their native migration.
pub fn decode_video_transformed(source: &std::path::Path, transform: DecodeTransform) -> Result<DecodeStats> {
    let native = matches!(&transform, DecodeTransform {
        crop: _,
        vertical_flip: _,
        horizontal_flip: _,
        scale: _,
        epx: None,
        transpose: _,
        rotate: None,
        pad: _,
        burn_subs: None,
        overlay: None,
        yadif: None,
        bwdif: None,
        w3fdif: None,
        tblend: None,
        tmix: None,
        hqdn3d: None,
        gblur: None,
        eq: None,
        unsharp: None,
        hue: None,
        avgblur: None,
        boxblur: None,
        negate: None,
        edgedetect: None,
        sobel: None,
        prewitt: None,
        roberts: None,
        kirsch: None,
        scharr: None,
        atadenoise: None,
        owdenoise: None,
        vaguedenoiser: None,
        nlmeans: None,
        bm3d: None,
        dctdnoiz: None,
        fftdnoiz: None,
        smartblur: None,
        sab: None,
        bilateral: None,
        cas: None,
        vignette: None,
        curves: None,
        colorbalance: None,
        colorlevels: None,
        colorchannelmixer: None,
        deflicker: None,
        photosensitivity: None,
        monochrome: None,
        grayworld: None,
        drawbox: None,
        drawgrid: None,
        lagfun: None,
        amplify: None,
        bitplanenoise: None,
        deband: None,
        gradfun: None,
        lenscorrection: None,
        pixelize: None,
        removegrain: None,
        yaepblur: None,
        vibrance: None,
        dilation: None,
        erosion: None,
        colorize: None,
        exposure: None,
        chromashift: None,
        colorcontrast: None,
        colorcorrect: None,
        histeq: None,
        shuffleplanes: None,
        lutyuv: None,
        colorhold: None,
        fade: None,
        perspective: None,
        lumakey: None,
        chromakey: None,
        colorkey: None,
        despill: None,
        selectivecolor: None,
        stereo3d: None,
        field: None,
        hqx: None,
        xbr: None,
        il: None,
        super2xsai: None,
        kerndeint: None,
        phase: None,
        estdif: None,
        tinterlace: None,
        separatefields: None,
        weave: None,
        doubleweave: None,
        framepack: None,
        telecine: None,
        pullup: None,
        decimate: None,
        mpdecimate: None,
        framestep: None,
        tile: None,
        untile: None,
        shuffleframes: None,
        reverse: None,
        r#loop: None,
        thumbnail: None,
        freezedetect: None,
        pseudocolor: None,
        minterpolate: None,
        fps: None,
        colorspace: None,
        zscale: None,
        tonemap: None,
        pix_fmt: None,
        interval: _,
        input_format: None,
    });
    if !native { return fvid_media::decode_video_transformed(source, transform); }
    let interval = transform.interval.map(|(from, to)| {
        if from < 0 || to <= from { return Err("decode interval requires 0 <= from < to".to_owned()); }
        Ok((std::time::Duration::from_micros(from as u64), std::time::Duration::from_micros(to as u64)))
    }).transpose()?;
    let geometry = crate::native_geometry::VideoGeometry {
        crop: transform.crop.map(|r| [r.x, r.y, r.width, r.height]),
        horizontal_flip: transform.horizontal_flip,
        vertical_flip: transform.vertical_flip,
        scale: transform.scale.map(|r| [r.width as usize, r.height as usize]),
        transpose: transform.transpose.map(|r| crate::native_geometry::Transpose::parse(r.as_str())).transpose().map_err(|e| e.to_string())?,
        pad: transform.pad.map(|r| [r.width as usize, r.height as usize, r.x as usize, r.y as usize]),
    };
    let stats = crate::native_media::decode_video_transformed(source, interval, &geometry).map_err(|e| e.to_string())?;
    Ok(DecodeStats { backend: stats.backend, video_frames: stats.video_frames, width: stats.width,
        height: stats.height, pixel_format: stats.pixel_format, decode_errors: stats.decode_errors })
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
    let stats = crate::native_export::export_aac_pcm_selected(
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
    validate_native_copy_options(options, true)?;
    crate::native_plan::decode_audio_selected(source, transform, options.streams.first().copied())
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
    validate_native_copy_options(options, false)?;
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

/// Own parsers describe supported containers; unmigrated formats retain the
/// legacy adapter. A native parsing error never falls back to another parser.
pub fn probe(source: &std::path::Path) -> Result<MediaInfo> { probe_as(source, None) }
pub fn probe_as(source: &std::path::Path, format: Option<&str>) -> Result<MediaInfo> {
    match crate::native_probe::try_probe_as(source, format)? {
        Some(info) => Ok(info),
        None => fvid_media::probe_as(source, format),
    }
}
