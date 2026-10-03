//! Native plain SRT burn-in into Y4M frames and atomic FFV1/Matroska export.
use crate::{owned_matroska as mkv, owned_y4m as y4m};
use fvid_control::CopyOptions;
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn prepare(
    source: &Path,
    subs: &Path,
) -> Result<(y4m::Header, Vec<crate::owned_play_controls::SubtitleCue>)> {
    let header = if crate::owned_y4m_decode::supports(source) {
        let mut reader = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
        let mut line = Vec::new();
        if !y4m::line(&mut reader, &mut line)? {
            return Err("empty subtitle burn input".into());
        }
        let header = y4m::Header::parse(&line)?;
        header.frame_rate()?;
        header
    } else {
        let reader = crate::owned_webm::WebmReader::open(
            BufReader::new(File::open(source).map_err(|e| e.to_string())?),
            Default::default(),
        )
        .map_err(|e| e.to_string())?;
        let track = reader
            .tracks
            .first()
            .ok_or("subtitle burn source has no video")?;
        if track.codec != "V_FFV1" || reader.tracks.len() != 1 {
            return Err("owned subtitle burn requires Y4M or single-track FFV1".into());
        }
        y4m::Header::parse(
            format!(
                "YUV4MPEG2 W{} H{} F25:1 Ip C420\n",
                track.width, track.height
            )
            .as_bytes(),
        )?
    };
    if std::fs::metadata(subs).map_err(|e| e.to_string())?.len() > 1 << 20 {
        return Err("subtitle text exceeds 1 MiB".into());
    }
    let text = std::fs::read_to_string(subs).map_err(|e| e.to_string())?;
    if !text.contains("-->") {
        return Err("owned burn-in currently requires plain SRT".into());
    }
    let cues = crate::owned_play_controls::parse_subtitle_text(&text)?;
    Ok((header, cues))
}
fn policy(options: &CopyOptions) -> Result<()> {
    if !(options.streams.is_empty() || options.streams == [0])
        || options.max_controlled_bytes.is_some()
        || !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty()
    {
        return Err(
            "owned subtitle burn-in does not implement requested metadata/memory policy".into(),
        );
    }
    crate::owned_budget::check_rss_budget(options)
}
pub fn supports(source: &Path, subs: &Path, options: &CopyOptions) -> bool {
    policy(options).is_ok()
        && (crate::owned_y4m_decode::supports(source)
            || crate::owned_ffv1_export::supports(source, &Default::default()))
        && subs
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("srt"))
}
pub fn plan_burn_subtitles(
    source: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan> {
    policy(options)?;
    let (header, cues) = prepare(source, subs)?;
    let font_size = (header.height as f32 / 18.0).clamp(12.0, 64.0);
    // Qualify font coverage in the same renderer that execution uses.
    for cue in &cues {
        crate::owned_text_raster::rasterize(&cue.text, header.width, header.height, font_size)?;
    }
    use fvid_media_info::{MediaPlan, PlanStep};
    Ok(MediaPlan { command: "burn-subtitles".into(), input: source.into(), inputs: vec![source.into(), subs.into()],
        streams: Vec::new(), graph: None, steps: vec![
            PlanStep { action: "decode".into(), detail: "owned Y4M frames and SRT cues".into() },
            PlanStep { action: "burn".into(), detail: "bundled Ubuntu plain-text mask, native YUV composition".into() },
            PlanStep { action: "encode".into(), detail: "owned FFV1 and Matroska".into() },
        ], notes: vec!["no FFmpeg or libav execution; ASS styling and shaping are not implemented by this path".into()] })
}
pub fn burn_subtitles(
    source: &Path,
    destination: &Path,
    subs: &Path,
    options: &CopyOptions,
) -> Result<fvid_media_info::LosslessStats> {
    policy(options)?;
    let (header, cues) = prepare(source, subs)?;
    let font_size = (header.height as f32 / 18.0).clamp(12.0, 64.0);
    if !crate::owned_y4m_decode::supports(source) {
        let mut process =
            |frame: &mut crate::owned_frame::GeometryFrame, depth, pts, full_range| -> Result<()> {
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
            source,
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
    let input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
    let (stats, event, consumed) = mkv::export_atomic(
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        |file| {
            let track = mkv::TrackSpec {
                encoding: mkv::Encoding::Ffv1V1 {
                    width: header.width as u32,
                    height: header.height as u32,
                },
                name: "",
                language: "und",
            };
            let metadata = mkv::TrackOptions {
                video: Some(mkv::VideoMetadata {
                    pixel_aspect: header.pixel_aspect().map_err(mkv::Error)?,
                    colour: Some(mkv::ColourDescription {
                        matrix: 6,
                        full_range: header.full_range().map_err(mkv::Error)?,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let mut writer = mkv::PacketWriter::new_with_options(file, &[track], &[metadata])?;
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
                        return Err("operation cancelled".into());
                    }
                    crate::owned_budget::check_rss_budget(options)?;
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
