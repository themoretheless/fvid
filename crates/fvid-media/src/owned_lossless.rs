//! Owned lossless export admission and public API.
use fvid_control::CopyOptions;
use fvid_media_info::{DecodeTransform, LosslessStats, LosslessTransform};
use std::path::Path;
fn request(t: &LosslessTransform) -> Option<DecodeTransform> {
    let accepted = LosslessTransform {
        crop: t.crop.clone(),
        vertical_flip: t.vertical_flip.clone(),
        horizontal_flip: t.horizontal_flip.clone(),
        scale: t.scale.clone(),
        transpose: t.transpose.clone(),
        pad: t.pad.clone(),
        interval: t.interval,
        avgblur: t.avgblur.clone(),
        boxblur: t.boxblur.clone(),
        negate: t.negate.clone(),
        sobel: t.sobel.clone(),
        prewitt: t.prewitt.clone(),
        roberts: t.roberts.clone(),
        kirsch: t.kirsch.clone(),
        scharr: t.scharr.clone(),
        pixelize: t.pixelize.clone(),
        chromashift: t.chromashift.clone(),
        dilation: t.dilation.clone(),
        erosion: t.erosion.clone(),
        shuffleplanes: t.shuffleplanes.clone(),
        ..Default::default()
    };
    if *t != accepted {
        return None;
    }
    Some(DecodeTransform {
        crop: t.crop.clone(),
        vertical_flip: t.vertical_flip.clone(),
        horizontal_flip: t.horizontal_flip.clone(),
        scale: t.scale.clone(),
        transpose: t.transpose.clone(),
        pad: t.pad.clone(),
        interval: t.interval,
        avgblur: t.avgblur.clone(),
        boxblur: t.boxblur.clone(),
        negate: t.negate.clone(),
        sobel: t.sobel.clone(),
        prewitt: t.prewitt.clone(),
        roberts: t.roberts.clone(),
        kirsch: t.kirsch.clone(),
        scharr: t.scharr.clone(),
        pixelize: t.pixelize.clone(),
        chromashift: t.chromashift.clone(),
        dilation: t.dilation.clone(),
        erosion: t.erosion.clone(),
        shuffleplanes: t.shuffleplanes.clone(),
        ..Default::default()
    })
}
fn metadata(o: &CopyOptions) -> Result<crate::owned_matroska::FileMetadata, String> {
    if o.metadata_set.len() + o.metadata_delete.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    let mut file = crate::owned_matroska::FileMetadata::default();
    for key in &o.metadata_delete {
        if key.contains('\0') || !file.tags.set(key, "") {
            return Err("unsupported owned container metadata key".into());
        }
    }
    for (key, value) in &o.metadata_set {
        if key.contains('\0') || value.contains('\0') {
            return Err("NUL in container metadata".into());
        }
        if !file.tags.set(key, value) {
            return Err("unsupported owned container metadata key".into());
        }
    }
    Ok(file)
}
fn policy(o: &CopyOptions) -> bool {
    (o.streams.is_empty() || o.streams == [0])
        && o.max_packet_bytes != 0
        && o.max_controlled_bytes.is_none()
        && o.max_rss_bytes.is_none()
        && o.metadata_set.len() + o.metadata_delete.len() <= 64
        && o.metadata_set.iter().all(|(k, v)| {
            !k.contains('\0')
                && !v.contains('\0')
                && crate::owned_file_tags::FileTags::supports_key(k)
        })
        && o.metadata_delete
            .iter()
            .all(|k| !k.contains('\0') && crate::owned_file_tags::FileTags::supports_key(k))
        && o.stream_metadata_set.is_empty()
        && o.stream_metadata_delete.is_empty()
}
pub(crate) fn supports(source: &Path, t: &LosslessTransform, o: &CopyOptions) -> bool {
    let mut reader = match std::fs::File::open(source) {
        Ok(file) => std::io::BufReader::new(file),
        Err(_) => return false,
    };
    let mut line = Vec::new();
    if !crate::owned_y4m::line(&mut reader, &mut line).is_ok_and(|present| present) {
        return false;
    }
    let Ok(header) = crate::owned_y4m::Header::parse(&line) else {
        return false;
    };
    // Keep chroma-siting aliases on their previous backend until mapped.
    if header
        .tokens
        .iter()
        .any(|t| matches!(t.as_str(), "C420mpeg2" | "C420paldv"))
    {
        return false;
    }
    policy(o)
        && request(t).is_some_and(|r| crate::owned_y4m_decode::supports_transformed(source, &r))
}
pub fn transcode_lossless(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    let file_metadata = metadata(options)?;
    if !policy(options) {
        return Err("owned Y4M lossless export does not yet implement requested policy".into());
    }
    let request = request(&transform)
        .ok_or("owned Y4M lossless export does not yet implement requested transforms")?;
    let skipped = if let Some((from, to)) = request.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        let mut reader =
            std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        crate::owned_y4m::line(&mut reader, &mut line)?;
        let header = crate::owned_y4m::Header::parse(&line)?;
        let [n, d] = header.frame_rate()?;
        let denominator = d as u128 * 1_000_000;
        for time in [from, to] {
            if time as u128 * n as u128 % denominator != 0 {
                return Err("interval boundary is not exact in video time base".into());
            }
        }
        u64::try_from(from as u128 * n as u128 / denominator)
            .map_err(|_| "interval timestamp overflow")?
    } else {
        0
    };
    let (stats, event) = crate::owned_matroska::export_y4m_ffv1_policy(
        source,
        destination,
        &request,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        true,
        &file_metadata,
    )
    .map_err(|e| e.to_string())?;
    Ok(LosslessStats {
        backend: "fvid",
        video_frames: stats.video_frames,
        decoded_frames: stats
            .video_frames
            .checked_add(skipped)
            .ok_or("frame count overflow")?,
        seek_used: false,
        video_packets: event.packets,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: stats.pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: transform.vertical_flip,
        horizontal_flip: transform.horizontal_flip,
    })
}

/// Preserve the library crop API without a libav decoder or encoder.
pub fn crop_lossless(
    source: &Path,
    destination: &Path,
    crop: fvid_media_info::CropRect,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            crop: Some(crop),
            ..Default::default()
        },
        options,
    )
}
