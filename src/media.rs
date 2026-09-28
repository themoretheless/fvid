//! Media API migration: untransformed video decode uses FVid's native pipeline.
//! Remaining exports still use the legacy adapter and retain its dependencies.
pub use fvid_media::*;

pub fn decode_video(source: &std::path::Path) -> Result<DecodeStats> {
    let stats = crate::native_media::decode_video(source).map_err(|e| e.to_string())?;
    Ok(DecodeStats {
        backend: stats.backend,
        video_frames: stats.video_frames,
        width: stats.width,
        height: stats.height,
        pixel_format: stats.pixel_format,
        decode_errors: stats.decode_errors,
    })
}
