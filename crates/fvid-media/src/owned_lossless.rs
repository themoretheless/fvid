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
fn policy(o: &CopyOptions) -> bool {
    (o.streams.is_empty() || o.streams == [0])
        && o.max_packet_bytes != 0
        && o.max_controlled_bytes.is_none()
        && o.max_rss_bytes.is_none()
        && o.metadata_set.is_empty()
        && o.metadata_delete.is_empty()
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
    if !policy(options) {
        return Err("owned Y4M lossless export does not yet implement requested policy".into());
    }
    let request = request(&transform)
        .ok_or("owned Y4M lossless export does not yet implement requested transforms")?;
    let (stats, event) = crate::owned_matroska::export_y4m_ffv1_policy(
        source,
        destination,
        &request,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
    )
    .map_err(|e| e.to_string())?;
    Ok(LosslessStats {
        backend: "fvid",
        video_frames: stats.video_frames,
        decoded_frames: stats.video_frames,
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
