//! Native planar Y4M export. No foreign decoder, encoder or muxer is used.
pub(crate) mod pcm_matroska;
mod overlay;
pub(crate) use overlay::TimedOverlay;
pub use overlay::{overlay_video, overlay_eligible, overlay_video_transformed};
use crate::{
    Result, invalid,
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Export a constant-rate planar video. Variable timing/geometry is rejected
/// rather than silently retimed. The destination is created only on success;
/// an existing destination (including a symlink) is never replaced.
pub fn export_y4m(source: &Path, destination: &Path) -> Result<u64> {
    export_y4m_interval(source, destination, None)
}

/// Export frames with presentation starts in [from, to), decoding reference
/// pre-roll through the owned codec. The resulting Y4M starts at time zero.
pub fn export_y4m_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<u64> {
    export_y4m_transformed(
        source,
        destination,
        interval,
        &crate::native_geometry::VideoGeometry::default(),
    )
}

/// Save owned geometry transformations after container display orientation.
/// Scaling retains the pre-scale display aspect by adjusting pixel aspect.
pub fn export_y4m_transformed(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<u64> {
    export_y4m_pipeline(
        source,
        destination,
        interval,
        geometry,
        &Default::default(),
    )
}

/// Export owned spatial and pixel processing while retaining frame timing and depth.
pub fn export_y4m_pipeline(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<u64> {
    Ok(export_y4m_pipeline_with_options(
        source,
        destination,
        interval,
        geometry,
        filters,
        &Default::default(),
    )?
    .packets)
}

/// Export owned spatial and pixel processing with packet, payload, and RSS budgets.
pub fn export_y4m_pipeline_with_options(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    validate_y4m_options(options)?;
    if destination.extension().and_then(|s| s.to_str()) != Some("y4m") {
        return Err(invalid("native video export output requires .y4m"));
    }
    validate_video_selection_with_options(source, options.streams.first().copied(), options)?;
    export_y4m_sources_with_options(
        &[source.to_owned()],
        destination,
        Y4mInterval { range: interval, relative: false },
        geometry,
        filters,
        options,
    )
}

/// Decode compatible video segments in order into one constant-rate Y4M stream.
/// Each source gets a fresh decoder; only raw presentation frames are appended.
pub fn concat_y4m(sources: &[PathBuf], destination: &Path, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    concat_y4m_with_options(sources, destination, &options)
}

pub fn concat_y4m_with_options(
    sources: &[PathBuf],
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    validate_y4m_options(options)?;
    if !(2..=256).contains(&sources.len()) {return Err(invalid("concat requires 2..=256 inputs"));}
    if destination.extension().and_then(|s|s.to_str())!=Some("y4m") {return Err(invalid("native video concat output requires .y4m"));}
    let selected = options.streams.first().copied();
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        validate_video_selection_with_options(source,selected,options)?;
    }
    export_y4m_sources_with_options(
        sources,
        destination,
        Y4mInterval { range: None, relative: false },
        &Default::default(),
        &Default::default(),
        options,
    )
}

pub(crate) fn validate_y4m_options(options: &fvid_media::CopyOptions) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(crate::media_control::CancelFlag::is_cancelled)
    {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(invalid("Y4M output does not support metadata mutations"));
    }
    if options.streams.len() > 1 {
        return Err(invalid("native media operation does not yet support stream selection, custom budgets or metadata mutations"));
    }
    Ok(())
}

pub(crate) fn inspect_first_y4m_frame(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<(
    (u128, u128),
    (usize, usize, &'static str, u8, bool, (u32, u32)),
    usize,
)> {
    inspect_first_y4m_frame_with_filters(source, geometry, &Default::default())
}

pub(crate) fn inspect_first_y4m_frame_with_filters(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
) -> Result<(
    (u128, u128),
    (usize, usize, &'static str, u8, bool, (u32, u32)),
    usize,
)> {
    inspect_first_y4m_frame_with_filters_and_options(
        source,
        geometry,
        filters,
        &fvid_media::CopyOptions::default(),
    )
}

pub(crate) fn inspect_first_y4m_frame_with_filters_and_options(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    options: &fvid_media::CopyOptions,
) -> Result<(
    (u128, u128),
    (usize, usize, &'static str, u8, bool, (u32, u32)),
    usize,
)> {
    let mut reader = NativeReader::software(
        BufReader::new(File::open(source)?),
        options.max_controlled_bytes.unwrap_or(usize::MAX),
    )?;
    let frame = reader
        .read_frame_raw()?
        .ok_or_else(|| invalid("input has no decoded video frames"))?;
    let (start, end, scale) = reader
        .frame_interval()
        .ok_or_else(|| invalid("missing frame timing"))?;
    let duration = end
        .checked_sub(start)
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("invalid frame duration"))?;
    let [width, height] = reader.dimensions();
    let rotation = reader.rotation();
    let (mut chroma, depth) = match &frame {
        RawFrame::Planar(p) => (
            match p.frame.subsampling {
                Some([2, 2]) => "420",
                Some([2, 1]) => "422",
                Some([1, 1]) => "444",
                _ => return Err(invalid("unsupported Y4M chroma layout")),
            },
            p.depth,
        ),
        RawFrame::Avc { picture, .. }
            if width % 2 == 0
                && height % 2 == 0
                && picture.cb.len()
                    == picture.coded_width.div_ceil(2) * picture.coded_height.div_ceil(2)
                && picture.cr.len() == picture.cb.len() =>
        {
            ("420", picture.bit_depth)
        }
        RawFrame::Planar8(p) if p.chroma_width == p.width && p.chroma_height == p.height => {
            ("444", 8)
        }
        RawFrame::Planar8(p)
            if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height =>
        {
            ("422", 8)
        }
        RawFrame::Planar8(p)
            if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height.div_ceil(2) =>
        {
            ("420", 8)
        }
        _ => return Err(invalid("Y4M export requires supported planar frames")),
    };
    let full = match &frame {
        RawFrame::Avc { colour, .. } => colour.full,
        RawFrame::Planar8(p) => p.colour.full,
        RawFrame::Planar(p) => p.colour.full,
        _ => reader.colour().full_range,
    };
    let insets = reader.insets();
    let [visible_w, visible_h] =
        crate::virtual_camera::visible_dimensions([width, height], insets)?;
    let aspect = transformed_aspect(reader.pixel_aspect(), visible_w, visible_h, geometry)?;
    let (width, height) = if geometry.is_identity() && rotation == 0 && insets == [0; 4] && filters.is_empty() {
        (width, height)
    } else {
        let mut picture = geometry.apply_cropped_display(&frame, width, height, rotation, insets)?;
        if !filters.is_empty() {
            filters.configure_vignette_source(&reader, geometry)?;
            filters.apply_colour_clock(
                &mut picture,
                depth,
                full,
                reader.colour().matrix,
                0,
                Some(start as f64 / scale as f64),
                crate::native_pixels::frame_clock(&reader)?,
            )?;
        }
        chroma = match picture.subsampling {
            Some([2, 2]) => "420",
            Some([2, 1]) => "422",
            Some([1, 1]) => "444",
            _ => {
                return Err(invalid(
                    "Y4M export cannot represent this transformed chroma layout",
                ));
            }
        };
        (picture.width, picture.height)
    };
    let cw = if chroma == "444" {
        width
    } else {
        width.div_ceil(2)
    };
    let ch = if chroma == "420" {
        height.div_ceil(2)
    } else {
        height
    };
    let bytes = width
        .checked_mul(height)
        .and_then(|n| {
            cw.checked_mul(ch)
                .and_then(|c| c.checked_mul(2))
                .and_then(|c| n.checked_add(c))
        })
        .and_then(|n| n.checked_mul(if depth == 8 { 1 } else { 2 }))
        .ok_or_else(|| invalid("Y4M payload count overflow"))?;
    Ok((
        (duration, u128::from(scale)),
        (width, height, chroma, depth, full, aspect),
        bytes,
    ))
}

pub(crate) fn validate_y4m_concat_compatibility(sources: &[PathBuf]) -> Result<()> {
    validate_y4m_concat_compatibility_with_options(sources, &fvid_media::CopyOptions::default())
}

pub(crate) fn validate_y4m_concat_compatibility_with_options(
    sources: &[PathBuf],
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    let mut first_rate: Option<(u128, u128)> = None;
    let mut first_layout = None;
    for source in sources {
        let ((duration, scale), layout, _) = inspect_first_y4m_frame_with_filters_and_options(
            source,
            &Default::default(),
            &Default::default(),
            options,
        )?;
        if let Some((first_duration, first_scale)) = first_rate {
            if product(duration, first_scale)? != product(first_duration, scale)? {
                return Err(invalid("Y4M concat requires identical frame rates"));
            }
        } else {
            first_rate = Some((duration, scale));
        }
        if let Some(expected) = first_layout {
            if expected != layout {
                return Err(invalid("Y4M export requires fixed frame geometry and depth"));
            }
        } else {
            first_layout = Some(layout);
        }
    }
    Ok(())
}

pub(crate) fn validate_video_selection(source: &Path, selected: Option<usize>) -> Result<()> {
    validate_video_selection_with_options(source, selected, &fvid_media::CopyOptions::default())
}

pub(crate) fn validate_video_selection_with_options(
    source: &Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    use std::io::Read;
    let mut file=File::open(source)?;
    let mut prefix=[0;9];file.read_exact(&mut prefix)?;
    if &prefix==b"YUV4MPEG2" {
        if selected.is_some_and(|i|i!=0) {return Err(invalid("Y4M has only stream 0"));}
        return Ok(());
    }
    let info=crate::native_probe::probe_with_options(source, options).map_err(|e|invalid(&e))?;
    let videos:Vec<_>=info.streams.iter().filter(|s|s.media_type=="video").collect();
    if videos.len()!=1 {return Err(invalid("native video export requires exactly one video track per input"));}
    if selected.is_some_and(|i|i!=videos[0].index) || (selected.is_none() && info.streams.len()!=1) {
        return Err(invalid("video-only export requires explicit selection when other tracks exist"));
    }
    Ok(())
}

pub(crate) fn validate_video_packet_budget(
    source: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    validate_video_packet_budget_with_geometry_and_limit(
        source,
        &Default::default(),
        options,
        options.max_packets,
    )
}

pub(crate) fn validate_video_packet_budget_with_geometry(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    options: &fvid_media::CopyOptions,
) -> Result<()> {
    validate_video_packet_budget_with_geometry_and_limit(
        source,
        geometry,
        options,
        options.max_packets,
    )
}

pub(crate) fn validate_video_packet_budget_with_geometry_and_limit(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    options: &fvid_media::CopyOptions,
    max_packets: Option<u64>,
) -> Result<()> {
    validate_video_packet_budget_with_geometry_interval_and_limit(
        source,
        geometry,
        options,
        None,
        false,
        max_packets,
    )
}

pub(crate) fn validate_video_packet_budget_with_geometry_interval_and_limit(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    options: &fvid_media::CopyOptions,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    relative_interval: bool,
    max_packets: Option<u64>,
) -> Result<()> {
    validate_video_packet_budget_with_geometry_filters_interval_and_limit(
        source,
        geometry,
        &Default::default(),
        options,
        interval,
        relative_interval,
        max_packets,
    )
}

pub(crate) fn validate_video_packet_budget_with_geometry_filters_interval_and_limit(
    source: &Path,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    options: &fvid_media::CopyOptions,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    relative_interval: bool,
    max_packets: Option<u64>,
) -> Result<()> {
    use std::io::Read;
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    let mut file = File::open(source)?;
    let mut prefix = [0u8; 9];
    let read = file.read(&mut prefix)?;
    if read == 9 && &prefix == b"YUV4MPEG2" {
        let mut input = BufReader::new(File::open(source)?);
        let mut line = Vec::new();
        if crate::line(&mut input, &mut line)? {
            let header = crate::Header::parse(&line)?;
            let frame_len = header.frame_len()?;
            if frame_len > options.max_packet_bytes {
                return Err(invalid("Y4M frame exceeds byte limit"));
            }
            if let Some(mem_limit) = options.max_controlled_bytes
                && frame_len > mem_limit
            {
                return Err(invalid("Y4M frame exceeds controlled memory budget"));
            }
        }
        let ((duration, scale), (_, _, _, depth, _, _), bytes) =
            inspect_first_y4m_frame_with_filters_and_options(source, geometry, filters, options)?;
        if bytes > options.max_packet_bytes {
            return Err(invalid("Y4M frame exceeds byte limit"));
        }
        if let Some(mem_limit) = options.max_controlled_bytes
            && filters.working_bytes(bytes, depth) > mem_limit
        {
            return Err(invalid("Y4M frame exceeds controlled memory budget"));
        }
        if let Some((from, to)) = interval {
            if let Some(frame_count) = y4m_segment_frame_upper_bound_with_options(source, None, options)? {
                let mut selected_in_interval = 0u64;
                for index in 0..frame_count {
                    let start = u128::from(index).saturating_mul(duration);
                    let stamp = product(start, 1_000_000_000)?;
                    if stamp >= product(to.as_nanos(), scale)? {
                        break;
                    }
                    if stamp >= product(from.as_nanos(), scale)? {
                        selected_in_interval = selected_in_interval.saturating_add(1);
                    }
                }
                if selected_in_interval == 0 {
                    return Err(invalid("input has no decoded video frames"));
                }
            }
        }
        return Ok(());
    }
    let selected = options.streams.first().copied();
    let max_samples = max_packets
        .map(|limit| usize::try_from(limit.max(1)).unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX);
    if read >= 8 && crate::container::mp4::recognizes_prefix(&prefix[..8]) {
        let effective_packet_limit = options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
        let reader = crate::container::mp4::Mp4Reader::open(
            BufReader::new(File::open(source)?),
            crate::container::mp4::Limits {
                packet_bytes: effective_packet_limit,
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )?;
        for (idx, track) in reader.tracks().iter().enumerate() {
            if track.handler != *b"vide" || selected.is_some_and(|sel| sel != idx) {
                continue;
            }
            let non_reordered = track.edits.is_empty()
                && track.timescale > 0
                && track
                    .samples
                    .expanded()
                    .is_some_and(|samples| {
                        samples.first().is_none_or(|s| s.pts >= 0)
                            && samples.windows(2).all(|w| w[0].pts <= w[1].pts)
                    });
            let mut empty_interval = false;
            let exceeds = match &track.samples {
                crate::container::mp4::SampleIndex::Expanded(samples) => {
                    if !non_reordered {
                        samples
                            .iter()
                            .any(|s| s.size as usize > effective_packet_limit)
                    } else if let Some((from, to)) = interval {
                        let first_pts =
                            samples.first().map_or(0u128, |s| s.pts.max(0) as u128);
                        let scale = u128::from(track.timescale);
                        let from_scaled = from.as_nanos().saturating_mul(scale);
                        let to_scaled = to.as_nanos().saturating_mul(scale);
                        let mut emitted_in_interval = 0usize;
                        let mut bad = false;
                        for s in samples {
                            let start = s.pts.max(0) as u128;
                            let rel = if relative_interval {
                                start.saturating_sub(first_pts)
                            } else {
                                start
                            };
                            let stamp = rel.saturating_mul(1_000_000_000);
                            if stamp >= to_scaled || emitted_in_interval >= max_samples {
                                break;
                            }
                            if s.size as usize > effective_packet_limit {
                                bad = true;
                                break;
                            }
                            if stamp >= from_scaled {
                                emitted_in_interval = emitted_in_interval.saturating_add(1);
                            }
                        }
                        if !bad && max_samples > 0 && emitted_in_interval == 0 {
                            empty_interval = true;
                        }
                        bad
                    } else {
                        samples
                            .iter()
                            .take(max_samples)
                            .any(|s| s.size as usize > effective_packet_limit)
                    }
                }
                crate::container::mp4::SampleIndex::Uniform(uniform) => {
                    uniform.count > 0
                        && max_samples > 0
                        && uniform.bytes_per_frame as usize > effective_packet_limit
                }
            };
            if exceeds {
                return Err(invalid("MP4 packet exceeds budget"));
            }
            if empty_interval {
                return Err(invalid("input has no decoded video frames"));
            }
        }
        let (_, (_, _, _, depth, _, _), bytes) =
            inspect_first_y4m_frame_with_filters_and_options(source, geometry, filters, options)?;
        if bytes > options.max_packet_bytes {
            return Err(invalid("Y4M frame exceeds byte limit"));
        }
        if let Some(mem_limit) = options.max_controlled_bytes
            && filters.working_bytes(bytes, depth) > mem_limit
        {
            return Err(invalid("Y4M frame exceeds controlled memory budget"));
        }
        return Ok(());
    }
    if read >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        let mut limits = crate::container::webm::Limits::default();
        let memory_packets = options
            .max_controlled_bytes
            .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets
                .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader =
            crate::container::webm::WebmReader::open(BufReader::new(File::open(source)?), limits)
                .map_err(map_packet_limit)?;
        reader.scan_all().map_err(map_packet_limit)?;
        let effective_packet_limit = options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
        let selected_tracks: Vec<(u64, bool)> = reader
            .tracks
            .iter()
            .enumerate()
            .filter(|(idx, t)| t.kind == 1 && selected.is_some_and(|sel| sel == *idx) || (t.kind == 1 && selected.is_none()))
            .map(|(_, t)| {
                (
                    t.number,
                    matches!(t.codec.as_str(), "V_VP9" | "V_AV1" | "V_FFV1"),
                )
            })
            .collect();
        let mut visible_on_track = 0usize;
        let mut first_pts_on_track: Option<i64> = None;
        for index in 0..reader.packets.len() {
            let packet = &reader.packets[index];
            let Some((_, non_reordered)) = selected_tracks
                .iter()
                .find(|(num, _)| *num == packet.track)
            else {
                continue;
            };
            if *non_reordered {
                if visible_on_track >= max_samples {
                    break;
                }
                if !packet.invisible
                    && let Some((_, to)) = interval
                {
                    let base = *first_pts_on_track.get_or_insert(packet.pts_ns);
                    if let Ok(rel_ns) = u128::try_from(i128::from(packet.pts_ns) - i128::from(base))
                        && rel_ns >= to.as_nanos()
                    {
                        break;
                    }
                }
            }
            if reader.packet_payload_size(index)? > effective_packet_limit {
                return Err(invalid("WebM packet exceeds budget"));
            }
            if !packet.invisible {
                let base = *first_pts_on_track.get_or_insert(packet.pts_ns);
                let in_interval = if let Some((from, _)) = interval {
                    u128::try_from(i128::from(packet.pts_ns) - i128::from(base))
                        .is_ok_and(|rel_ns| rel_ns >= from.as_nanos())
                } else {
                    true
                };
                if in_interval {
                    visible_on_track = visible_on_track.saturating_add(1);
                }
            }
        }
        let (_, (_, _, _, depth, _, _), bytes) =
            inspect_first_y4m_frame_with_filters_and_options(source, geometry, filters, options)?;
        if bytes > options.max_packet_bytes {
            return Err(invalid("Y4M frame exceeds byte limit"));
        }
        if let Some(mem_limit) = options.max_controlled_bytes
            && filters.working_bytes(bytes, depth) > mem_limit
        {
            return Err(invalid("Y4M frame exceeds controlled memory budget"));
        }
        if interval.is_some()
            && max_samples > 0
            && selected_tracks.iter().all(|(_, non_reordered)| *non_reordered)
            && visible_on_track == 0
        {
            return Err(invalid("input has no decoded video frames"));
        }
    }
    Ok(())
}

pub(crate) fn y4m_segment_frame_upper_bound(
    source: &Path,
    selected: Option<usize>,
) -> Result<Option<u64>> {
    y4m_segment_frame_upper_bound_with_options(
        source,
        selected,
        &fvid_media::CopyOptions::default(),
    )
}

pub(crate) fn y4m_segment_frame_upper_bound_with_options(
    source: &Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<Option<u64>> {
    use std::io::Read;
    let mut file = File::open(source)?;
    let mut prefix = [0u8; 9];
    let read = file.read(&mut prefix)?;
    if read == 9 && &prefix == b"YUV4MPEG2" {
        let metadata = file.metadata()?;
        let mut input = BufReader::new(File::open(source)?);
        let mut line = Vec::new();
        if !crate::line(&mut input, &mut line)? {
            return Ok(Some(0));
        }
        let header = crate::Header::parse(&line)?;
        let stride = (header.frame_len()? as u64).saturating_add(6);
        let payload = metadata.len().saturating_sub(line.len() as u64);
        if stride > 0 && payload % stride == 0 {
            return Ok(Some(payload / stride));
        }
        return Ok(None);
    }
    if read >= 8 && crate::container::mp4::recognizes_prefix(&prefix[..8]) {
        let reader = crate::container::mp4::Mp4Reader::open(
            BufReader::new(File::open(source)?),
            crate::container::mp4::Limits {
                packet_bytes: options
                    .max_packet_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                metadata_bytes: crate::container::mp4::Limits::default()
                    .metadata_bytes
                    .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
                ..Default::default()
            },
        )?;
        for (idx, track) in reader.tracks().iter().enumerate() {
            if track.handler != *b"vide" || selected.is_some_and(|sel| sel != idx) {
                continue;
            }
            if track.edits.is_empty() {
                return Ok(Some(track.samples.len() as u64));
            }
            return Ok(None);
        }
        return Ok(None);
    }
    if read >= 4 && prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        let mut limits = crate::container::webm::Limits::default();
        let memory_packets = options
            .max_controlled_bytes
            .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets
                .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader = crate::container::webm::WebmReader::open(
            BufReader::new(File::open(source)?),
            limits,
        )
        .map_err(map_packet_limit)?;
        reader.scan_all().map_err(map_packet_limit)?;
        for (idx, track) in reader.tracks.iter().enumerate() {
            if track.kind != 1 || selected.is_some_and(|sel| sel != idx) {
                continue;
            }
            let number = track.number;
            let count = reader
                .packets
                .iter()
                .filter(|p| p.track == number && !p.invisible)
                .count() as u64;
            return Ok(Some(count));
        }
    }
    Ok(None)
}

struct Y4mInterval {
    range: Option<(std::time::Duration, std::time::Duration)>,
    relative: bool,
}

fn export_y4m_sources(sources: &[PathBuf], destination: &Path,
    selection: Y4mInterval, geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    export_y4m_sources_with_options(sources, destination, selection, geometry, filters, &options)
}

fn export_y4m_sources_with_options(
    sources: &[PathBuf],
    destination: &Path,
    selection: Y4mInterval,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    let Y4mInterval { range: interval, relative: relative_interval } = selection;
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    let mut control=crate::native_media::DecodeProgress::new_with_limits(
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_rss_bytes,
        options.max_packets,
    )?;
    if interval.is_some_and(|(from, to)| from >= to) {
        return Err(invalid("export interval requires from < to"));
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, file) = (0..100)
        .find_map(|_| {
            let path = directory.join(format!(
                ".fvid-y4m-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Some(Ok((Temporary(path), file))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            }
        })
        .ok_or_else(|| invalid("cannot reserve Y4M output"))??;
    let mut output = BufWriter::new(file);
    let mut output_rate: Option<(u128,u128)> = None;
    let mut layout = None;
    let mut count = 0;
    let mut payload_bytes=0u64;
    'sources: for source in sources {
    control.check()?;
    let remaining_packets = control
        .packet_limit()
        .map(|limit| limit.saturating_sub(count));
    validate_video_packet_budget_with_geometry_filters_interval_and_limit(
        source,
        geometry,
        filters,
        options,
        interval,
        relative_interval,
        remaining_packets,
    )?;
    if control.packet_limit_reached() {
        break;
    }
    let mut reader = NativeReader::software(
        BufReader::new(File::open(source)?),
        options.max_controlled_bytes.unwrap_or(usize::MAX),
    )?;
    let segment_start=count;
    let mut origin=None;
    let mut previous: Option<(u128, u128, u128)> = None;
    loop {
        control.check()?;
        if control.packet_limit_reached() {
            break 'sources;
        }
        let Some(frame) = reader.read_frame_raw()? else {
            break;
        };
        let (start, end, scale) = reader
            .frame_interval()
            .ok_or_else(|| invalid("missing frame timing"))?;
        let first=*origin.get_or_insert(start);
        if let Some((from, to)) = interval {
            let relative_start=if relative_interval {start.checked_sub(first).ok_or_else(||invalid("frame precedes segment origin"))?} else {start};
            let stamp = product(relative_start, 1_000_000_000)?;
            if stamp >= product(to.as_nanos(), u128::from(scale))? {
                break;
            }
            if stamp < product(from.as_nanos(), u128::from(scale))? {
                continue;
            }
        }
        let duration = end
            .checked_sub(start)
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("invalid frame duration"))?;
        if let Some((old_end, old_duration, old_scale)) = previous
            && (product(start, old_scale)? != product(old_end, u128::from(scale))?
                || product(duration, old_scale)? != product(old_duration, u128::from(scale))?)
            {
                return Err(invalid(
                    "Y4M export requires constant contiguous frame timing",
                ));
            }
        control.check()?;
        if let Some((first_duration,first_scale))=output_rate {
            if product(duration,first_scale)?!=product(first_duration,u128::from(scale))? {
                return Err(invalid("Y4M concat requires identical frame rates"));
            }
        } else {output_rate=Some((duration,u128::from(scale)));}
        previous = Some((end, duration, u128::from(scale)));
        let [width, height] = reader.dimensions();
        let rotation = reader.rotation();
        let (mut chroma, depth) = match &frame {
            RawFrame::Planar(p) => (
                match p.frame.subsampling {
                    Some([2, 2]) => "420",
                    Some([2, 1]) => "422",
                    Some([1, 1]) => "444",
                    _ => return Err(invalid("unsupported Y4M chroma layout")),
                },
                p.depth,
            ),
            RawFrame::Avc { picture, .. }
                if width % 2 == 0
                    && height % 2 == 0
                    && picture.cb.len()
                        == picture.coded_width.div_ceil(2) * picture.coded_height.div_ceil(2)
                    && picture.cr.len() == picture.cb.len() =>
            {
                ("420", picture.bit_depth)
            }
            RawFrame::Planar8(p) if p.chroma_width == p.width && p.chroma_height == p.height => {
                ("444", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height =>
            {
                ("422", 8)
            }
            RawFrame::Planar8(p)
                if p.chroma_width == p.width.div_ceil(2) && p.chroma_height == p.height.div_ceil(2) =>
            {
                ("420", 8)
            }
            _ => return Err(invalid("Y4M export requires supported planar frames")),
        };
        let full = match &frame {
            RawFrame::Avc { colour, .. } => colour.full,
            RawFrame::Planar8(p) => p.colour.full,
            RawFrame::Planar(p) => p.colour.full,
            _ => reader.colour().full_range,
        };
        let insets = reader.insets();
        let [visible_w, visible_h] = crate::virtual_camera::visible_dimensions([width, height], insets)?;
        let aspect = transformed_aspect(reader.pixel_aspect(), visible_w, visible_h, geometry)?;
        let mut transformed = if geometry.is_identity() && rotation == 0 && insets == [0; 4] && filters.is_empty() {
            None
        } else {
            Some(geometry.apply_cropped_display(&frame, width, height, rotation, insets)?)
        };
        if let Some(picture) = &mut transformed {
            filters.configure_vignette_source(&reader,geometry)?;
            filters.apply_colour_clock(picture, depth, full, reader.colour().matrix, count-segment_start, Some(start as f64/scale as f64), crate::native_pixels::frame_clock(&reader)?)?;
        }
        let (width, height) = if let Some(picture) = &transformed {
            chroma = match picture.subsampling {
                Some([2, 2]) => "420",
                Some([2, 1]) => "422",
                Some([1, 1]) => "444",
                _ => {
                    return Err(invalid(
                        "Y4M export cannot represent this transformed chroma layout",
                    ));
                }
            };
            (picture.width, picture.height)
        } else {
            (width, height)
        };
        let current = (width, height, chroma, depth, full, aspect);
        if let Some(first) = layout {
            if first != current {
                return Err(invalid(
                    "Y4M export requires fixed frame geometry and depth",
                ));
            }
        } else {
            layout = Some(current);
            let divisor = gcd(u128::from(scale), duration);
            let tag = if depth == 8 {
                chroma.to_string()
            } else {
                format!("{chroma}p{depth}")
            };
            // The native reader expresses aspect after the display rotation.
            let (an, ad) = aspect;
            writeln!(
                output,
                "YUV4MPEG2 W{width} H{height} F{}:{} Ip A{an}:{ad} C{tag} XCOLORRANGE={}",
                u128::from(scale) / divisor,
                duration / divisor,
                if full { "FULL" } else { "LIMITED" }
            )?;
        }
        output.write_all(b"FRAME\n")?;
        if let Some(picture) = transformed {
            output.write_all(&picture.data)?;
        } else {
            match frame {
                RawFrame::Avc { picture, .. } => picture.write_planar(&mut output)?,
                RawFrame::Planar(p) => output.write_all(&p.frame.data)?,
                RawFrame::Planar8(p) => {
                    output.write_all(&p.y)?;
                    output.write_all(&p.cb)?;
                    output.write_all(&p.cr)?;
                }
                _ => unreachable!(),
            }
        }
        let cw=if chroma=="444" {width} else {width.div_ceil(2)};
        let ch=if chroma=="420" {height.div_ceil(2)} else {height};
        let bytes=width.checked_mul(height).and_then(|n|cw.checked_mul(ch).and_then(|c|c.checked_mul(2)).and_then(|c|n.checked_add(c))).and_then(|n|n.checked_mul(if depth==8 {1} else {2})).ok_or_else(||invalid("Y4M payload count overflow"))?;
        if bytes > options.max_packet_bytes {
            return Err(invalid("Y4M frame exceeds byte limit"));
        }
        payload_bytes=payload_bytes.checked_add(bytes as u64).ok_or_else(||invalid("Y4M payload count overflow"))?;
        control.packet(bytes)?;
        count += 1;
        if count % 256 != 0 {control.emit(false);}
        control.check()?;
        if control.packet_limit_reached() {
            break 'sources;
        }
    }
    if count==segment_start {return Err(invalid("input has no decoded video frames"));}
    }
    if count == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    // Both paths share a directory. Unlike rename, hard_link cannot clobber a
    // destination created concurrently; unsupported filesystems return an error.
    control.check()?;
    std::fs::hard_link(&temporary.0, destination)?;
    control.emit(true);
    Ok(crate::media_control::ProgressEvent {packets:count,payload_bytes,done:true})
}
pub(crate) fn transformed_aspect(
    aspect: (u32, u32),
    width: usize,
    height: usize,
    geometry: &crate::native_geometry::VideoGeometry,
) -> Result<(u32, u32)> {
    let (mut n, mut d) = (u128::from(aspect.0), u128::from(aspect.1));
    let [_, _, mut w, mut h] = geometry.crop.unwrap_or([0, 0, width, height]);
    if geometry.transpose.is_some() {
        std::mem::swap(&mut n, &mut d);
        std::mem::swap(&mut w, &mut h);
    }
    if let Some(rotation) = geometry.rotate {
        let angle = rotation.degrees.rem_euclid(360.0);
        if angle == 90.0 || angle == 270.0 {
            std::mem::swap(&mut n, &mut d);
        }
        let (rw, rh) = rotation.size(
            u32::try_from(w).map_err(|_| invalid("rotation width overflow"))?,
            u32::try_from(h).map_err(|_| invalid("rotation height overflow"))?,
        );
        w = rw as usize;
        h = rh as usize;
    }
    if let Some([pw, ph, _, _]) = geometry.pad {
        w = pw;
        h = ph;
    }
    if let Some([ow, oh]) = geometry.scale {
        n = product(product(n, w as u128)?, oh as u128)?;
        d = product(product(d, h as u128)?, ow as u128)?;
    }
    if n == 0 || d == 0 {
        return Err(invalid("invalid transformed pixel aspect"));
    }
    let divisor = gcd(n, d);
    Ok((
        u32::try_from(n / divisor).map_err(|_| invalid("pixel aspect overflow"))?,
        u32::try_from(d / divisor).map_err(|_| invalid("pixel aspect overflow"))?,
    ))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn product(a: u128, b: u128) -> Result<u128> {
    a.checked_mul(b)
        .ok_or_else(|| invalid("Y4M timestamp overflow"))
}

/// Export ADTS AAC to raw f32le or float WAV, publishing only a complete decode.
pub fn export_aac_pcm(source: &Path, destination: &Path) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_interval(source, destination, None)
}

/// Export a half-open interval from the ADTS decoded sample timeline.
pub fn export_aac_pcm_interval(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_with_volume(source, destination, interval, 1.0)
}

/// Decode AAC to PCM/WAV with finite linear gain after interval selection.
pub fn export_aac_pcm_with_volume(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_transformed(source, destination, interval, volume, None)
}

/// Native mono/stereo conversion. Unchanged channel layouts pass through.
pub fn export_aac_pcm_transformed(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_resampled(source, destination, interval, volume, channels, None)
}

/// Native AAC export including band-limited sample-rate conversion.
pub fn export_aac_pcm_resampled(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_controlled(source, destination, interval, volume, channels, sample_rate, None, None)
}

/// AAC PCM export with encoded-packet progress and cancellation. Counts include
/// reference pre-roll/replayed edits; done is emitted only after publication.
#[expect(clippy::too_many_arguments, reason = "Preserve the public export/filter entrypoint signature for existing callers")]
pub fn export_aac_pcm_controlled(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_aac_pcm_selected(source, destination, interval, volume, channels, sample_rate, None, cancel, progress)
}

/// Export one explicitly selected container audio stream (zero-based index).
#[expect(clippy::too_many_arguments, reason = "Preserve the public export/filter entrypoint signature for existing callers")]
pub fn export_aac_pcm_selected(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(
        source, destination, interval,
        PcmTransform { volume, channels, sample_rate },
        selected,
        PcmExportLimits { cancel, progress, max_rss_bytes: None, max_packet_bytes: None, max_packets: None, options: None, source_matroska_metadata: None },
        false,
    )
}

/// Export owned AAC, MP4 ALAC or packed RIFF/WAVE PCM through the shared PCM pipeline.
#[expect(clippy::too_many_arguments, reason = "Preserve the public export/filter entrypoint signature for existing callers")]
pub fn export_audio_pcm_selected(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(
        source, destination, interval,
        PcmTransform { volume, channels, sample_rate },
        selected,
        PcmExportLimits { cancel, progress, max_rss_bytes: None, max_packet_bytes: None, max_packets: None, options: None, source_matroska_metadata: None },
        true,
    )
}

/// Export selected owned audio while checking process RSS before decoding and
/// between packets. This limit also covers decoder work during WAVE spooling.
pub fn export_audio_pcm_selected_with_rss_limit(
    source: &Path,
    destination: &Path,
    selected: Option<usize>,
    max_rss_bytes: Option<u64>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(
        source, destination, None,
        PcmTransform { volume: 1., channels: None, sample_rate: None },
        selected,
        PcmExportLimits { cancel, progress, max_rss_bytes, max_packet_bytes: None, max_packets: None, options: None, source_matroska_metadata: None },
        true,
    )
}

/// Export selected owned audio with encoded-payload and process RSS limits.
/// Payload limits are enforced by each container reader before packet allocation.
pub fn export_audio_pcm_selected_with_limits(
    source: &Path,
    destination: &Path,
    selected: Option<usize>,
    max_packet_bytes: usize,
    max_rss_bytes: Option<u64>,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(
        source, destination, None,
        PcmTransform { volume: 1., channels: None, sample_rate: None },
        selected,
        PcmExportLimits { cancel, progress, max_rss_bytes, max_packet_bytes: Some(max_packet_bytes), max_packets: None, options: None, source_matroska_metadata: None },
        true,
    )
}

/// Internal compressed-audio spooling controls. Unmigrated admission/metadata
/// policies are qualified by the caller before entering this bridge.
pub fn export_audio_pcm_selected_with_controls(
    source: &Path,
    destination: &Path,
    selected: Option<usize>,
    options: &fvid_media::CopyOptions,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_pcm_selected(
        source, destination, None,
        PcmTransform { volume: 1., channels: None, sample_rate: None },
        selected,
        PcmExportLimits { cancel: options.cancel.as_ref(), progress: options.progress.as_ref(), max_rss_bytes: options.max_rss_bytes, max_packet_bytes: Some(options.max_packet_bytes), max_packets: options.max_packets, options: Some(options), source_matroska_metadata: None },
        true,
    )
}

/// Owned transformed export with packet-work, encoded payload and RSS limits.
pub fn export_audio_pcm_transformed_with_controls(
    source: &Path, destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64, channels: Option<u16>, sample_rate: Option<u32>,
    options: &fvid_media::CopyOptions,
) -> Result<crate::native_media::AudioDecodeStats> {
    export_audio_pcm_transformed_with_source_metadata(
        source,
        destination,
        interval,
        volume,
        channels,
        sample_rate,
        options,
        None,
    )
}

pub(crate) fn export_audio_pcm_transformed_with_source_metadata(
    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
    options: &fvid_media::CopyOptions,
    source_matroska_metadata: Option<pcm_matroska::MatroskaPcmMetadata>,
) -> Result<crate::native_media::AudioDecodeStats> {
    crate::media::validate_owned_audio_options_for_destination(options, Some(destination))
        .map_err(|error| invalid(&error))?;
    export_pcm_selected(
        source, destination, interval,
        PcmTransform { volume, channels, sample_rate },
        options.streams.first().copied(),
        PcmExportLimits { cancel: options.cancel.as_ref(), progress: options.progress.as_ref(), max_rss_bytes: options.max_rss_bytes, max_packet_bytes: Some(options.max_packet_bytes), max_packets: options.max_packets, options: Some(options), source_matroska_metadata },
        true,
    )
}

struct PcmTransform {
    volume: f64,
    channels: Option<u16>,
    sample_rate: Option<u32>,
}

struct PcmExportLimits<'a> {
    cancel: Option<&'a crate::media_control::CancelFlag>,
    progress: Option<&'a crate::media_control::ProgressHook>,
    max_rss_bytes: Option<u64>,
    max_packet_bytes: Option<usize>,
    max_packets: Option<u64>,
    options: Option<&'a fvid_media::CopyOptions>,
    source_matroska_metadata: Option<pcm_matroska::MatroskaPcmMetadata>,
}

fn probe_mp4_audio_geometry<R: std::io::Read + std::io::Seek>(
    reader: &mut crate::container::mp4::Mp4Reader<R>,
    allow_wave: bool,
    selected: Option<usize>,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    control: &mut crate::native_media::DecodeProgress<'_>,
) -> Result<(u32, u16, Option<u32>)> {
    let index = if allow_wave {
        crate::native_media::mp4_audio_index(reader, selected)?
    } else {
        crate::native_media::mp4_aac_index(reader, selected)?
    };
    let max_packets = control.packet_limit();
    let in_band_ps = crate::native_media::negotiate_mp4_ps_with_interval(reader, index, interval, max_packets, || control.check())?;
    let source_track = &reader.tracks()[index];
    let first = source_track.samples.get(0).ok_or_else(|| invalid("MP4 audio has no samples"))?;
    let setup = source_track.decoder_setup(first.description_index)?;
    let track = &setup;
    let decoder = if in_band_ps {
        crate::native_audio_decoder::PacketPcmDecoder::with_in_band_ps(track)?
    } else {
        crate::native_audio_decoder::PacketPcmDecoder::new(track)?
    };
    Ok((decoder.sample_rate(), decoder.channels(), decoder.channel_mask()))
}

fn probe_matroska_audio_geometry<R: std::io::Read + std::io::Seek>(
    reader: &mut crate::container::webm::WebmReader<R>,
    allow_wave: bool,
    selected: Option<usize>,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    control: &mut crate::native_media::DecodeProgress<'_>,
) -> Result<(u32, u16, Option<u32>)> {
    let index = if allow_wave {
        crate::native_media::matroska_audio_index(reader, selected)?
    } else {
        crate::native_media::matroska_aac_index(reader, selected)?
    };
    let max_packets = control.packet_limit();
    let in_band_ps = crate::native_media::negotiate_matroska_ps_with_interval(reader, index, interval, max_packets, || control.check())?;
    let track = &reader.tracks[index];
    let decoder = if in_band_ps {
        crate::native_audio_decoder::PacketPcmDecoder::Ps(Box::new(
            crate::codec::aac_ps_native::NativePsAacDecoder::new_with_in_band_ps(
                &track.codec_private,
                u32::try_from(track.sample_rate).map_err(|_| invalid("AAC output clock overflow"))?,
            )
            .map_err(|e| invalid(&e.0))?,
        ))
    } else {
        crate::native_audio_decoder::PacketPcmDecoder::from_matroska(track)?
    };
    Ok((decoder.sample_rate(), decoder.channels(), decoder.channel_mask()))
}

fn export_pcm_selected(    source: &Path,
    destination: &Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    transform: PcmTransform,
    selected: Option<usize>,
    limits: PcmExportLimits<'_>,
    allow_wave: bool,
) -> Result<crate::native_media::AudioDecodeStats> {
    let PcmTransform { volume, channels, sample_rate } = transform;
    let PcmExportLimits { cancel, progress, max_rss_bytes, max_packet_bytes, max_packets, options, source_matroska_metadata } = limits;
    if cancel.is_some_and(|c| c.is_cancelled())
        || options
            .and_then(|o| o.cancel.as_ref())
            .is_some_and(|c| c.is_cancelled())
    {
        return Err(invalid("media operation cancelled"));
    }
    if max_packet_bytes == Some(0) || options.is_some_and(|o| o.max_packet_bytes == 0) {
        return Err(invalid("packet byte limit must be positive"));
    }
    if max_packets == Some(0) || options.is_some_and(|o| o.max_packets == Some(0)) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.is_some_and(|o| o.max_controlled_bytes == Some(0)) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    let mut control = crate::native_media::DecodeProgress::new_with_limits(cancel, progress, max_rss_bytes, max_packets)?;
    if sample_rate.is_some_and(|rate| !(8000..=384000).contains(&rate)) {
        return Err(invalid("sample rate must be within 8000..=384000"));
    }
    if !volume.is_finite() || !(0.0..=64.0).contains(&volume) {
        return Err(invalid("volume must be a finite linear gain within 0..=64"));
    }
    use std::io::{Read, Seek, SeekFrom};
    let (wav, matroska_output) = match destination.extension().and_then(|s| s.to_str()) {
        Some("wav") => (true, false),
        Some("f32le") => (false, false),
        Some("mka" | "mkv") => (false, true),
        _ => return Err(invalid("native PCM output requires .f32le, .wav, .mka or .mkv extension")),
    };
    let mut input = BufReader::new(File::open(source)?);
    let mut prefix = [0; 8];
    input.read_exact(&mut prefix)?;
    input.seek(SeekFrom::Start(0))?;
    let (mut mp4, mut matroska, adts, wave) = if allow_wave && matches!(&prefix[..4],b"RIFF"|b"RF64"|b"RIFX") {
        if selected.is_some_and(|n|n!=0) {return Err(invalid("WAVE has only stream 0"));}
        let info=crate::native_pcm::inspect(&mut input,cancel)?;
        info.validate_decode()?;
        (None,None,None,Some((input,info)))
    } else if crate::container::mp4::recognizes_prefix(&prefix) {
        let metadata_bytes = crate::container::mp4::Limits::default()
            .metadata_bytes
            .min(
                options
                    .and_then(|o| o.max_controlled_bytes)
                    .unwrap_or(usize::MAX),
            );
        let mut reader = crate::container::mp4::Mp4Reader::open_indexed(
            input,
            crate::container::mp4::Limits {
                metadata_bytes,
                ..Default::default()
            },
        )?;
        if let Some(maximum) = max_packet_bytes {
            reader.restrict_packet_bytes(maximum);
        }
        (Some(reader), None, None, None)
    } else if prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        let mut limits = crate::container::webm::Limits::default();
        let memory_packets = options
            .and_then(|o| o.max_controlled_bytes)
            .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::Error| {
            if memory_packets
                .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader =
            crate::container::webm::WebmReader::open(input, limits).map_err(map_packet_limit)?;
        if let Some(maximum) = max_packet_bytes { reader.restrict_packet_bytes(maximum); }
        (None, Some(reader), None,None)
    } else {
        (None, None, Some(crate::container::adts::StreamReader::open_with_packet_limit(input, max_packet_bytes.unwrap_or(usize::MAX))?),None)
    };
    let adts = if let Some(reader) = adts {
        if selected.is_some_and(|index| index != 0) { return Err(invalid("ADTS has only stream 0")); }
        Some(crate::native_media::negotiate_adts_aac_reader(reader, interval, &mut control)?)
    } else { None };
    let (mut input_rate, mut input_channels, mut native_mask) = if let Some(reader) = &mut mp4 {
        probe_mp4_audio_geometry(reader, allow_wave, selected, interval, &mut control)?
    } else if let Some(reader) = &mut matroska {
        probe_matroska_audio_geometry(reader, allow_wave, selected, interval, &mut control)?
    } else if let Some((_,info))=&wave {
        (info.sample_rate,info.channels,Some(info.channel_mask))
    } else {
        if selected.is_some_and(|index| index != 0) { return Err(invalid("ADTS has only stream 0")); }
        let reader = adts.as_ref().ok_or_else(|| invalid("missing ADTS reader"))?;
        (reader.sample_rate(), reader.channels(), Some(reader.channel_mask()))
    };
    // Normalize dynamic MP4 runs on the requested output grid before applying
    // gain or publishing a fixed-geometry header. Decoder clocks remain native.
    let mp4_target = if let Some(reader) = &mut mp4 {
        let index = if allow_wave { crate::native_media::mp4_audio_index(reader, selected)? }
            else { crate::native_media::mp4_aac_index(reader, selected)? };
        if crate::native_media::mp4_normalized_geometry_controlled_with_interval(reader,index,interval,max_packets,||control.check())?.is_some()
            || channels.is_some_and(|n| n != input_channels)
            || (native_mask != Some(0) && sample_rate.is_some_and(|n| n != input_rate))
        {
            let rate = sample_rate.unwrap_or(input_rate);
            let count = channels.unwrap_or(input_channels);
            if !(1..=8).contains(&count) { return Err(invalid("MP4 normalization supports 1..=8 destination channels")); }
            let mask = if channels.is_some() {
                fvid_media::owned_pcm_channels::standard_mask(count).unwrap_or(0) as u32
            } else {
                native_mask.unwrap_or_else(|| fvid_media::owned_pcm_channels::standard_mask(input_channels).unwrap_or(0) as u32)
            };
            if mask == 0 { return Err(invalid("MP4 normalization requires an explicit destination speaker layout")); }
            input_rate = rate; input_channels = count; native_mask = Some(mask);
            Some((rate,count,mask))
        } else { None }
    } else { None };
    let unknown_pcm_layout = if let Some(reader)=&matroska {
        let index=crate::native_media::matroska_audio_index(reader,selected)?;
        reader.tracks[index].codec.starts_with("A_PCM/") && input_channels>2
    } else {false};
    let output_rate = sample_rate.unwrap_or(input_rate);
    let output_channels = channels.unwrap_or(input_channels);
    if output_channels != input_channels
        && !((input_channels <= 8 && matches!(output_channels, 1 | 2))
            || (input_channels <= 2 && (1..=8).contains(&output_channels)))
    {
        return Err(invalid("native audio channel conversion supports mono/stereo downmix or 1..=8 channel upmix from mono/stereo"));
    }
    if output_channels != input_channels {
        if unknown_pcm_layout {return Err(invalid("PCM channel conversion requires a known speaker layout"));}
        if !(1..=8).contains(&input_channels) { return Err(invalid("native audio rematrixing supports 1..=8 input channels")); }
        if native_mask.is_some_and(|mask| !((input_channels <= 2 && mask == 0) || (mask != 0 && fvid_media::owned_pcm_channels::rematrix_layout_supported(input_channels, mask)))) {return Err(invalid("native audio rematrixing requires a supported speaker layout"));}
        if let Some((_, info)) = &wave { info.validate_rematrix()?; }
    }
    let output_mask = match &wave {
        _ if unknown_pcm_layout && output_channels == input_channels => 0,
        Some((_, info)) if output_channels == input_channels => info.channel_mask,
        _ if output_channels == input_channels && native_mask.is_some() => native_mask.unwrap(),
        _ => fvid_media::owned_pcm_channels::standard_mask(output_channels)
            .map(|m| m as u32)
            .unwrap_or(default_pcm_mask(output_channels)?),
    };
    let info_chunks = if wav {
        let source_info = if wave.is_some() {
            let mut file = File::open(source)?;
            let inspected = fvid_media::owned_wave_inspect::inspect(&mut file, cancel)?;
            fvid_media::owned_wave_inspect::info_chunks(&mut file, &inspected, cancel)?
        } else if let Some(reader) = &mp4 {
            let index = if allow_wave {
                crate::native_media::mp4_audio_index(reader, selected)?
            } else {
                crate::native_media::mp4_aac_index(reader, selected)?
            };
            let track = &reader.tracks()[index];
            fvid_media::owned_wave_metadata::info_for_file_tags(
                reader.tags(),
                &track.name,
                &track.language,
                Some(reader.encoder()),
                &std::collections::BTreeMap::new(),
            )
            .map_err(|e| invalid(&e))?
        } else if let Some(reader) = &mut matroska {
            let memory_packets = options
                .and_then(|o| o.max_controlled_bytes)
                .map(|bytes| bytes / std::mem::size_of::<crate::container::webm::Packet>());
            reader.scan_all().map_err(|error| {
                if memory_packets
                    .is_some_and(|max| max < crate::container::webm::Limits::default().packets)
                    && error
                        .to_string()
                        .contains("WebM packet count exceeds limit")
                {
                    invalid("controlled memory budget exceeded: Matroska packet index limit")
                } else {
                    error
                }
            })?;
            let index = if allow_wave {
                crate::native_media::matroska_audio_index(reader, selected)?
            } else {
                crate::native_media::matroska_aac_index(reader, selected)?
            };
            let track = &reader.tracks[index];
            let mut extra = reader.metadata.clone();
            if let Some(global_track_tags) = reader.track_metadata.get(&0) {
                extra.extend(global_track_tags.clone());
            }
            if let Some(uid) = reader.track_uids.get(&track.number)
                && let Some(scoped) = reader.track_metadata.get(uid)
            {
                extra.extend(scoped.clone());
            }
            let language = reader
                .track_languages
                .get(&track.number)
                .map(String::as_str)
                .unwrap_or(&track.language);
            fvid_media::owned_wave_metadata::info_for_file_tags(
                &reader.tags,
                &track.name,
                language,
                if allow_wave { Some(&reader.writing_app) } else { None },
                &extra,
            )
            .map_err(|e| invalid(&e))?
        } else {
            Vec::new()
        };
        if let Some(opts) = options {
            if !opts.stream_metadata_set.is_empty() || !opts.stream_metadata_delete.is_empty() {
                return Err(invalid("WAVE output does not support stream metadata mutations"));
            }
            if let Some(max) = opts.max_controlled_bytes {
                let mut estimated = source_info.len();
                for (key, value) in &opts.metadata_set {
                    estimated = estimated
                        .saturating_add(8)
                        .saturating_add(key.len())
                        .saturating_add(value.len())
                        .saturating_add(2);
                    if estimated > max {
                        return Err(invalid(
                            "controlled memory budget exceeded: WAVE metadata limit",
                        ));
                    }
                }
            }
            let edited = fvid_media::owned_wave_metadata::edit_info_chunks(
                &source_info,
                &opts.metadata_delete,
                &opts.metadata_set,
            )
            .map_err(|e| invalid(&e))?;
            if opts
                .max_controlled_bytes
                .is_some_and(|max| edited.len() > max)
            {
                return Err(invalid(
                    "controlled memory budget exceeded: WAVE metadata limit",
                ));
            }
            edited
        } else {
            source_info
        }
    } else {
        Vec::new()
    };
    let matroska_base_metadata = if matroska_output {
        if let Some(meta) = source_matroska_metadata {
            Some(meta)
        } else {
            Some(
                pcm_matroska::extract_source_pcm_metadata_with_interval_packet_and_memory_limit(
                    source,
                    selected,
                    allow_wave,
                    interval,
                    max_packet_bytes,
                    options.and_then(|o| o.max_packets),
                    options.and_then(|o| o.max_controlled_bytes),
                    cancel,
                )?,
            )
        }
    } else {
        None
    };
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-pcm-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(e) => Some(Err(e)),
        }
    }).ok_or_else(|| invalid("cannot reserve PCM output"))??;
    let mut output = BufWriter::new(file);
    if wav { output.write_all(&[0; 80])?; }
    let mut sink = pcm_matroska::Output::new_with_source_metadata(&mut output, matroska_output, output_rate, output_channels, matroska_base_metadata, options)?;
    let mut resampler = crate::pcm_resample::Resampler::new(&mut sink, input_rate, output_rate, output_channels)?;
    let effective_input_mask = if let Some((_, info)) = &wave {
        if info.channel_mask != 0 {
            info.channel_mask
        } else {
            fvid_media::owned_pcm_channels::standard_mask(input_channels).unwrap_or(0) as u32
        }
    } else if let Some(mask) = native_mask.filter(|&m| m != 0) {
        mask
    } else {
        fvid_media::owned_pcm_channels::standard_mask(input_channels).unwrap_or(0) as u32
    };
    let mut pcm = fvid_media::owned_pcm_gain::PcmGain::with_mask(
        &mut resampler,
        volume,
        input_channels,
        output_channels,
        effective_input_mask,
    )
    .map_err(|e| invalid(&e))?;
    let mut stats = if let Some(reader) = mp4 {
        crate::native_media::decode_mp4_audio_reader_targeted(reader, &mut pcm, interval, selected, &mut control, mp4_target)?
    } else if let Some(reader) = matroska {
        crate::native_media::decode_matroska_audio_reader_controlled(reader, &mut pcm, interval, selected, &mut control)?
    } else if let Some((reader,info))=wave {
        crate::native_pcm::decode_reader(reader,info,&mut pcm,interval,&mut control,max_packet_bytes)?
    } else {
        adts.ok_or_else(|| invalid("missing ADTS reader"))?.decode(&mut pcm, &mut control)?
    };
    control.emit(false);
    control.check()?;
    if !pcm.frame_complete() { return Err(invalid("incomplete decoded audio frame")); }
    stats.channels = output_channels;
    stats.sample_frames = resampler.finish()?;
    stats.sample_rate = output_rate;
    if let Some(frames) = sink.finish()?
        && frames != stats.sample_frames {return Err(invalid("PCM muxed sample count mismatch"));}
    if wav {
        let mut header = float_wav_header_with_mask(&stats, output_mask)?;
        if !info_chunks.is_empty() {
            let info_len = u32::try_from(info_chunks.len())
                .map_err(|_| invalid("WAVE metadata exceeds RIFF size"))?;
            let riff = u32::from_le_bytes(header[4..8].try_into().unwrap())
                .checked_add(info_len)
                .ok_or_else(|| invalid("WAVE metadata exceeds RIFF size"))?;
            header[4..8].copy_from_slice(&riff.to_le_bytes());
            output.write_all(&info_chunks)?;
        }
        output.seek(SeekFrom::Start(0))?;
        output.write_all(&header)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    control.check()?;
    std::fs::hard_link(&temporary.0, destination)?;
    control.emit(true);
    Ok(stats)
}

pub(crate) fn default_pcm_mask(channels:u16)->Result<u32> {
    fvid_media::owned_wav::default_pcm_mask(channels).map_err(|e|invalid(&e))
}
pub(crate) fn float_wav_header_with_mask(stats:&crate::native_media::AudioDecodeStats,mask:u32)->Result<Vec<u8>> {
    fvid_media::owned_wav::float_wav_header_with_mask(stats.sample_rate,stats.channels,stats.sample_frames,mask).map_err(|e|invalid(&e))
}

/// Lossless ADTS packet remux with atomic no-overwrite publication.
/// .mka/.mkv selects Matroska; other destination names retain the MP4 contract.
pub fn remux_adts_aac(source: &Path, destination: &Path) -> Result<u64> {
    remux_adts_aac_controlled(source, destination, None, None)
}

/// Native remux with packet progress and cancellation. `done=true` is emitted
/// only after the complete output has been synced and published without overwrite.
pub fn remux_adts_aac_controlled(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<u64> {
    Ok(remux_adts_aac_stats(source, destination, cancel, progress)?.packets)
}

/// Native remux counters returned after successful atomic publication.
pub fn remux_adts_aac_stats(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    remux_adts_aac_with_options(source, destination, &options)
}

pub fn remux_adts_aac_with_options(
    source: &Path,
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    if options.cancel.as_ref().is_some_and(|flag| flag.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    if options
        .max_controlled_bytes
        .is_some_and(|limit| limit < 16384)
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let input = crate::container::adts::StreamReader::open_with_packet_limit(
        BufReader::new(File::open(source)?),
        options
            .max_packet_bytes
            .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
    )?;
    publish_adts_readers_limited(
        vec![input],
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packets,
        Some(options),
    )
}

/// Content detection for the owned ADTS concat route.
pub fn is_adts_source(source: &Path) -> Result<bool> {
    use std::io::Read;
    let mut input=File::open(source)?;
    let mut header=[0;7];
    match input.read_exact(&mut header) {
        Ok(())=>Ok(crate::container::adts::header(&header).is_some()),
        Err(e) if e.kind()==std::io::ErrorKind::UnexpectedEof=>Ok(false),
        Err(e)=>Err(e.into()),
    }
}

/// Copy ADTS segments into one MP4 or Matroska track without decoding. Encoder
/// priming/padding is retained; this is packet concatenation, not gapless editing.
pub fn concat_adts_aac(sources: &[std::path::PathBuf], destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    concat_adts_aac_with_options(sources, destination, &options)
}

pub fn concat_adts_aac_with_options(
    sources: &[std::path::PathBuf],
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    if !(2..=256).contains(&sources.len()) {return Err(invalid("concat requires 2..=256 inputs"));}
    if !matches!(destination.extension().and_then(|s|s.to_str()),Some("mp4"|"m4a"|"mka"|"mkv")) {return Err(invalid("native ADTS concat output requires .mp4/.m4a or .mka/.mkv"));}
    if options
        .max_controlled_bytes
        .is_some_and(|limit| sources.len().saturating_mul(8192).saturating_add(16384) > limit)
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let mut readers=Vec::with_capacity(sources.len());
    for source in sources {
        if options.cancel.as_ref().is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
        fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
        let reader = crate::container::adts::StreamReader::open_with_packet_limit(
            BufReader::new(File::open(source)?),
            options
                .max_packet_bytes
                .min(options.max_controlled_bytes.unwrap_or(usize::MAX)),
        )?;
        readers.push(reader);
    }
    publish_adts_readers_limited(
        readers,
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packets,
        Some(options),
    )
}

fn publish_adts_readers_limited(
    mut readers: Vec<crate::container::adts::StreamReader<BufReader<File>>>,
    destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
    max_packets: Option<u64>,
    options: Option<&fvid_media::CopyOptions>,
) -> Result<crate::media_control::ProgressEvent> {
    let matroska_out = matches!(destination.extension().and_then(|s|s.to_str()),Some("mka"|"mkv"));
    if !matroska_out
        && options.is_some_and(|o| {
            !o.metadata_set.is_empty()
                || !o.metadata_delete.is_empty()
                || !o.stream_metadata_set.is_empty()
                || !o.stream_metadata_delete.is_empty()
        })
    {
        return Err(invalid("ADTS MP4 output does not support metadata mutations"));
    }
    if matroska_out {
        let meta_bytes =
            crate::container::matroska_write::estimate_adts_matroska_metadata_bytes(options)?;
        let base_bytes = if readers.len() == 1 {
            16384usize
        } else {
            readers.len().saturating_mul(8192).saturating_add(16384)
        };
        if options
            .and_then(|o| o.max_controlled_bytes)
            .is_some_and(|limit| base_bytes.saturating_add(meta_bytes) > limit)
        {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-mp4-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => Some(Err(error)),
        }
    }).ok_or_else(|| invalid("cannot reserve MP4 output"))??;
    let mut output = BufWriter::new(file);
    let max_controlled_bytes = options.and_then(|o| o.max_controlled_bytes);
    let event = if matroska_out {
        if readers.len()==1 {crate::container::matroska_write::write_adts_with_options(readers.remove(0), &mut output, cancel, progress, max_packets, options)?}
        else {crate::container::matroska_write::concat_adts_with_options(readers, &mut output, cancel, progress, max_packets, options)?}
    } else if readers.len()==1 {
        crate::container::mp4_write::write_adts_aac_reader_with_limits(readers.remove(0), &mut output, cancel, progress, max_packets, max_controlled_bytes)?
    } else {
        crate::container::mp4_write::concat_adts_readers_with_limits(readers, &mut output, cancel, progress, max_packets, max_controlled_bytes)?
    };
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0, destination)?;
    if let Some(hook) = progress { hook.emit(crate::media_control::ProgressEvent { done: true, ..event }); }
    Ok(crate::media_control::ProgressEvent { done: true, ..event })
}

/// Stream MP4 into fast-start layout without changing packets. Already
/// initialized fragmented MP4 is copied unchanged; fragment relocation is rejected.
pub fn remux_mp4(source: &Path, destination: &Path) -> Result<()> {
    remux_mp4_controlled(source, destination, None, None)
}

/// Byte progress and cancellation for native MP4 relocation. Only mdat payload
/// bytes are counted; packets remain zero. Done is emitted after publication.
pub fn remux_mp4_controlled(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<()> {
    remux_mp4_stats(source, destination, cancel, progress).map(|_| ())
}

/// MP4 relocation counters: mdat bytes, with packets zero (not inspected).
pub fn remux_mp4_stats(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_mp4_stats_with_memory_limit(source, destination, cancel, progress, None)
}

pub fn remux_mp4_stats_with_memory_limit(
    source: &Path,
    destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
    max_controlled_bytes: Option<usize>,
) -> Result<crate::media_control::ProgressEvent> {
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    if max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    let mut input=BufReader::new(File::open(source)?);
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_| {
        let path=directory.join(format!(".fvid-relocate-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file)=>Some(Ok((Temporary(path),file))),
            Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>None,
            Err(error)=>Some(Err(error)),
        }
    }).ok_or_else(||invalid("cannot reserve MP4 output"))??;
    let mut output=BufWriter::new(file);
    let event = crate::container::mp4_relocate::fast_start_with_memory_limit(&mut input, &mut output, cancel, progress, max_controlled_bytes)?;
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|flag| flag.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0,destination)?;
    if let Some(hook) = progress { hook.emit(crate::media_control::ProgressEvent { done: true, ..event }); }
    Ok(crate::media_control::ProgressEvent { done: true, ..event })
}

/// Trim ADTS audio to float WAVE, decoding pre-roll from the beginning so AAC
/// overlap state is retained. Select sample starts in [from,to), in microseconds.
pub fn trim_adts_wave(source: &Path, destination: &Path, from: i64, to: i64,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str()) != Some("wav") {return Err(invalid("native ADTS trim output requires .wav"));}
    if !is_adts_source(source)? {return Err(invalid("ADTS input required"));}
    export_aac_pcm_selected(source,destination,Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),1.0,None,None,None,cancel,progress)
}

/// Decode a selected owned audio interval to float PCM, WAVE or Matroska.
/// Bounds select sample starts in [from, to), in microseconds.
pub fn trim_audio_pcm(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if destination.extension().and_then(|s|s.to_str()) == Some("wav") {
        return trim_audio_wave(source,destination,from,to,selected,cancel,progress);
    }
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    let index=crate::native_plan::audio_trim_selection(source,selected).map_err(|e|invalid(&e))?;
    export_audio_pcm_selected(source,destination,
        Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),
        1.0,None,None,Some(index),cancel,progress)
}

/// Decode a selected owned audio interval with packet and RSS budgets.
pub fn trim_audio_pcm_with_controls(
    source: &Path,
    destination: &Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<crate::native_media::AudioDecodeStats> {
    crate::media::validate_owned_audio_options_for_destination(options, Some(destination))
        .map_err(|e| invalid(&e))?;
    if from < 0 || to <= from {
        return Err(invalid("trim requires 0 <= from < to"));
    }
    let selected = options.streams.first().copied();
    let interval = (
        std::time::Duration::from_micros(from as u64),
        std::time::Duration::from_micros(to as u64),
    );
    let (index, allow_wave) = if crate::native_media::is_aac_trim_source_with_interval_and_options(source, selected, Some(interval), options)? {
        (crate::native_plan::aac_trim_selection_with_options(source, selected, options).map_err(|e| invalid(&e))?, false)
    } else {
        (crate::native_plan::audio_trim_selection_with_interval_and_options(source, selected, Some(interval), options).map_err(|e| invalid(&e))?, true)
    };
    export_pcm_selected(
        source,
        destination,
        Some(interval),
        PcmTransform {
            volume: 1.0,
            channels: None,
            sample_rate: None,
        },
        Some(index),
        PcmExportLimits {
            cancel: options.cancel.as_ref(),
            progress: options.progress.as_ref(),
            max_rss_bytes: options.max_rss_bytes,
            max_packet_bytes: Some(options.max_packet_bytes),
            max_packets: options.max_packets,
            options: Some(options),
            source_matroska_metadata: None,
        },
        allow_wave,
    )
}

/// Decode a selected owned audio presentation interval to WAVE.
/// Multiple container tracks require explicit selection to avoid silent loss.
pub fn trim_audio_wave(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if crate::native_media::is_aac_trim_source(source, selected)? {
        return trim_aac_wave(source, destination, from, to, selected, cancel, progress);
    }
    if from < 0 || to <= from { return Err(invalid("trim requires 0 <= from < to")); }
    if destination.extension().and_then(|s|s.to_str()) != Some("wav") { return Err(invalid("native audio trim output requires .wav")); }
    if cancel.is_some_and(|c|c.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    let index = crate::native_plan::audio_trim_selection(source, selected).map_err(|e|invalid(&e))?;
    export_audio_pcm_selected(source, destination,
        Some((std::time::Duration::from_micros(from as u64), std::time::Duration::from_micros(to as u64))),
        1.0, None, None, Some(index), cancel, progress)
}

pub fn trim_aac_wave(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::native_media::AudioDecodeStats> {
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str()) != Some("wav") {return Err(invalid("native AAC trim output requires .wav"));}
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    let index=crate::native_plan::aac_trim_selection(source,selected).map_err(|e|invalid(&e))?;
    export_aac_pcm_selected(source,destination,Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))),1.0,None,None,Some(index),cancel,progress)
}

/// Decode video reference pre-roll and retain presentation starts in [from,to).
/// Bounds are relative to the first presented frame. Output retains source cadence.
pub fn trim_y4m(source: &Path, destination: &Path, from: i64, to: i64, selected: Option<usize>,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    let mut options = fvid_media::CopyOptions::default();
    options.streams = selected.into_iter().collect();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    trim_y4m_with_options(source, destination, from, to, &options)
}

pub fn trim_y4m_with_options(
    source: &Path,
    destination: &Path,
    from: i64,
    to: i64,
    options: &fvid_media::CopyOptions,
) -> Result<crate::media_control::ProgressEvent> {
    validate_y4m_options(options)?;
    if from < 0 || to <= from {return Err(invalid("trim requires 0 <= from < to"));}
    if destination.extension().and_then(|s|s.to_str())!=Some("y4m") {return Err(invalid("native video trim output requires .y4m"));}
    validate_video_selection_with_options(source,options.streams.first().copied(),options)?;
    export_y4m_sources_with_options(
        &[source.to_owned()],
        destination,
        Y4mInterval { range: Some((std::time::Duration::from_micros(from as u64),std::time::Duration::from_micros(to as u64))), relative: true },
        &Default::default(),
        &Default::default(),
        options,
    )
}

/// Whether the MP4 contains exactly one AAC track and no omitted tracks.
/// A single contiguous media edit is eligible; packet validation occurs during remux.
pub fn is_single_track_mp4_aac(source: &Path) -> Result<bool> {
    let input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    Ok(input.refused().is_empty() && input.tracks().len() == 1
        && input.tracks()[0].handler == *b"soun" && input.tracks()[0].codec == *b"mp4a"
        && (input.tracks()[0].edits.is_empty() || (input.tracks()[0].edits.len() == 1
            && input.tracks()[0].edits[0].media_time >= 0)))
}

/// Publish a single-track AAC MP4 as Matroska using owned packets, edits,
/// file tags and chapters. Existing outputs are never replaced.
pub fn remux_mp4_aac_matroska(
    source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_mp4_matroska_inner(source, destination, cancel, progress, true)
}

/// Select the owned AVC/HEVC/AAC Matroska remux route without discarding tracks.
pub fn is_native_mp4_matroska(source: &Path) -> Result<bool> {
    is_native_mp4_matroska_with_memory_limit(source, None)
}

/// Bound metadata admission during route inspection as well as execution.
pub fn is_native_mp4_matroska_with_memory_limit(source: &Path, max_controlled_bytes: Option<usize>) -> Result<bool> {
    if max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    let mut limits = crate::container::mp4::Limits::default();
    if let Some(limit) = max_controlled_bytes {
        limits.metadata_bytes = limits.metadata_bytes.min(limit);
    }
    let input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), limits)?;
    Ok(crate::container::mp4_matroska::eligible(&input))
}

/// Publish all supported MP4 video/audio tracks as Matroska, with metadata and edits.
pub fn remux_mp4_matroska(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_mp4_matroska_inner(source, destination, cancel, progress, false)
}
fn remux_mp4_matroska_inner(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>, aac_only: bool,
) -> Result<crate::media_control::ProgressEvent> {
    if cancel.is_some_and(|c| c.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    if !matches!(destination.extension().and_then(|s| s.to_str()), Some("mka" | "mkv")) {
        return Err(invalid("Matroska remux output requires .mka or .mkv"));
    }
    let mut input = crate::container::mp4::Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    let directory = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary, file) = (0..100).find_map(|_| {
        let path = directory.join(format!(".fvid-mka-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => Some(Ok((Temporary(path), file))),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
            Err(error) => Some(Err(error)),
        }
    }).ok_or_else(|| invalid("cannot reserve Matroska output"))??;
    let mut output = BufWriter::new(file);
    let event = if aac_only {
        crate::container::matroska_write::write_mp4_aac_file(&mut input, &mut output, cancel, progress)?
    } else {
        crate::container::mp4_matroska::write(&mut input, &mut output, cancel, progress)?
    };
    output.flush()?;
    output.get_ref().sync_all()?;
    drop(output);
    if cancel.is_some_and(|c| c.is_cancelled()) { return Err(invalid("media operation cancelled")); }
    std::fs::hard_link(&temporary.0, destination)?;
    let event = crate::media_control::ProgressEvent { done: true, ..event };
    if let Some(hook) = progress { hook.emit(event); }
    Ok(event)
}

/// Decode MP4 video to owned FFV1 and copy all supported AAC companions.
/// Destination is atomically published only after successful flush and sync.
pub fn transcode_mp4_ffv1(source:&Path,destination:&Path,cancel:Option<&crate::media_control::CancelFlag>,progress:Option<&crate::media_control::ProgressHook>)->Result<crate::media_info::LosslessStats> {
    transcode_mp4_ffv1_transformed(source,destination,&Default::default(),&Default::default(),cancel,progress)
}

/// Spatial FFV1 export with the same atomic publication guarantees.
pub fn transcode_ffv1_transformed(source:&Path,destination:&Path,
    geometry:&crate::native_geometry::VideoGeometry,filters:&crate::native_pixels::PixelFilters,
    cancel:Option<&crate::media_control::CancelFlag>,progress:Option<&crate::media_control::ProgressHook>,
)->Result<crate::media_info::LosslessStats> {
    transcode_ffv1_selected(source,destination,geometry,filters,cancel,progress,None)
}

pub fn transcode_ffv1_selected(source:&Path,destination:&Path,
    geometry:&crate::native_geometry::VideoGeometry,filters:&crate::native_pixels::PixelFilters,
    cancel:Option<&crate::media_control::CancelFlag>,progress:Option<&crate::media_control::ProgressHook>,
    step:Option<fvid_media::owned_framestep::FrameStep>,
)->Result<crate::media_info::LosslessStats> {
    if destination.extension().and_then(|s|s.to_str())!=Some("mkv"){return Err(invalid("lossless export requires FFV1 in .mkv"));}
    if cancel.is_some_and(|c|c.is_cancelled()){return Err(invalid("media operation cancelled"));}
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_|{
        let path=directory.join(format!(".fvid-ffv1-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path){Ok(file)=>Some(Ok((Temporary(path),file))),Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>None,Err(error)=>Some(Err(error))}
    }).ok_or_else(||invalid("cannot reserve FFV1 output"))??;
    let mut output=BufWriter::new(file);
    let (stats,event)=if crate::native_lossless_y4m::eligible(source)? {
        if let Some(step)=step {
            crate::native_lossless_y4m::write_selected(source,&mut output,geometry,filters,cancel,progress,None,step)?
        } else { crate::native_lossless_y4m::write(source,&mut output,geometry,filters,cancel,progress)? }
    } else if let Some(step)=step {
        crate::native_lossless::write_mp4_selected(source,&mut output,geometry,filters,cancel,progress,None,step)?
    } else { crate::native_lossless::write_mp4_transformed(source,&mut output,geometry,filters,cancel,progress)? };
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|c|c.is_cancelled()){return Err(invalid("media operation cancelled"));}
    std::fs::hard_link(&temporary.0,destination)?;
    if let Some(hook)=progress{hook.emit(crate::media_control::ProgressEvent{done:true,..event});}
    Ok(stats)
}


/// Recognize the EBML signature; the copy operation validates the Matroska document.
pub fn is_matroska_source(source: &Path) -> Result<bool> {
    use std::io::Read;
    let mut input=File::open(source)?;
    let mut signature=[0;4];
    match input.read_exact(&mut signature) {
        Ok(())=>Ok(signature==[0x1a,0x45,0xdf,0xa3]),
        Err(e) if e.kind()==std::io::ErrorKind::UnexpectedEof=>Ok(false),
        Err(e)=>Err(e.into()),
    }
}

/// Identity Matroska remux: retain every track, metadata element and payload.
pub fn remux_matroska(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<crate::media_control::ProgressEvent> {
    remux_matroska_with_packet_limit(source, destination, cancel, progress, 64 << 20)
}

/// Identity Matroska remux with an encoded packet byte limit.
pub fn remux_matroska_with_packet_limit(source: &Path, destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
    packet_bytes: usize,
) -> Result<crate::media_control::ProgressEvent> {
    remux_matroska_with_packet_and_memory_limits(
        source,
        destination,
        cancel,
        progress,
        packet_bytes,
        None,
    )
}

pub fn remux_matroska_with_packet_and_memory_limits(
    source: &Path,
    destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>,
    progress: Option<&crate::media_control::ProgressHook>,
    packet_bytes: usize,
    max_controlled_bytes: Option<usize>,
) -> Result<crate::media_control::ProgressEvent> {
    let audio_only=match destination.extension().and_then(|s|s.to_str()) {
        Some("mkv")=>false, Some("mka")=>true,
        _=>return Err(invalid("native Matroska copy output requires .mkv or .mka")),
    };
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    if packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    let mut input=BufReader::new(File::open(source)?);
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_| {
        let path=directory.join(format!(".fvid-matroska-copy-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file)=>Some(Ok((Temporary(path),file))),
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>None,
            Err(e)=>Some(Err(e)),
        }
    }).ok_or_else(||invalid("cannot reserve Matroska copy output"))??;
    let mut output=BufWriter::new(file);
    let event=crate::container::matroska_copy::copy_with_packet_and_memory_limits(&mut input,&mut output,audio_only,cancel,progress,packet_bytes,max_controlled_bytes)?;
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    std::fs::hard_link(&temporary.0,destination)?;
    let event=crate::media_control::ProgressEvent {done:true,..event};
    if let Some(hook)=progress {hook.emit(event);}
    Ok(event)
}

/// Compatibility name retained for callers of the original MP4-only exporter.
pub use transcode_ffv1_transformed as transcode_mp4_ffv1_transformed;


/// Concatenate compressed MP4 packets, or decode compatible single audio inputs
/// independently into PCM Matroska when packet-copy admission is unavailable.
/// None retains the caller's routing for unsupported sources or destinations.
pub fn try_concat_mp4_matroska(sources: &[PathBuf], destination: &Path,
    cancel: Option<&crate::media_control::CancelFlag>, progress: Option<&crate::media_control::ProgressHook>,
) -> Result<Option<crate::media_control::ProgressEvent>> {
    let mut options = fvid_media::CopyOptions::default();
    options.cancel = cancel.cloned();
    options.progress = progress.cloned();
    try_concat_mp4_matroska_with_options(sources, destination, &options)
}

pub fn try_concat_mp4_matroska_with_options(
    sources: &[PathBuf],
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<Option<crate::media_control::ProgressEvent>> {
    let cancel = options.cancel.as_ref();
    let progress = options.progress.as_ref();
    if cancel.is_some_and(|c| c.is_cancelled()) {
        return Err(invalid("media operation cancelled"));
    }
    if options.max_packet_bytes == 0 {
        return Err(invalid("packet byte limit must be positive"));
    }
    if options.max_packets == Some(0) {
        return Err(invalid("packet work limit requires at least one packet"));
    }
    if options.max_controlled_bytes == Some(0) {
        return Err(invalid("controlled memory limit must be positive"));
    }
    fvid_media::owned_budget::check_rss_budget(options).map_err(|e| invalid(&e))?;
    let ext = destination.extension().and_then(|s| s.to_str());
    let matroska_out = match ext {
        Some("mkv" | "mka") => true,
        Some("wav") => false,
        _ => return Ok(None),
    };
    // Keep ADTS on its packet-copy route instead of the decoded PCM fallback.
    if !sources.is_empty() && sources.iter().all(|source| is_adts_source(source).unwrap_or(false)) {
        return Ok(None);
    }
    if !matroska_out
        && !sources.is_empty()
        && sources
            .iter()
            .all(|source| crate::native_pcm::is_wave(source).unwrap_or(false))
        && crate::native_pcm::concat_info_with_cancel(sources, cancel).is_ok()
    {
        return Ok(None);
    }
    let segments = if matroska_out {
        crate::container::mp4_concat::open_with_options(sources, options)?
    } else {
        None
    };
    let Some(mut segments) = segments else {
        let mut default_opts = fvid_media::CopyOptions::default();
        default_opts.cancel = options.cancel.clone();
        default_opts.max_rss_bytes = options.max_rss_bytes;
        if !crate::native_audio_mix::concat_eligible_with_options(sources, &default_opts)? {
            return Ok(None);
        }
        let stats =
            crate::native_audio_mix::concat_audio_with_options(sources, destination, options)?;
        return Ok(Some(crate::media_control::ProgressEvent {
            packets: stats.decoded_frames,
            payload_bytes: stats.sample_frames * u64::from(stats.channels) * 4,
            done: true,
        }));
    };
    let selected = crate::container::mp4_matroska::selection(
        segments[0].reader.tracks().len(),
        &options.streams,
    )?;
    if destination.extension().and_then(|s|s.to_str()) == Some("mka")
        && selected
            .iter()
            .any(|&i| segments[0].reader.tracks()[i].handler != *b"soun")
    {
        return Err(invalid(".mka concat output requires audio-only input"));
    }
    let directory=destination.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let (temporary,file)=(0..100).find_map(|_| {
        let path=directory.join(format!(".fvid-mp4-concat-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file)=>Some(Ok((Temporary(path),file))),
            Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>None,
            Err(e)=>Some(Err(e)),
        }
    }).ok_or_else(||invalid("cannot reserve concat output"))??;
    let mut output=BufWriter::new(file);
    let event=crate::container::mp4_concat::write_with_options(&mut segments,&mut output,options)?;
    output.flush()?;output.get_ref().sync_all()?;drop(output);
    if cancel.is_some_and(|c|c.is_cancelled()) {return Err(invalid("media operation cancelled"));}
    std::fs::hard_link(&temporary.0,destination)?;
    let event=crate::media_control::ProgressEvent {done:true,..event};
    if let Some(hook)=progress {hook.emit(event);}
    Ok(Some(event))
}
