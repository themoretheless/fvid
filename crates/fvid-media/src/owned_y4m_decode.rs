//! Owned raw planar Y4M decode-and-discard, with bounded scratch storage.
use crate::owned_y4m::{Header, PixelFormat, line};
use fvid_media_info::{CropRect, DecodeStats, DecodeTransform, ScaleSize};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

/// Recognize only the progressive planar grammar owned by FVid. Legacy callers
/// retain their existing route for unsupported interlace/chroma configurations.
pub(crate) fn supports(source: &Path) -> bool {
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    line(&mut reader, &mut bytes).is_ok_and(|present| present) && Header::parse(&bytes).is_ok()
}

/// Consume every raw sample byte rather than treating a metadata-only seek as
/// decoding. No RGB conversion, external codec, or frame-sized allocation.
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_reader(BufReader::new(
        File::open(source).map_err(|e| e.to_string())?,
    ))
}
pub fn decode_reader(source: impl BufRead) -> Result<DecodeStats> {
    decode_reader_transformed(source, &Default::default())
}
fn supported_request(transform: &DecodeTransform) -> bool {
    transform
        .input_format
        .as_deref()
        .is_none_or(|format| matches!(format, "y4m" | "yuv4mpegpipe"))
        && *transform
            == DecodeTransform {
                crop: transform.crop,
                scale: transform.scale,
                horizontal_flip: transform.horizontal_flip,
                vertical_flip: transform.vertical_flip,
                interval: transform.interval,
                input_format: transform.input_format.clone(),
                ..Default::default()
            }
}
pub(crate) fn supports_transformed(source: &Path, transform: &DecodeTransform) -> bool {
    supported_request(transform) && supports(source)
}
pub fn decode_video_transformed(source: &Path, transform: DecodeTransform) -> Result<DecodeStats> {
    decode_reader_transformed(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        &transform,
    )
}
fn crop_geometry(header: &Header, crop: Option<CropRect>) -> Result<(CropRect, usize)> {
    let crop = crop.unwrap_or(CropRect {
        x: 0,
        y: 0,
        width: header.width,
        height: header.height,
    });
    let (sx, sy) = header.format.subsampling();
    if crop.width == 0
        || crop.height == 0
        || crop.width % sx != 0
        || crop.height % sy != 0
        || crop.x % sx != 0
        || crop.y % sy != 0
        || crop
            .x
            .checked_add(crop.width)
            .is_none_or(|end| end > header.width)
        || crop
            .y
            .checked_add(crop.height)
            .is_none_or(|end| end > header.height)
    {
        return Err("invalid or unaligned Y4M crop".into());
    }
    let area = crop
        .width
        .checked_mul(crop.height)
        .ok_or("Y4M crop size overflow")?;
    let size = area
        .checked_add(
            (area / sx / sy)
                .checked_mul(2)
                .ok_or("Y4M crop size overflow")?,
        )
        .and_then(|n| n.checked_mul(if header.depth() == 8 { 1 } else { 2 }))
        .ok_or("Y4M crop size overflow")?;
    Ok((crop, size))
}
fn output_geometry(
    header: &Header,
    crop: CropRect,
    scale: Option<ScaleSize>,
) -> Result<(usize, usize, usize)> {
    let (w, h) = scale.map_or((crop.width, crop.height), |s| {
        (s.width as usize, s.height as usize)
    });
    if scale.is_some() && (w == 0 || h == 0 || w > 8192 || h > 4320 || w % 2 != 0 || h % 2 != 0) {
        return Err("scale must be even and within 1..=8192 x 1..=4320".into());
    }
    let (sx, sy) = header.format.subsampling();
    let area = w.checked_mul(h).ok_or("Y4M scale size overflow")?;
    let size = area
        .checked_add(
            (area / sx / sy)
                .checked_mul(2)
                .ok_or("Y4M scale size overflow")?,
        )
        .and_then(|n| n.checked_mul(if header.depth() == 8 { 1 } else { 2 }))
        .ok_or("Y4M scale size overflow")?;
    Ok((w, h, size))
}
fn point_sample(index: usize, input: usize, output: usize) -> usize {
    let increment = (((input as u128) << 16) + output as u128 / 2) / output as u128;
    (((index as u128 * increment + increment / 2) >> 16) as usize).min(input - 1)
}
/// Transform planar pixels using the same request subset as owned decode.
pub fn transform_frame_requested(
    header: &Header,
    frame: &[u8],
    transform: &DecodeTransform,
) -> Result<Vec<u8>> {
    if !supported_request(transform) {
        return Err("owned Y4M decoder does not yet implement requested transform options".into());
    }
    let mut output = Vec::new();
    transform_frame_into(
        header,
        frame,
        transform.crop,
        transform.horizontal_flip,
        transform.vertical_flip,
        transform.scale,
        &mut output,
    )?;
    Ok(output)
}
/// Apply crop and reflections to each plane, keeping multibyte samples intact.
pub fn transform_frame(
    header: &Header,
    frame: &[u8],
    crop: Option<CropRect>,
    horizontal: bool,
    vertical: bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    transform_frame_into(header, frame, crop, horizontal, vertical, None, &mut output)?;
    Ok(output)
}
fn transform_frame_into(
    header: &Header,
    frame: &[u8],
    crop: Option<CropRect>,
    horizontal: bool,
    vertical: bool,
    scale: Option<ScaleSize>,
    output: &mut Vec<u8>,
) -> Result<()> {
    if frame.len() != header.frame_len()? {
        return Err("Y4M frame size mismatch".into());
    }
    let (crop, _) = crop_geometry(header, crop)?;
    let (ow, oh, output_size) = output_geometry(header, crop, scale)?;
    let (sx, sy) = header.format.subsampling();
    let step = if header.depth() == 8 { 1 } else { 2 };
    output.clear();
    output
        .try_reserve_exact(output_size)
        .map_err(|e| e.to_string())?;
    let mut offset = 0;
    for (dx, dy) in [(1, 1), (sx, sy), (sx, sy)] {
        let stride = header.width / dx * step;
        if scale.is_some() {
            let (iw, ih, dw, dh) = (crop.width / dx, crop.height / dy, ow / dx, oh / dy);
            for row in 0..dh {
                let mut y = point_sample(row, ih, dh);
                if vertical {
                    y = ih - 1 - y;
                }
                for col in 0..dw {
                    let mut x = point_sample(col, iw, dw);
                    if horizontal {
                        x = iw - 1 - x;
                    }
                    let at = offset + (crop.y / dy + y) * stride + (crop.x / dx + x) * step;
                    output.extend_from_slice(&frame[at..at + step]);
                }
            }
            offset += stride * (header.height / dy);
            continue;
        }
        let row_bytes = crop.width / dx * step;
        for row in 0..crop.height / dy {
            let row = if vertical {
                crop.height / dy - 1 - row
            } else {
                row
            };
            let start = offset + (crop.y / dy + row) * stride + crop.x / dx * step;
            let at = output.len();
            output.extend_from_slice(&frame[start..start + row_bytes]);
            if horizontal {
                fvid_cpu::hflip_row(&mut output[at..], crop.width / dx, step);
            }
        }
        offset += stride * (header.height / dy);
    }
    Ok(())
}
pub fn decode_reader_transformed(
    mut source: impl BufRead,
    transform: &DecodeTransform,
) -> Result<DecodeStats> {
    if !supported_request(transform) {
        return Err("owned Y4M decoder does not yet implement requested transform options".into());
    }
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode interval requires 0 <= from < to".into());
    }
    let mut bytes = Vec::new();
    if !line(&mut source, &mut bytes)? {
        return Err("empty Y4M input".into());
    }
    let header = Header::parse(&bytes)?;
    let [rate_n, rate_d] = header.frame_rate()?;
    let (crop, _) = crop_geometry(&header, transform.crop)?;
    let (ow, oh, _) = output_geometry(&header, crop, transform.scale)?;
    let width = u32::try_from(ow).map_err(|_| "Y4M width exceeds decode API range")?;
    let height = u32::try_from(oh).map_err(|_| "Y4M height exceeds decode API range")?;
    let frame_bytes = header.frame_len()?;
    let geometry = transform.crop.is_some()
        || transform.horizontal_flip
        || transform.vertical_flip
        || transform.scale.is_some();
    let mut input = Vec::new();
    if geometry {
        input
            .try_reserve_exact(frame_bytes)
            .map_err(|e| e.to_string())?;
        input.resize(frame_bytes, 0);
    }
    let mut output = Vec::new();
    let mut scratch = [0u8; 8192];
    let mut index = 0u64;
    let mut frames = 0u64;
    loop {
        let clock = u128::from(index) * rate_d as u128 * 1_000_000;
        if transform
            .interval
            .is_some_and(|(_, to)| clock >= to as u128 * rate_n as u128)
        {
            break;
        }
        if !line(&mut source, &mut bytes)? {
            break;
        }
        if bytes != b"FRAME\n" && !bytes.starts_with(b"FRAME ") {
            return Err("expected Y4M FRAME marker".into());
        }
        let selected = transform
            .interval
            .is_none_or(|(from, _)| clock >= from as u128 * rate_n as u128);
        let mut remaining = frame_bytes;
        while remaining != 0 {
            let count = remaining.min(scratch.len());
            let buffer = if geometry && selected {
                let at = frame_bytes - remaining;
                &mut input[at..at + count]
            } else {
                &mut scratch[..count]
            };
            source.read_exact(buffer).map_err(|error| {
                if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    "truncated Y4M frame payload".into()
                } else {
                    error.to_string()
                }
            })?;
            remaining -= count;
        }
        if selected {
            if geometry {
                transform_frame_into(
                    &header,
                    &input,
                    transform.crop,
                    transform.horizontal_flip,
                    transform.vertical_flip,
                    transform.scale,
                    &mut output,
                )?;
                std::hint::black_box(&output);
            }
            frames = frames.checked_add(1).ok_or("Y4M frame count overflow")?;
        }
        index = index.checked_add(1).ok_or("Y4M frame count overflow")?;
    }
    let layout = match header.format {
        PixelFormat::Yuv420 => "420",
        PixelFormat::Yuv422 => "422",
        PixelFormat::Yuv444 => "444",
    };
    let pixel_format = if header.depth() == 8 {
        format!("yuv{layout}p")
    } else {
        format!("yuv{layout}p{}le", header.depth())
    };
    Ok(DecodeStats {
        backend: if geometry {
            "owned Y4M planar decode"
        } else {
            "owned Y4M raw decode"
        },
        video_frames: frames,
        width,
        height,
        pixel_format,
        decode_errors: 0,
    })
}
