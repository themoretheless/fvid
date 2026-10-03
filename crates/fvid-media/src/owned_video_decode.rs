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
    let mut frame_transform = transform.clone();
    frame_transform.input_format = None;
    frame_transform.interval = None;
    frame_transform.framestep = None;
    if !crate::owned_y4m_decode::supported_request(&frame_transform)
        || frame_transform.overlay.is_some()
        || frame_transform.reverse.is_some()
        || frame_transform.shuffleframes.is_some()
        || !transform
            .input_format
            .as_deref()
            .is_none_or(|f| matches!(f, "matroska" | "webm"))
    {
        return Ok(None);
    }
    let step = match crate::owned_framestep::FrameStep::parse(
        transform.framestep.as_deref().unwrap_or(""),
    ) {
        Ok(step) => step,
        Err(_) => return Ok(None),
    };
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
    let full_range = track.colour.full_range;
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
    let mut selected_inputs = 0u64;
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
        let emit = visible && selected && !past_end && step.emits(selected_inputs);
        if visible && selected && !past_end {
            selected_inputs = selected_inputs
                .checked_add(1)
                .ok_or("FFV1 frame count overflow")?;
        }
        if frame_transform == DecodeTransform::default() {
            stats.pixel_format = if decoded.depth == 8 {
                base.into()
            } else {
                format!("{base}{}le", decoded.depth)
            };
        } else if emit || stats.pixel_format.is_empty() {
            let Some((width, height, format, pixels)) = process_frame(
                &decoded,
                decoder.monochrome() == Some(true),
                full_range,
                &frame_transform,
            )?
            else {
                return Ok(None);
            };
            stats.width = width;
            stats.height = height;
            stats.pixel_format = format;
            std::hint::black_box(pixels);
        }
        if past_end {
            break;
        }
        if emit {
            stats.video_frames = stats
                .video_frames
                .checked_add(1)
                .ok_or("FFV1 frame count overflow")?;
        }
    }
    Ok(Some(stats))
}

fn process_frame(
    decoded: &crate::owned_ffv1_decoder::Decoded,
    monochrome: bool,
    full_range: bool,
    transform: &DecodeTransform,
) -> Result<Option<(u32, u32, String, Vec<u8>)>> {
    use crate::owned_y4m::{Header, PixelFormat};
    let (format, layout) = match decoded.frame.subsampling {
        Some([1, 1]) => (PixelFormat::Yuv444, "444"),
        Some([2, 1]) => (PixelFormat::Yuv422, "422"),
        Some([2, 2]) => (PixelFormat::Yuv420, "420"),
        Some([1, 2]) => (PixelFormat::Yuv440, "440"),
        Some([4, 1]) => (PixelFormat::Yuv411, "411"),
        Some([4, 4]) => (PixelFormat::Yuv410, "410"),
        _ => return Ok(None),
    };
    // These source layouts cannot be transposed without a chroma conversion.
    if transform.transpose.is_some() && format == PixelFormat::Yuv411 {
        return Ok(None);
    }
    let chroma = if decoded.depth == 8 {
        layout.into()
    } else {
        format!("{layout}p{}", decoded.depth)
    };
    let header = Header {
        width: decoded.frame.width,
        height: decoded.frame.height,
        format,
        tokens: vec![
            format!("C{chroma}"),
            format!(
                "XCOLORRANGE={}",
                if full_range || monochrome {
                    "FULL"
                } else {
                    "LIMITED"
                }
            ),
        ],
    };
    let (crop, _) = crate::owned_y4m_decode::crop_geometry(&header, transform.crop)?;
    let (width, height, _) = crate::owned_y4m_decode::output_geometry(
        &header,
        crop,
        transform.scale,
        transform.transpose,
        transform.pad,
    )?;
    let promote = transform
        .shuffleplanes
        .as_deref()
        .map(crate::owned_shuffleplanes::ShufflePlanes::parse)
        .transpose()?
        .is_some_and(|f| f.mapping[0] != 0 || f.mapping[1] == 0 || f.mapping[2] == 0);
    let layout = if promote {
        "444"
    } else if transform.transpose.is_some() {
        match layout {
            "422" => "440",
            "440" => "422",
            layout => layout,
        }
    } else {
        layout
    };
    let base = if monochrome && transform.shuffleplanes.is_none() {
        "gray".into()
    } else {
        format!("yuv{layout}p")
    };
    let name = if decoded.depth == 8 {
        base
    } else {
        format!("{base}{}le", decoded.depth)
    };
    let pixels = crate::owned_y4m_decode::transform_frame_requested(
        &header,
        &decoded.frame.data,
        transform,
    )?;
    Ok(Some((
        u32::try_from(width).map_err(|_| "FFV1 output width overflow")?,
        u32::try_from(height).map_err(|_| "FFV1 output height overflow")?,
        name,
        pixels,
    )))
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
                    overlay: Some(Default::default()),
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn compressed_frames_use_owned_filters_with_exact_luma_and_source_selection() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for depth in [8u8, 10, 16] {
            let packet = std::fs::read(root.join(format!("ffv1-gray-{depth}-0.packet"))).unwrap();
            let raw = std::fs::read(root.join(format!("ffv1-gray-{depth}-0.gray"))).unwrap();
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 3, usize::MAX).unwrap();
            let frame = decoder.decode(&packet).unwrap();
            let request = DecodeTransform {
                horizontal_flip: true,
                vertical_flip: true,
                negate: Some(String::new()),
                ..Default::default()
            };
            let (w, h, format, pixels) = process_frame(&frame, true, false, &request)
                .unwrap()
                .unwrap();
            assert_eq!((w, h), (4, 3));
            assert_eq!(
                format,
                if depth == 8 {
                    "gray".into()
                } else {
                    format!("gray{depth}le")
                }
            );
            let bytes = if depth == 8 { 1 } else { 2 };
            let maximum = (1u32 << depth) - 1;
            let expected: Vec<u8> = raw
                .chunks_exact(bytes)
                .rev()
                .flat_map(|s| {
                    let value = if bytes == 1 {
                        u32::from(s[0])
                    } else {
                        u32::from(u16::from_le_bytes([s[0], s[1]]))
                    };
                    ((maximum - value) as u16).to_le_bytes()[..bytes].to_vec()
                })
                .collect();
            assert_eq!(&pixels[..raw.len()], expected);
            let (_, _, _, padded) = process_frame(
                &frame,
                true,
                false,
                &DecodeTransform {
                    pad: Some(fvid_media_info::PadRect {
                        width: 8,
                        height: 6,
                        x: 2,
                        y: 2,
                    }),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
            // Gray storage uses black zero at every depth, even without a container range tag.
            assert!(padded[..8 * bytes].iter().all(|&v| v == 0));
            assert_eq!(
                &padded[(2 * 8 + 2) * bytes..(2 * 8 + 6) * bytes],
                &raw[..4 * bytes]
            );

            let stats = crate::decode_video_transformed(
                &root.join(format!("ffv1-gray-{depth}.mkv")),
                DecodeTransform {
                    scale: Some(fvid_media_info::ScaleSize {
                        width: 8,
                        height: 6,
                    }),
                    framestep: Some("2".into()),
                    ..request
                },
            )
            .unwrap();
            assert_eq!(stats.backend, "owned Matroska FFV1 decode");
            assert_eq!((stats.width, stats.height, stats.video_frames), (8, 6, 1));
        }
        let stats = crate::decode_video_transformed(
            &root.join("ffv1-positive-start.mkv"),
            DecodeTransform {
                interval: Some((40_000, 80_000)),
                framestep: Some("2".into()),
                horizontal_flip: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.video_frames, 1);
    }
}
