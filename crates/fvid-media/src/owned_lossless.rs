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
        rotate: t.rotate,
        pad: t.pad.clone(),
        interval: t.interval,
        framestep: t.framestep.clone(),
        shuffleframes: t.shuffleframes.clone(),
        reverse: t.reverse.clone(),
        overlay: t.overlay.clone(),
        avgblur: t.avgblur.clone(),
        gblur: t.gblur.clone(),
        boxblur: t.boxblur.clone(),
        unsharp: t.unsharp.clone(),
        eq: t.eq.clone(),
        hue: t.hue.clone(),
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
        rotate: t.rotate,
        pad: t.pad.clone(),
        interval: t.interval,
        framestep: t.framestep.clone(),
        shuffleframes: t.shuffleframes.clone(),
        reverse: t.reverse.clone(),
        overlay: t.overlay.clone(),
        avgblur: t.avgblur.clone(),
        gblur: t.gblur.clone(),
        boxblur: t.boxblur.clone(),
        unsharp: t.unsharp.clone(),
        eq: t.eq.clone(),
        hue: t.hue.clone(),
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
pub(crate) fn metadata(o: &CopyOptions) -> Result<crate::owned_matroska::FileMetadata, String> {
    if o.metadata_set.len() + o.metadata_delete.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    let mut file = crate::owned_matroska::FileMetadata::default();
    for key in &o.metadata_delete {
        if key.contains('\0') { return Err("NUL in container metadata".into()); }
        file.tags.set(key, "");
    }
    for (key, value) in &o.metadata_set {
        if key.contains('\0') || value.contains('\0') {
            return Err("NUL in container metadata".into());
        }
        // Nonstandard text tags are written alongside these standard fields.
        file.tags.set(key, value);
    }
    Ok(file)
}
fn stream_tag_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && !key.contains('\0')
        && !matches!(
            key.to_ascii_lowercase().as_str(),
            "rotate" | "stereo_mode" | "alpha_mode"
        )
}
fn policy(o: &CopyOptions, text_tags: bool) -> bool {
    (o.streams.is_empty() || o.streams == [0])
        && o.max_packet_bytes != 0
        && o.max_controlled_bytes.is_none()
        && o.max_rss_bytes.is_none()
        && o.metadata_set.len() + o.metadata_delete.len() <= 64
        && o.metadata_set.iter().all(|(k, v)| {
            !k.contains('\0')
                && !v.contains('\0')
                && (crate::owned_file_tags::FileTags::supports_key(k)
                    || (text_tags && !k.is_empty() && k.len() <= 128 && v.len() <= 1024))
        })
        && o.metadata_delete.iter().all(|k| {
            !k.contains('\0')
                && (crate::owned_file_tags::FileTags::supports_key(k)
                    || (text_tags && !k.is_empty() && k.len() <= 128))
        })
        && if text_tags {
            o.stream_metadata_set.len() + o.stream_metadata_delete.len() <= 64
                && o.stream_metadata_set.iter().all(|(index, key, value)| {
                    *index == 0
                        && stream_tag_key(key)
                        && !value.contains('\0')
                        && value.len() <= 1024
                        && (!key.eq_ignore_ascii_case("language") || value.len() <= 128)
                })
                && o.stream_metadata_delete
                    .iter()
                    .all(|(index, key)| *index == 0 && stream_tag_key(key))
        } else {
            o.stream_metadata_set.is_empty() && o.stream_metadata_delete.is_empty()
        }
}
/// Whether the owned export pipeline admits this source, transform and policy.
pub fn supports(source: &Path, t: &LosslessTransform, o: &CopyOptions) -> bool {
    if policy(o, true) && request(t).is_some_and(|r| crate::owned_ffv1_export::supports(source, &r)) {
        return true;
    }
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
    policy(o, true) && metadata(o).is_ok()
        && request(t).is_some_and(|r| crate::owned_y4m_decode::supports_transformed(source, &r))
}
pub fn transcode_lossless(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    if options
        .cancel
        .as_ref()
        .is_some_and(fvid_control::CancelFlag::is_cancelled)
    {
        return Err("media operation cancelled".into());
    }
    let request = request(&transform)
        .ok_or("owned Y4M lossless export does not yet implement requested transforms")?;
    if policy(options, true) && crate::owned_ffv1_export::supports(source, &request) {
        let (stats, event, consumed) =
            crate::owned_ffv1_export::export(source, destination, &request, options)?;
        return Ok(LosslessStats {
            backend: "fvid",
            video_frames: stats.video_frames,
            decoded_frames: consumed,
            seek_used: false,
            video_packets: event.packets,
            copied_packets: 0,
            trimmed_audio_sample_frames: 0,
            pixel_format: stats.pixel_format,
            encoder: "ffv1".into(),
            fvid_crop_payload_copies: 0,
            vertical_flip: transform.vertical_flip,
            horizontal_flip: transform.horizontal_flip,
        });
    }
    let file_metadata = metadata(options)?;
    if !policy(options, true) {
        return Err("owned Y4M lossless export does not yet implement requested policy".into());
    }
    if let Some((from, to)) = request.interval {
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
            .map_err(|_| "interval timestamp overflow")?;
    }
    let (stats, event, consumed) = crate::owned_matroska::export_y4m_ffv1_policy(
        source,
        destination,
        &request,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        options.max_packet_bytes,
        options.max_packets,
        true,
        &file_metadata,
        Some(options),
    )
    .map_err(|e| e.to_string())?;
    Ok(LosslessStats {
        backend: "fvid",
        video_frames: stats.video_frames,
        decoded_frames: consumed,
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

/// Settings implemented by the owned version-1 FFV1 encoder.
/// Unknown options never become silently ignored codec requests.
pub fn supports_encoder(settings: &fvid_media_info::EncoderSettings) -> bool {
    settings.validate().is_ok() && settings.name == "ffv1"
        && settings.options.iter().all(|(key, value)| key == "level" && value == "1")
}

/// Explicit FFV1 encoding through the owned lossless pipeline.
pub fn transcode(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
    settings: &fvid_media_info::EncoderSettings,
) -> Result<LosslessStats, String> {
    settings.validate()?;
    if !supports_encoder(settings) {
        return Err("owned transcode does not yet implement requested encoder/settings".into());
    }
    transcode_lossless(source, destination, transform, options)
}

/// Plan the same owned Y4M-to-FFV1 execution admitted by the export API.
/// Unsupported requests are refused rather than advertised as executable.
pub fn plan_transcode_lossless(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
    encoder: Option<&str>,
) -> Result<fvid_media_info::MediaPlan, String> {
    use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
    if encoder.is_some_and(|name| name != "ffv1") || !supports(source, transform, options) {
        return Err("request has no owned lossless export plan".into());
    }
    metadata(options)?;
    let info = crate::owned_probe::probe(source)?;
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "read source video frames with the owned container parser and decoder".into(),
    }];
    if let Some((from, to)) = transform.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        if crate::owned_y4m_decode::supports(source) {
            let mut input =
                std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?);
            let mut bytes = Vec::new();
            crate::owned_y4m::line(&mut input, &mut bytes)?;
            let header = crate::owned_y4m::Header::parse(&bytes)?;
            let [n, d] = header.frame_rate()?;
            if [from, to]
                .into_iter()
                .any(|time| time as u128 * n as u128 % (d as u128 * 1_000_000) != 0)
            {
                return Err("interval boundary is not exact in video time base".into());
            }
        }
        steps.push(PlanStep {
            action: "interval".into(),
            detail: format!("presentation window [{from},{to}) µs"),
        });
    }
    if *transform != LosslessTransform::default() {
        steps.push(PlanStep {
            action: "filter".into(),
            detail: "apply requested geometry and pixel transforms in the owned frame pipeline"
                .into(),
        });
    }
    steps.push(PlanStep {
        action: "encode".into(),
        detail: "encode FFV1 with the owned encoder".into(),
    });
    steps.push(PlanStep {
        action: "mux".into(),
        detail: "write Matroska with the owned muxer".into(),
    });
    Ok(MediaPlan {
        command: "transcode-lossless".into(), input: source.into(), inputs: std::iter::once(source.to_path_buf()).chain(transform.overlay.iter().map(|spec| spec.path.clone())).collect(),
        streams: info.streams.into_iter().map(|stream| PlanStream {
            index: stream.index, media_type: stream.media_type, codec: stream.codec, disposition: "primary_video".into(),
        }).collect(),
        steps, graph: None,
        notes: vec!["backend: owned video/FFV1/Matroska; no external demuxer, filter graph or encoder".into(),
            "destination must support the owned Matroska export; packet, cancellation and publication checks also run during execution".into()],
    })
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    fn source() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/shuffleplanes-444-8.y4m")
    }
    #[test]
    fn owned_plan_matches_executed_filter_export() {
        let source = source();
        let transform = LosslessTransform {
            negate: Some("".into()),
            ..Default::default()
        };
        let options = CopyOptions::default();
        let plan = plan_transcode_lossless(&source, &transform, &options, Some("ffv1")).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].disposition, "primary_video");
        assert_eq!(
            plan.steps
                .iter()
                .map(|step| step.action.as_str())
                .collect::<Vec<_>>(),
            ["decode", "filter", "encode", "mux"]
        );
        assert!(plan.notes[0].starts_with("backend: owned"));
        let directory =
            std::env::temp_dir().join(format!("fvid-owned-lossless-plan-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("filtered.mkv");
        let stats = transcode_lossless(&source, &output, transform.clone(), &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.encoder, "ffv1");
        assert!(stats.video_frames > 0);
        let info = crate::owned_webm_probe::probe_webm(&output).unwrap();
        assert_eq!(info.streams[0].codec, "ffv1");
        #[cfg(feature = "legacy-ffmpeg")]
        {
            let public =
                crate::plan_transcode_lossless(&source, &transform, &options, Some("ffv1"))
                    .unwrap();
            assert_eq!(public.steps, plan.steps);
            assert_eq!(public.notes, plan.notes);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn rotation_library_export_and_reexport_preserve_pixels_and_timing() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/rotate-grid.y4m");
        let directory =
            std::env::temp_dir().join(format!("fvid-library-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let options = CopyOptions::default();
        let turn = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("90").unwrap()),
            ..Default::default()
        };
        assert!(supports(&source, &turn, &options));
        plan_transcode_lossless(&source, &turn, &options, None).unwrap();
        let first = directory.join("first.mkv");
        let second = directory.join("second.mkv");
        assert_eq!(
            transcode_lossless(&source, &first, turn.clone(), &options)
                .unwrap()
                .video_frames,
            3
        );
        assert!(supports(&first, &turn, &options));
        transcode_lossless(&first, &second, turn, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &second,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (3, 2));
                assert_eq!(&frame.pixels[..6], &[6, 5, 4, 3, 2, 1]);
                assert_eq!(&frame.pixels[6..], &[128; 12]);
                assert_eq!(frame.pts_ns, frames * 500_000_000);
                assert_eq!(frame.duration_ns, Some(500_000_000));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/rotate-white420.y4m");
        let turn = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("45").unwrap()),
            ..Default::default()
        };
        std::fs::remove_file(&first).unwrap();
        transcode_lossless(&source, &first, turn, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &first,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (11, 11));
                assert_eq!(frame.pixels.len(), 193);
                assert_eq!(frame.pixels[0], 16);
                assert_eq!(frame.pixels[60], 235);
                assert!(frame.pixels[121..].iter().all(|&sample| sample == 128));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        std::fs::remove_file(&first).unwrap();
        let transform = LosslessTransform {
            rotate: Some(fvid_media_info::RotateAngle::parse("45").unwrap()),
            pad: Some(fvid_media_info::PadRect {
                width: 12,
                height: 12,
                x: 0,
                y: 0,
            }),
            scale: Some(fvid_media_info::ScaleSize {
                width: 6,
                height: 6,
            }),
            ..Default::default()
        };
        transcode_lossless(&source, &first, transform, &options).unwrap();
        let mut frames = 0;
        crate::owned_video_decode::decode_ffv1(
            &first,
            &Default::default(),
            Some(&mut |frame| {
                assert_eq!((frame.width, frame.height), (6, 6));
                assert_eq!(frame.pixels.len(), 54);
                assert!(frame.pixels[36..].iter().all(|&sample| sample == 128));
                frames += 1;
                Ok(())
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(frames, 3);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn plan_refuses_unsupported_execution_and_inexact_interval() {
        let source = source();
        let options = CopyOptions::default();
        assert!(
            plan_transcode_lossless(&source, &Default::default(), &options, Some("h264")).is_err()
        );
        let inexact = LosslessTransform {
            interval: Some((1, 40_000)),
            ..Default::default()
        };
        assert!(
            plan_transcode_lossless(&source, &inexact, &options, None)
                .unwrap_err()
                .contains("not exact")
        );
        let unsupported = LosslessTransform {
            gblur: Some("sigma=NaN".into()),
            ..Default::default()
        };
        assert!(plan_transcode_lossless(&source, &unsupported, &options, None).is_err());
    }
}

/// Composite a scheduled Y4M foreground, then export with owned FFV1/Matroska.
pub fn overlay_video(
    source: &Path,
    overlay: &Path,
    destination: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<LosslessStats, String> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            overlay: Some(fvid_media_info::OverlaySpec {
                path: overlay.into(),
                x,
                y,
            }),
            ..Default::default()
        },
        options,
    )
}
pub fn plan_overlay(
    source: &Path,
    overlay: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<fvid_media_info::MediaPlan, String> {
    let transform = LosslessTransform {
        overlay: Some(fvid_media_info::OverlaySpec {
            path: overlay.into(),
            x,
            y,
        }),
        ..Default::default()
    };
    let mut plan = plan_transcode_lossless(source, &transform, options, None)?;
    plan.command = "overlay".into();
    Ok(plan)
}

#[cfg(test)]
mod framestep_tests {
    use super::*;
    #[test]
    fn stepped_file_export_counts_consumed_frames_and_preserves_packet_timing() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("framestep-six-frames.y4m");
        for (case, interval, indices, consumed) in [
            ("full", None, vec![0u8, 2, 4], 6),
            ("range", Some((250_000, 1_000_000)), vec![1u8, 3], 4),
        ] {
            let transform = LosslessTransform {
                framestep: Some("step=2".into()),
                interval,
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-framestep-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, indices.len() as u64);
            assert_eq!(stats.video_packets, indices.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), indices.len());
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            for (slot, index) in indices.into_iter().enumerate() {
                let packet = &reader.packets[slot];
                assert_eq!(packet.pts_ns, i64::from(index) * 250_000_000 - origin);
                assert_eq!(packet.duration_ns, Some(250_000_000));
                let mut expected = vec![10 + index; 16];
                expected.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    expected
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn stepped_export_checks_discarded_payloads_and_input_packet_limit() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        for (case, name, max_packets, message) in [
            (
                "truncated",
                "framestep-discarded-truncated.y4m",
                None,
                "truncated Y4M frame payload",
            ),
            (
                "limit",
                "framestep-six-frames.y4m",
                Some(5),
                "Y4M input packet count exceeds limit",
            ),
        ] {
            let source = root.join(name);
            let output = std::env::temp_dir().join(format!(
                "fvid-framestep-refusal-{}-{case}.mkv",
                std::process::id()
            ));
            let transform = LosslessTransform {
                framestep: Some("2".into()),
                ..Default::default()
            };
            let options = CopyOptions {
                max_packets,
                ..Default::default()
            };
            assert!(supports(&source, &transform, &options));
            let error =
                crate::transcode_lossless(&source, &output, transform, &options).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(!output.exists());
        }
    }
}

#[cfg(test)]
mod shuffleframes_tests {
    use super::*;
    #[test]
    fn shuffled_files_preserve_samples_timing_and_consumed_counts() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("shuffleframes-seven-frames.y4m");
        for (case, mapping, interval, step, expected, consumed) in [
            (
                "inverse",
                "2 1 0",
                None,
                None,
                vec![(2u8, 0u64), (1, 1), (0, 2), (5, 3), (4, 4), (3, 5)],
                7,
            ),
            (
                "drops",
                "mapping=2|-1|2",
                None,
                None,
                vec![(2, 0), (2, 2), (5, 3), (5, 5)],
                7,
            ),
            (
                "range",
                "2 1 0",
                Some((250_000, 1_500_000)),
                None,
                vec![(3, 1), (2, 2), (1, 3)],
                6,
            ),
            (
                "step",
                "2 1 0",
                None,
                Some("2"),
                vec![(4, 0), (2, 2), (0, 4)],
                7,
            ),
        ] {
            let transform = LosslessTransform {
                shuffleframes: Some(mapping.into()),
                interval,
                framestep: step.map(String::from),
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-shuffle-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, expected.len() as u64);
            assert_eq!(stats.video_packets, expected.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), expected.len());
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            for (slot, (value, position)) in expected.into_iter().enumerate() {
                assert_eq!(
                    reader.packets[slot].pts_ns,
                    position as i64 * 250_000_000 - origin
                );
                assert_eq!(reader.packets[slot].duration_ns, Some(250_000_000));
                let mut pixels = vec![10 + value; 16];
                pixels.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    pixels
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn all_dropped_or_incomplete_groups_do_not_publish_empty_video() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("shuffleframes-seven-frames.y4m");
        for (case, mapping) in [("all-drop", "-1"), ("partial", "0 1 2 3 4 5 6 7")] {
            let transform = LosslessTransform {
                shuffleframes: Some(mapping.into()),
                ..Default::default()
            };
            let output = std::env::temp_dir().join(format!(
                "fvid-shuffle-empty-{}-{case}.mkv",
                std::process::id()
            ));
            let error =
                crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
                    .unwrap_err();
            assert!(error.contains("Matroska has no selected frames"), "{error}");
            assert!(!output.exists());
        }
    }
    #[test]
    fn discarded_shuffle_tail_still_validates_payload_before_publication() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/shuffleframes-discarded-truncated.y4m");
        let output =
            std::env::temp_dir().join(format!("fvid-shuffle-truncated-{}.mkv", std::process::id()));
        let transform = LosslessTransform {
            shuffleframes: Some("2 1 0".into()),
            ..Default::default()
        };
        assert!(supports(&source, &transform, &CopyOptions::default()));
        let error = crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            .unwrap_err();
        assert!(error.contains("truncated Y4M frame payload"), "{error}");
        assert!(!output.exists());
    }
}

#[cfg(test)]
mod reverse_tests {
    use super::*;
    #[test]
    fn reversed_files_preserve_forward_packet_timing_and_source_counts() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/reverse-six-frames.y4m");
        for (case, interval, step, shuffle, values, positions, consumed) in [
            (
                "full",
                None,
                None,
                None,
                vec![5u8, 4, 3, 2, 1, 0],
                vec![0u64, 1, 2, 3, 4, 5],
                6,
            ),
            (
                "range",
                Some((250_000, 1_000_000)),
                None,
                None,
                vec![3, 2, 1],
                vec![1, 2, 3],
                4,
            ),
            (
                "step",
                None,
                Some("2"),
                None,
                vec![4, 2, 0],
                vec![0, 2, 4],
                6,
            ),
            (
                "shuffle",
                None,
                None,
                Some("2 1 0"),
                vec![3, 4, 5, 0, 1, 2],
                vec![0, 1, 2, 3, 4, 5],
                6,
            ),
        ] {
            let transform = LosslessTransform {
                reverse: Some(String::new()),
                interval,
                framestep: step.map(String::from),
                shuffleframes: shuffle.map(String::from),
                ..Default::default()
            };
            let options = CopyOptions::default();
            assert!(supports(&source, &transform, &options));
            let plan = crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let output = std::env::temp_dir().join(format!(
                "fvid-reverse-export-{}-{case}.mkv",
                std::process::id()
            ));
            let stats = crate::transcode_lossless(&source, &output, transform, &options).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.decoded_frames, consumed);
            assert_eq!(stats.video_frames, values.len() as u64);
            let mut reader = crate::owned_webm::WebmReader::open(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            reader.scan_all().unwrap();
            assert_eq!(reader.packets.len(), values.len());
            let origin = interval.map_or(0, |(from, _)| from as i64 * 1000);
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
            for (slot, (value, position)) in values.into_iter().zip(positions).enumerate() {
                assert_eq!(
                    reader.packets[slot].pts_ns,
                    position as i64 * 250_000_000 - origin
                );
                assert_eq!(reader.packets[slot].duration_ns, Some(250_000_000));
                let mut expected = vec![10 + value; 16];
                expected.extend([128; 8]);
                assert_eq!(
                    decoder
                        .decode(&reader.read_packet(slot).unwrap())
                        .unwrap()
                        .frame
                        .data,
                    expected
                );
            }
            drop(reader);
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn reverse_rejects_damaged_input_before_any_final_file_is_published() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/reverse-truncated-last-frame.y4m");
        let output =
            std::env::temp_dir().join(format!("fvid-reverse-damaged-{}.mkv", std::process::id()));
        let transform = LosslessTransform {
            reverse: Some(String::new()),
            ..Default::default()
        };
        assert!(supports(&source, &transform, &CopyOptions::default()));
        let error = crate::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            .unwrap_err();
        assert!(error.contains("truncated Y4M frame payload"), "{error}");
        assert!(!output.exists());
    }
}
