//! Native plain SRT burn-in into Y4M frames and atomic FFV1/Matroska export.
use crate::{owned_matroska as mkv, owned_y4m as y4m};
use fvid_control::CopyOptions;
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn prepare(
    source: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<(
    y4m::Header,
    Vec<crate::owned_play_controls::SubtitleCue>,
    Option<crate::owned_mp4_video_bridge::Temporary>,
)> {
    let mut bridge = None;
    let header = if crate::owned_y4m_decode::supports(source) {
        let mut reader = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        if !y4m::line(&mut reader, &mut line)? {
            return Err("empty subtitle burn input".into());
        }
        let header = y4m::Header::parse(&line)?;
        header.frame_rate()?;
        let frame_len = header.frame_len()?;
        if frame_len > options.max_packet_bytes {
            return Err("Y4M frame byte length exceeds packet byte limit".into());
        }
        let mask_bytes = header.width.saturating_mul(header.height);
        if options
            .max_controlled_bytes
            .is_some_and(|limit| frame_len.saturating_add(mask_bytes) > limit)
        {
            return Err("subtitle burn frame exceeds controlled memory budget".into());
        }
        header
    } else {
        if crate::owned_mp4_video_bridge::recognizes(source) {
            bridge = crate::owned_mp4_video_bridge::prepare(source, Some(options))?;
        }
        let container_source = bridge.as_ref().map_or(source, |t| t.path.as_path());
        let mut limits = crate::owned_webm::Limits::default();
        let effective_packet_limit = options
            .max_controlled_bytes
            .map_or(options.max_packet_bytes, |limit| {
                options.max_packet_bytes.min(limit)
            });
        limits.packet_bytes = limits.packet_bytes.min(effective_packet_limit);
        let memory_packets = options
            .max_controlled_bytes
            .map(|bytes| bytes / std::mem::size_of::<crate::owned_webm::Packet>());
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: crate::owned_webm::Error| {
            if memory_packets.is_some_and(|max| max < crate::owned_webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                "controlled memory budget exceeded: Matroska packet index limit".to_owned()
            } else {
                error.to_string()
            }
        };
        let mut reader = crate::owned_webm::WebmReader::open(
            BufReader::new(File::open(container_source).map_err(|e| e.to_string())?),
            limits,
        )
        .map_err(map_packet_limit)?;
        reader.restrict_packet_bytes(effective_packet_limit);
        let track = reader
            .tracks
            .first()
            .ok_or("subtitle burn source has no video")?;
        if reader.tracks.len() != 1
            || track.kind != 1
            || !crate::owned_ffv1_export::supports_with_options(
                container_source,
                &Default::default(),
                Some(options),
            )
        {
            return Err(
                "owned subtitle burn requires Y4M or supported single-track Matroska/MP4 video"
                    .into(),
            );
        }
        let (mut width, mut height) = track.visible();
        if matches!(track.rotation, 90 | 270) {
            std::mem::swap(&mut width, &mut height);
        }
        let mask_bytes = (width as usize).saturating_mul(height as usize);
        if options
            .max_controlled_bytes
            .is_some_and(|limit| mask_bytes.saturating_mul(2) > limit)
        {
            return Err("subtitle burn frame exceeds controlled memory budget".into());
        }
        y4m::Header::parse(
            format!(
                "YUV4MPEG2 W{} H{} F25:1 Ip C420\n",
                width, height
            )
            .as_bytes(),
        )?
    };
    let subs_len = std::fs::metadata(subs).map_err(|e| e.to_string())?.len();
    if subs_len > 1 << 20 {
        return Err("subtitle text exceeds 1 MiB".into());
    }
    let subs_bytes = usize::try_from(subs_len).map_err(|_| "subtitle text exceeds 1 MiB")?;
    if let Some(limit) = options.max_controlled_bytes.filter(|&limit| limit > 0)
        && subs_bytes.saturating_mul(3) > limit
    {
        return Err("controlled memory budget exceeded: subtitle burn SRT working set".into());
    }
    let text = std::fs::read_to_string(subs).map_err(|e| e.to_string())?;
    if !text.contains("-->") {
        return Err("owned burn-in currently requires plain SRT".into());
    }
    let cues = crate::owned_play_controls::parse_subtitle_text(&text)?;
    if let Some(limit) = options.max_controlled_bytes.filter(|&limit| limit > 0) {
        let cues_bytes = cues
            .iter()
            .fold(
                cues.len()
                    .saturating_mul(std::mem::size_of::<crate::owned_play_controls::SubtitleCue>()),
                |acc, cue| acc.saturating_add(cue.text.len()),
            )
            .saturating_add(text.len());
        if cues_bytes > limit {
            return Err("controlled memory budget exceeded: subtitle burn SRT working set".into());
        }
    }
    Ok((header, cues, bridge))
}
fn policy(options: &CopyOptions) -> Result<()> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    if !crate::owned_lossless::policy(options, true) {
        return Err(
            "owned subtitle burn-in does not implement requested metadata/memory policy".into(),
        );
    }
    crate::owned_lossless::metadata(options)?;
    crate::owned_lossless::check_options_matroska_metadata_budget(options)?;
    crate::owned_budget::check_rss_budget(options)
}
pub fn supports(source: &Path, subs: &Path, options: &CopyOptions) -> bool {
    !options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
        && (options.streams.is_empty() || options.streams == [0])
        && options.max_packet_bytes != 0
        && options.max_packets != Some(0)
        && options.max_controlled_bytes != Some(0)
        && crate::owned_budget::check_rss_budget(options).is_ok()
        && crate::owned_lossless::policy(options, true)
        && crate::owned_lossless::metadata(options).is_ok()
        && (crate::owned_y4m_decode::supports(source)
            || crate::owned_ffv1_export::supports_with_options(
                source,
                &Default::default(),
                Some(options),
            )
            || match crate::owned_mp4_video_bridge::prepare(source, Some(options)) {
                Ok(Some(temporary)) => crate::owned_ffv1_export::supports_with_options(
                    &temporary.path,
                    &Default::default(),
                    Some(options),
                ),
                Err(_) if crate::owned_mp4_video_bridge::recognizes(source) => true,
                _ => false,
            })
        && subs
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("srt"))
}
pub fn plan_burn_subtitles(
    source: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    policy(options)?;
    let (header, cues, bridge) = prepare(source, subs, options)?;
    let font_size = (header.height as f32 / 18.0).clamp(12.0, 64.0);
    // Qualify font coverage in the same renderer that execution uses.
    for cue in &cues {
        crate::owned_text_raster::rasterize(&cue.text, header.width, header.height, font_size)?;
    }
    if crate::owned_y4m_decode::supports(source) {
        let encode_packets =
            options.max_packet_bytes != CopyOptions::default().max_packet_bytes
                || options.max_controlled_bytes.is_some();
        let mut reader = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
        let mut visit = |h: &y4m::Header, bytes: &[u8], pts: u64, _: u64| {
            if options
                .cancel
                .as_ref()
                .is_some_and(fvid_control::CancelFlag::is_cancelled)
            {
                return Err("media operation cancelled".into());
            }
            crate::owned_budget::check_rss_budget(options)?;
            let frame_len = h.frame_len()?;
            if frame_len > options.max_packet_bytes {
                return Err("Y4M frame byte length exceeds packet byte limit".into());
            }
            let mask_bytes = h.width.saturating_mul(h.height);
            if options
                .max_controlled_bytes
                .is_some_and(|limit| frame_len.saturating_add(mask_bytes) > limit)
            {
                return Err("subtitle burn frame exceeds controlled memory budget".into());
            }
            if encode_packets {
                let (sx, sy) = h.format.subsampling();
                let mut frame = crate::owned_frame::GeometryFrame {
                    width: h.width,
                    height: h.height,
                    subsampling: Some([sx, sy]),
                    data: bytes.to_vec(),
                };
                let text = cues
                    .iter()
                    .filter(|cue| {
                        cue.start_us >= 0
                            && pts as u128 >= cue.start_us as u128 * 1000
                            && (pts as u128) < cue.end_us as u128 * 1000
                    })
                    .map(|cue| cue.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    let mask =
                        crate::owned_text_raster::rasterize(&text, h.width, h.height, font_size)?;
                    crate::owned_text_raster::composite_white(
                        &mut frame,
                        h.depth(),
                        h.full_range()?,
                        &mask,
                    )?;
                }
                let packet = crate::owned_ffv1_encoder::encode(&frame, h.depth())?;
                if packet.len() > options.max_packet_bytes {
                    return Err("subtitle burn packet exceeds byte limit".into());
                }
                if options
                    .max_controlled_bytes
                    .is_some_and(|limit| packet.len() > limit)
                {
                    return Err("subtitle burn packet exceeds controlled memory budget".into());
                }
            }
            Ok(())
        };
        let (stats, _) = crate::owned_y4m_decode::visit_reader_transformed_counted(
            &mut reader,
            &Default::default(),
            options.max_packets,
            &mut visit,
        )?;
        if stats.video_frames == 0 {
            return Err("subtitle burn input has no video frames".into());
        }
    } else {
        let container_source = bridge.as_ref().map_or(source, |t| t.path.as_path());
        crate::owned_lossless::check_source_matroska_metadata_budget(
            container_source,
            None,
            options,
        )?;
        let mut stage = options.clone();
        stage.streams.clear();
        stage.stream_metadata_set.clear();
        stage.stream_metadata_delete.clear();
        let encode_packets =
            options.max_packet_bytes != CopyOptions::default().max_packet_bytes
                || options.max_controlled_bytes.is_some();
        let mut selected_frames = 0u64;
        let mut inspect = |view: crate::owned_video_decode::FrameView<'_>| -> Result<()> {
            selected_frames += 1;
            crate::owned_budget::check_rss_budget(options)?;
            let mask_bytes = (view.width as usize).saturating_mul(view.height as usize);
            if options
                .max_controlled_bytes
                .is_some_and(|limit| view.pixels.len().saturating_add(mask_bytes) > limit)
            {
                return Err("subtitle burn frame exceeds controlled memory budget".into());
            }
            if !encode_packets {
                return Ok(());
            }
            let mut data = crate::owned_frame::buffer(view.pixels.len())?;
            data.copy_from_slice(view.pixels);
            let mut working = crate::owned_frame::GeometryFrame {
                width: view.width as usize,
                height: view.height as usize,
                subsampling: view.subsampling,
                data,
            };
            let text = active_text(&cues, view.pts_ns);
            if !text.is_empty() {
                let mask = crate::owned_text_raster::rasterize(
                    &text,
                    working.width,
                    working.height,
                    font_size,
                )?;
                crate::owned_text_raster::composite_white(
                    &mut working,
                    view.depth,
                    view.full_range,
                    &mask,
                )?;
            }
            let packet = if view.monochrome {
                let samples = working
                    .width
                    .checked_mul(working.height)
                    .and_then(|v| v.checked_mul(if view.depth == 8 { 1 } else { 2 }))
                    .ok_or("FFV1 gray size overflow")?;
                match view.alpha {
                    Some(alpha) => crate::owned_ffv1_encoder::encode_gray_with_alpha(
                        working.width,
                        working.height,
                        &working.data[..samples],
                        alpha,
                        view.depth,
                    )?,
                    None => crate::owned_ffv1_encoder::encode_gray(
                        working.width,
                        working.height,
                        &working.data[..samples],
                        view.depth,
                    )?,
                }
            } else {
                match view.alpha {
                    Some(alpha) => {
                        crate::owned_ffv1_encoder::encode_with_alpha(&working, view.depth, alpha)?
                    }
                    None => crate::owned_ffv1_encoder::encode(&working, view.depth)?,
                }
            };
            if packet.len() > options.max_packet_bytes {
                return Err("encoded FFV1 packet exceeds byte limit".into());
            }
            if options
                .max_controlled_bytes
                .is_some_and(|limit| packet.len() > limit)
            {
                return Err("subtitle burn packet exceeds controlled memory budget".into());
            }
            Ok(())
        };
        let result = crate::owned_video_decode::decode_webm(
            container_source,
            &Default::default(),
            Some(&mut inspect),
            Some(&stage),
        )?;
        drop(inspect);
        result.ok_or("unsupported owned FFV1 export capability")?;
        if selected_frames == 0 {
            return Err("Matroska has no selected frames".into());
        }
    }
    let info = crate::owned_probe::probe_with_options(source, options)?;
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    let streams = info
        .streams
        .into_iter()
        .map(|stream| PlanStream {
            index: stream.index,
            media_type: stream.media_type,
            codec: stream.codec,
            disposition: "primary_video".into(),
        })
        .collect();
    let mut steps = vec![
        PlanStep {
            action: "decode".into(),
            detail: "owned Y4M frames and SRT cues".into(),
        },
        PlanStep {
            action: "burn".into(),
            detail: "bundled Ubuntu plain-text mask, native YUV composition".into(),
        },
        PlanStep {
            action: "encode".into(),
            detail: "owned FFV1 and Matroska".into(),
        },
    ];
    if !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        steps.push(PlanStep {
            action: "metadata".into(),
            detail: format!(
                "container set={}, delete={}; stream set={}, delete={}",
                options.metadata_set.len(),
                options.metadata_delete.len(),
                options.stream_metadata_set.len(),
                options.stream_metadata_delete.len()
            ),
        });
    }
    let mut notes = vec![
        "no FFmpeg or libav execution; ASS styling and shaping are not implemented by this path"
            .into(),
    ];
    if options.max_packet_bytes != CopyOptions::default().max_packet_bytes
        || options.max_packets.is_some()
        || options.max_controlled_bytes.is_some()
        || options.max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?} bytes; RSS limit: {:?} bytes",
            options.max_packet_bytes,
            options.max_packets,
            options.max_controlled_bytes,
            options.max_rss_bytes
        ));
    }
    Ok(MediaPlan {
        command: "burn-subtitles".into(),
        input: source.into(),
        inputs: vec![source.into(), subs.into()],
        streams,
        graph: None,
        steps,
        notes,
    })
}
pub fn burn_subtitles(
    source: &Path,
    destination: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::LosslessStats> {
    policy(options)?;
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    let (header, cues, bridge) = prepare(source, subs, options)?;
    let font_size = (header.height as f32 / 18.0).clamp(12.0, 64.0);
    if !crate::owned_y4m_decode::supports(source) {
        let container_source = bridge.as_ref().map_or(source, |t| t.path.as_path());
        crate::owned_lossless::check_source_matroska_metadata_budget(
            container_source,
            None,
            options,
        )?;
        let mut process =
            |frame: &mut crate::owned_frame::GeometryFrame, depth, pts, full_range| -> Result<()> {
                let mask_bytes = frame.width.saturating_mul(frame.height);
                if options
                    .max_controlled_bytes
                    .is_some_and(|limit| frame.data.len().saturating_add(mask_bytes) > limit)
                {
                    return Err("subtitle burn frame exceeds controlled memory budget".into());
                }
                let text = active_text(&cues, pts);
                if !text.is_empty() {
                    let mask = crate::owned_text_raster::rasterize(
                        &text,
                        frame.width,
                        frame.height,
                        font_size,
                    )?;
                    crate::owned_text_raster::composite_white(frame, depth, full_range, &mask)?;
                }
                Ok(())
            };
        let (stats, event, consumed) = crate::owned_ffv1_export::export_processed(
            container_source,
            destination,
            &Default::default(),
            options,
            Some(&mut process),
        )?;
        return Ok(fvid_media_info::LosslessStats {
            backend: "owned SRT burn-in FFV1 export",
            video_frames: stats.video_frames,
            decoded_frames: consumed,
            seek_used: false,
            video_packets: event.packets,
            copied_packets: 0,
            trimmed_audio_sample_frames: 0,
            pixel_format: stats.pixel_format,
            encoder: "ffv1".into(),
            fvid_crop_payload_copies: 0,
            vertical_flip: false,
            horizontal_flip: false,
        });
    }
    let file_metadata = crate::owned_lossless::metadata(options)?;
    let mut text_tags = std::collections::BTreeMap::new();
    for (key, value) in &options.metadata_set {
        text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
        if !crate::owned_file_tags::FileTags::supports_key(key) && !value.is_empty() {
            text_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for key in &options.metadata_delete {
        text_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
    }
    let mut description = mkv::VideoTrackDescription {
        name: String::new(),
        language: "und".into(),
        legacy_language: "und".into(),
        disposition: 1,
    };
    let mut track_tags = std::collections::BTreeMap::new();
    for (_, key, value) in &options.stream_metadata_set {
        track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
        if !crate::owned_ffv1_export::edit_track_description(&mut description, key, value)
            && !value.is_empty()
        {
            track_tags.insert(key.to_ascii_uppercase(), value.clone());
        }
    }
    for (_, key) in &options.stream_metadata_delete {
        track_tags.retain(|name: &String, _| !name.eq_ignore_ascii_case(key));
        crate::owned_ffv1_export::edit_track_description(&mut description, key, "");
    }
    if let Some(limit) = options.max_controlled_bytes.filter(|&limit| limit > 0)
        && crate::owned_lossless::estimate_matroska_metadata_working_bytes(
            &file_metadata,
            &text_tags,
            &track_tags,
            Some(&description),
        ) > limit
    {
        return Err("Matroska metadata working set exceeds controlled memory budget".into());
    }
    let input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let (stats, event, consumed) = mkv::export_atomic(
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        |file| {
            let video_metadata = mkv::VideoMetadata {
                pixel_aspect: header.pixel_aspect().map_err(mkv::Error)?,
                colour: Some(mkv::ColourDescription {
                    matrix: 6,
                    full_range: header.full_range().map_err(mkv::Error)?,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut writer = mkv::PacketWriter::new_ffv1_with_track_description(
                file,
                header.width as u32,
                header.height as u32,
                Some(&video_metadata),
                0,
                0,
                &file_metadata,
                &text_tags,
                &track_tags,
                &description,
            )?;
            let mut event = fvid_control::ProgressEvent {
                packets: 0,
                payload_bytes: 0,
                done: false,
            };
            let (stats, consumed) = crate::owned_y4m_decode::visit_reader_transformed_counted(
                input,
                &Default::default(),
                options.max_packets,
                &mut |h, bytes, pts, duration| {
                    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
                        return Err("media operation cancelled".into());
                    }
                    crate::owned_budget::check_rss_budget(options)?;
                    let frame_len = h.frame_len()?;
                    if frame_len > options.max_packet_bytes {
                        return Err("Y4M frame byte length exceeds packet byte limit".into());
                    }
                    let mask_bytes = h.width.saturating_mul(h.height);
                    if options
                        .max_controlled_bytes
                        .is_some_and(|limit| frame_len.saturating_add(mask_bytes) > limit)
                    {
                        return Err("subtitle burn frame exceeds controlled memory budget".into());
                    }
                    let (sx, sy) = h.format.subsampling();
                    let mut frame = crate::owned_frame::GeometryFrame {
                        width: h.width,
                        height: h.height,
                        subsampling: Some([sx, sy]),
                        data: bytes.to_vec(),
                    };
                    let text = cues
                        .iter()
                        .filter(|cue| {
                            cue.start_us >= 0
                                && pts as u128 >= cue.start_us as u128 * 1000
                                && (pts as u128) < cue.end_us as u128 * 1000
                        })
                        .map(|cue| cue.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        let mask = crate::owned_text_raster::rasterize(
                            &text, h.width, h.height, font_size,
                        )?;
                        crate::owned_text_raster::composite_white(
                            &mut frame,
                            h.depth(),
                            h.full_range()?,
                            &mask,
                        )?;
                    }
                    let packet = crate::owned_ffv1_encoder::encode(&frame, h.depth())?;
                    if packet.len() > options.max_packet_bytes {
                        return Err("subtitle burn packet exceeds byte limit".into());
                    }
                    if options
                        .max_controlled_bytes
                        .is_some_and(|limit| packet.len() > limit)
                    {
                        return Err(
                            "subtitle burn packet exceeds controlled memory budget".into(),
                        );
                    }
                    writer
                        .write_packet(0, pts, duration, true, &packet)
                        .map_err(|e| e.to_string())?;
                    event.packets += 1;
                    event.payload_bytes += packet.len() as u64;
                    if let Some(progress) = &options.progress {
                        progress.emit(event);
                    }
                    Ok(())
                },
            )
            .map_err(mkv::Error)?;
            if stats.video_frames == 0 {
                return Err(mkv::Error("subtitle burn input has no video frames".into()));
            }
            writer.finish()?;
            Ok((stats, event, consumed))
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(fvid_media_info::LosslessStats {
        backend: "owned SRT burn-in FFV1 export",
        video_frames: stats.video_frames,
        decoded_frames: consumed,
        seek_used: false,
        video_packets: event.packets,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: stats.pixel_format,
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: false,
        horizontal_flip: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_srt_burn_respects_half_open_cue_and_preserves_timestamps() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("subtitle-burn.y4m");
        let subs = root.join("subtitle-burn.srt");
        let output = std::env::temp_dir().join(format!("fvid-burn-{}.mkv", std::process::id()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(output.clone());
        let plan = plan_burn_subtitles(&source, &subs, &Default::default()).unwrap();
        assert!(plan.graph.is_none());
        assert!(!output.exists());
        let stats = burn_subtitles(&source, &output, &subs, &Default::default()).unwrap();
        assert_eq!(stats.video_frames, 3);
        let mut reader =
            crate::owned_webm::WebmReader::open(File::open(&output).unwrap(), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        let mut decoder = crate::owned_ffv1_decoder::Decoder::new(96, 64, 1 << 20).unwrap();
        let packets = reader.packets.clone();
        let mut original = vec![32; 96 * 64];
        original.extend(vec![64; 2 * 48 * 32]);
        for (index, packet) in packets.iter().enumerate() {
            assert_eq!(packet.pts_ns, index as i64 * 500_000_000);
            let bytes = reader.read_packet(index).unwrap();
            let picture = decoder.decode(&bytes).unwrap();
            if index == 1 {
                assert_ne!(picture.frame.data, original);
            } else {
                assert_eq!(picture.frame.data, original);
            }
        }
        assert!(burn_subtitles(&source, &output, &subs, &Default::default()).is_err());
        let compressed = output.with_extension("source.mkv");
        let second = output.with_extension("second.mkv");
        let _compressed_cleanup = Cleanup(compressed.clone());
        let _second_cleanup = Cleanup(second.clone());
        crate::owned_lossless::transcode_lossless(
            &source,
            &compressed,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert!(supports(&compressed, &subs, &Default::default()));
        assert!(
            plan_burn_subtitles(&compressed, &subs, &Default::default())
                .unwrap()
                .graph
                .is_none()
        );
        burn_subtitles(&compressed, &second, &subs, &Default::default()).unwrap();
        let mut compressed_reader =
            crate::owned_webm::WebmReader::open(File::open(&second).unwrap(), Default::default())
                .unwrap();
        compressed_reader.scan_all().unwrap();
        assert_eq!(compressed_reader.packets.len(), 3);
        for index in 0..3 {
            assert_eq!(
                compressed_reader.packets[index].pts_ns,
                reader.packets[index].pts_ns
            );
            assert_eq!(
                compressed_reader.read_packet(index).unwrap(),
                reader.read_packet(index).unwrap()
            );
        }
    }
}

fn active_text(cues: &[crate::owned_play_controls::SubtitleCue], pts_ns: i64) -> String {
    cues.iter()
        .filter(|cue| {
            i128::from(pts_ns) >= i128::from(cue.start_us) * 1000
                && i128::from(pts_ns) < i128::from(cue.end_us) * 1000
        })
        .map(|cue| cue.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
