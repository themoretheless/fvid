//! Public decode dispatch using FVid-owned container and codec implementations.
use fvid_media_info::{DecodeStats, DecodeTransform};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_video_transformed(source, DecodeTransform::default())
}
pub fn decode_video_transformed(source: &Path, transform: DecodeTransform) -> Result<DecodeStats> {
    if crate::owned_y4m_decode::supports_transformed(source, &transform) {
        return crate::owned_y4m_decode::decode_video_transformed(source, transform);
    }
    try_ffv1(source, &transform)?.ok_or_else(|| {
        "owned video decode does not yet support this container, codec or transform".into()
    })
}

/// No output is published while qualifying an unsupported stream for legacy callers.
/// Corrupt packets propagate errors; only explicit capability refusals permit fallback.
pub(crate) fn try_ffv1(source: &Path, transform: &DecodeTransform) -> Result<Option<DecodeStats>> {
    if *transform
        != (DecodeTransform {
            interval: transform.interval,
            input_format: transform.input_format.clone(),
            ..Default::default()
        })
        || !transform
            .input_format
            .as_deref()
            .is_none_or(|f| matches!(f, "matroska" | "webm"))
    {
        return Ok(None);
    }
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode interval requires 0 <= from < to".into());
    }
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut signature = [0; 4];
    if file.read(&mut signature).map_err(|e| e.to_string())? != 4
        || signature != [0x1a, 0x45, 0xdf, 0xa3]
    {
        return Ok(None);
    }
    let mut reader = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    reader.scan_all().map_err(|e| e.to_string())?;
    let Some(track) = reader.tracks.iter().find(|t| t.kind == 1) else {
        return Err("input has no video stream".into());
    };
    if track.codec != "V_FFV1"
        || !track.codec_private.is_empty()
        || track.crop != [0; 4]
        || track.rotation != 0
    {
        return Ok(None);
    }
    let number = track.number;
    let width = u32::try_from(track.width).map_err(|_| "FFV1 width exceeds API range")?;
    let height = u32::try_from(track.height).map_err(|_| "FFV1 height exceeds API range")?;
    // No implicit policy ceiling: allocation sizes are checked by the decoder.
    let mut decoder =
        crate::owned_ffv1_decoder::Decoder::new(width as usize, height as usize, usize::MAX)?;
    let origin = reader.packets.iter().map(|p| p.pts_ns).min().unwrap_or(0);
    let mut stats = DecodeStats {
        backend: "owned Matroska FFV1 decode",
        video_frames: 0,
        width,
        height,
        pixel_format: String::new(),
        decode_errors: 0,
    };
    for index in 0..reader.packets.len() {
        let p = &reader.packets[index];
        if p.track != number {
            continue;
        }
        let time = i128::from(p.pts_ns) - i128::from(origin);
        let past_end = transform
            .interval
            .is_some_and(|(_, to)| time >= i128::from(to) * 1000);
        if past_end && !stats.pixel_format.is_empty() {
            break;
        }
        let visible = !p.invisible;
        let selected = transform
            .interval
            .is_none_or(|(from, _)| time >= i128::from(from) * 1000);
        let packet = reader.read_packet(index).map_err(|e| e.to_string())?;
        let decoded = match decoder.decode(&packet) {
            Ok(frame) => frame,
            Err(error) if error.starts_with(crate::owned_ffv1_decoder::UNSUPPORTED_PREFIX) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let base = if decoder.monochrome() == Some(true) {
            "gray"
        } else {
            match decoded.frame.subsampling {
                Some([1, 1]) => "yuv444p",
                Some([2, 1]) => "yuv422p",
                Some([2, 2]) => "yuv420p",
                Some([1, 2]) => "yuv440p",
                Some([4, 1]) => "yuv411p",
                Some([4, 4]) => "yuv410p",
                _ => return Ok(None),
            }
        };
        stats.pixel_format = if decoded.depth == 8 {
            base.into()
        } else {
            format!("{base}{}le", decoded.depth)
        };
        if past_end {
            break;
        }
        if visible && selected {
            stats.video_frames = stats
                .video_frames
                .checked_add(1)
                .ok_or("FFV1 frame count overflow")?;
        }
    }
    Ok(Some(stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_matroska_ffv1_decode_and_interval_are_owned() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for depth in [8, 10, 16] {
            let source = root.join(format!("ffv1-gray-{depth}.mkv"));
            let full = crate::decode_video(&source).unwrap();
            assert_eq!(full.backend, "owned Matroska FFV1 decode");
            assert_eq!((full.width, full.height, full.video_frames), (4, 3, 2));
            assert_eq!(
                full.pixel_format,
                if depth == 8 {
                    "gray".into()
                } else {
                    format!("gray{depth}le")
                }
            );
            let selected = crate::decode_video_transformed(
                &source,
                DecodeTransform {
                    interval: Some((40_000, 80_000)),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(selected.video_frames, 1);
        }
    }
    #[test]
    fn positive_start_uses_container_origin_and_damage_is_not_a_capability_refusal() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let selected = crate::decode_video_transformed(
            &root.join("ffv1-positive-start.mkv"),
            DecodeTransform {
                interval: Some((40_000, 80_000)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(selected.video_frames, 1);
        assert_eq!(
            try_ffv1(
                &root.join("ffv1-invalid-range-header.mkv"),
                &Default::default()
            )
            .unwrap_err(),
            "invalid FFV1 range header"
        );
        assert!(
            try_ffv1(
                &root.join("ffv1-gray-8.mkv"),
                &DecodeTransform {
                    horizontal_flip: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
        );
    }
}
