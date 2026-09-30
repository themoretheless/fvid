//! Owned planar Y4M and single-video VP9/AV1 Matroska to FFV1 export.
use crate::container::matroska_write::{
    Encoding, PacketWriter, TrackOptions, TrackSpec, VideoMetadata,
};
use crate::{
    Result, invalid,
    native_geometry::VideoGeometry,
    native_pixels::PixelFilters,
    playback_native::{NativeReader, RawFrame},
};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
};
pub fn is_source(source: &Path) -> Result<bool> {
    let mut file = File::open(source)?;
    let mut magic = [0; 9];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == b"YUV4MPEG2"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}
/// Admission preserves the legacy path for Y4M profiles not owned yet.
pub fn eligible(source: &Path) -> Result<bool> {
    if !is_source(source)? {
        if !crate::native_export::is_matroska_source(source)? {
            return Ok(false);
        }
        let input = crate::container::webm::WebmReader::open(
            BufReader::new(File::open(source)?),
            Default::default(),
        )?;
        return Ok(input.tracks.len() == 1
            && input.tracks[0].kind == 1
            && matches!(input.tracks[0].codec.as_str(), "V_VP9" | "V_AV1")
            && input.tracks[0].crop == [0; 4]
            && input.tracks[0].rotation == 0);
    }
    let mut input = BufReader::new(File::open(source)?);
    let mut line = Vec::new();
    crate::line(&mut input, &mut line)?;
    let Ok(header) = crate::Header::parse(&line) else {
        return Ok(false);
    };
    Ok(header.tokens.iter().any(|t| t.starts_with('F'))
        && !header
            .tokens
            .iter()
            .any(|t| matches!(t.as_str(), "C420mpeg2" | "C420paldv")))
}
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(CancelFlag::is_cancelled) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
pub fn write<W: Write + Seek>(
    source: &Path,
    output: &mut W,
    geometry: &VideoGeometry,
    filters: &PixelFilters,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    check(cancel)?;
    if !eligible(source)? {
        return Err(invalid("unsupported owned planar lossless source"));
    }
    let mut webm = if is_source(source)? {
        None
    } else {
        Some(crate::container::webm::WebmReader::open(
            BufReader::new(File::open(source)?),
            Default::default(),
        )?)
    };
    if let Some(input) = webm.as_mut() {
        input.scan_all()?;
    }
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let first = reader
        .read_frame_raw()?
        .ok_or_else(|| invalid("input has no video frames"))?;
    let [w, h] = reader.dimensions();
    let depth_of = |frame: &RawFrame| -> Result<u8> {
        match frame {
            RawFrame::Planar(p) => Ok(p.depth),
            RawFrame::Planar8(_) | RawFrame::Yuv { .. } => Ok(8),
            _ => Err(invalid("expected planar sample planes")),
        }
    };
    let depth = depth_of(&first)?;
    let prepare = |frame: &RawFrame| -> Result<_> {
        if depth_of(frame)? != depth {
            return Err(invalid("frame sample depth changed"));
        }
        let mut samples = geometry.apply_display_media(frame, w, h, 0)?;
        filters.apply(&mut samples, depth)?;
        Ok(samples)
    };
    let mut colour = reader.colour();
    if let RawFrame::Planar8(ref p) = first {
        colour.full_range = p.colour.full;
    }
    let first = prepare(&first)?;
    let shape = (first.width, first.height, first.subsampling);
    let specs = [TrackSpec {
        encoding: Encoding::Ffv1V1 {
            width: u32::try_from(first.width).map_err(|_| invalid("FFV1 width overflow"))?,
            height: u32::try_from(first.height).map_err(|_| invalid("FFV1 height overflow"))?,
        },
        name: webm.as_ref().map_or("", |r| r.tracks[0].name.as_str()),
        language: webm.as_ref().map_or("", |r| r.tracks[0].language.as_str()),
    }];
    let options = [TrackOptions {
        video: Some(VideoMetadata {
            pixel_aspect: crate::native_export::transformed_aspect(
                reader.pixel_aspect(),
                w,
                h,
                geometry,
            )?,
            colour: Some(colour),
            hdr: reader.hdr(),
            ..Default::default()
        }),
        ..Default::default()
    }];
    let metadata = webm
        .as_ref()
        .map(|r| crate::container::matroska_write::FileMetadata {
            tags: r.tags.clone(),
            chapters: r.chapters.clone(),
        })
        .unwrap_or_default();
    let mut writer = PacketWriter::new_with_metadata(output, &specs, &options, &metadata)?;
    let mut stats = crate::media_info::LosslessStats {
        backend: "fvid",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: false,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: match first.subsampling {
            Some([2, 2]) => "yuv420p",
            Some([2, 1]) => "yuv422p",
            Some([1, 1]) => "yuv444p",
            _ => return Err(invalid("unsupported Y4M chroma layout")),
        }
        .into(),
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: geometry.vertical_flip,
        horizontal_flip: geometry.horizontal_flip,
    };
    if depth != 8 {
        stats.pixel_format = format!("{}{depth}le", stats.pixel_format);
    }
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    let mut next = Some(first);
    while let Some(samples) = next.take() {
        check(cancel)?;
        if (samples.width, samples.height, samples.subsampling) != shape {
            return Err(invalid("Y4M frame geometry changed"));
        }
        let (start, end, scale) = reader
            .frame_interval()
            .ok_or_else(|| invalid("missing Y4M timestamp"))?;
        let ns = |n: u128| -> Result<u64> {
            if scale == 0 {
                return Err(invalid("zero Y4M frame clock"));
            }
            u64::try_from(
                n.checked_mul(1_000_000_000)
                    .ok_or_else(|| invalid("Y4M clock overflow"))?
                    / u128::from(scale),
            )
            .map_err(|_| invalid("Y4M timestamp overflow"))
        };
        let (start, end) = (ns(start)?, ns(end)?);
        if end <= start {
            return Err(invalid("Y4M frame duration below one nanosecond"));
        }
        let packet = crate::codec::ffv1_encoder::encode(&samples, depth)?;
        check(cancel)?;
        writer.write_packet(0, start, end - start, true, &packet)?;
        stats.video_frames += 1;
        stats.decoded_frames += 1;
        stats.video_packets += 1;
        stats.fvid_crop_payload_copies += u64::from(geometry.crop.is_some());
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
        check(cancel)?;
        next = reader.read_frame_raw()?.as_ref().map(prepare).transpose()?;
    }
    check(cancel)?;
    Ok((stats, writer.finish()?))
}
