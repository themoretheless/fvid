//! Matroska FFV1 decode/filter/encode pipeline, with atomic file publication.
use crate::{owned_matroska as mkv, owned_video_decode as decode};
use fvid_control::CopyOptions;
use fvid_media_info::{DecodeStats, DecodeTransform};
use std::{fs::File, io::BufReader, path::Path};
type Result<T> = std::result::Result<T, String>;
fn input(source: &Path) -> Result<crate::owned_webm::WebmReader<BufReader<File>>> {
    let mut input = crate::owned_webm::WebmReader::open(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        Default::default(),
    )
    .map_err(|e| e.to_string())?;
    input.scan_all().map_err(|e| e.to_string())?;
    Ok(input)
}
pub(crate) fn supports(source: &Path, transform: &DecodeTransform) -> bool {
    let Ok(input) = input(source) else {
        return false;
    };
    if input.tracks.len() != 1 || !input.metadata_complete {
        return false;
    }
    let track = &input.tracks[0];
    if track.kind != 1
        || track.codec != "V_FFV1"
        || !track.codec_private.is_empty()
        || track.crop != [0; 4]
        || track.rotation != 0
        || !track.name.is_empty()
        || !matches!(track.language.as_str(), "" | "und")
        || (transform.interval.is_some() && !input.chapters.is_empty())
        || input.packets.iter().any(|p| {
            p.invisible
                || p.pts_ns < 0
                || p.discard_padding_ns != 0
                || p.duration_ns.unwrap_or(track.default_duration_ns) == 0
        })
    {
        return false;
    }
    // Qualify all keyframes before selecting this backend, preserving legacy tools.
    // Corruption remains owned and is reported by execution, without publication.
    !matches!(decode::try_ffv1(source, transform), Ok(None))
}
fn aspect(
    track: &crate::owned_webm::Track,
    transform: &DecodeTransform,
    width: u32,
    height: u32,
) -> Result<(u32, u32)> {
    let (n, d) = track.pixel_aspect();
    let (mut n, mut d) = (u128::from(n), u128::from(d));
    let (mut w, mut h) = transform
        .crop
        .map_or((u128::from(track.width), u128::from(track.height)), |c| {
            (c.width as u128, c.height as u128)
        });
    if transform.transpose.is_some() {
        std::mem::swap(&mut n, &mut d);
        std::mem::swap(&mut w, &mut h);
    }
    if let Some(p) = transform.pad {
        (w, h) = (u128::from(p.width), u128::from(p.height));
    }
    if transform.scale.is_some() {
        n = n
            .checked_mul(w)
            .and_then(|v| v.checked_mul(u128::from(height)))
            .ok_or("pixel aspect overflow")?;
        d = d
            .checked_mul(h)
            .and_then(|v| v.checked_mul(u128::from(width)))
            .ok_or("pixel aspect overflow")?;
    }
    let (mut a, mut b) = (n, d);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    if a == 0 {
        return Err("invalid pixel aspect".into());
    }
    Ok((
        u32::try_from(n / a).map_err(|_| "pixel aspect overflow")?,
        u32::try_from(d / a).map_err(|_| "pixel aspect overflow")?,
    ))
}
pub(crate) fn export(
    source: &Path,
    destination: &Path,
    transform: &DecodeTransform,
    options: &CopyOptions,
) -> Result<(DecodeStats, fvid_control::ProgressEvent, u64)> {
    let input = input(source)?;
    let track = input.tracks.first().ok_or("input has no video stream")?;
    let origin = input.packets.iter().map(|p| p.pts_ns).min().unwrap_or(0);
    let mut metadata = mkv::FileMetadata {
        tags: input.tags.clone(),
        chapters: input.chapters.clone(),
    };
    for key in &options.metadata_delete {
        if !metadata.tags.set(key, "") {
            return Err("unsupported owned container metadata key".into());
        }
    }
    for (key, value) in &options.metadata_set {
        if !metadata.tags.set(key, value) {
            return Err("unsupported owned container metadata key".into());
        }
    }
    mkv::export_atomic(
        destination,
        options.cancel.as_ref(),
        options.progress.as_ref(),
        |file| {
            let mut output = Some(file);
            let mut writer = None;
            let mut visitor = |view: decode::FrameView<'_>| -> Result<()> {
                let packet = if view.monochrome {
                    let samples = (view.width as usize)
                        .checked_mul(view.height as usize)
                        .and_then(|v| v.checked_mul(if view.depth == 8 { 1 } else { 2 }))
                        .ok_or("FFV1 gray size overflow")?;
                    crate::owned_ffv1_encoder::encode_gray(
                        view.width as usize,
                        view.height as usize,
                        &view.pixels[..samples],
                        view.depth,
                    )?
                } else {
                    let mut data = crate::owned_frame::buffer(view.pixels.len())?;
                    data.copy_from_slice(view.pixels);
                    crate::owned_ffv1_encoder::encode(
                        &crate::owned_frame::GeometryFrame {
                            width: view.width as usize,
                            height: view.height as usize,
                            subsampling: Some(view.subsampling),
                            data,
                        },
                        view.depth,
                    )?
                };
                if packet.len() > options.max_packet_bytes {
                    return Err("encoded FFV1 packet exceeds byte limit".into());
                }
                if writer.is_none() {
                    let video = mkv::VideoMetadata {
                        pixel_aspect: aspect(track, transform, view.width, view.height)?,
                        colour: Some(track.colour),
                        hdr: track.hdr,
                        ..Default::default()
                    };
                    writer = Some(
                        mkv::PacketWriter::new_ffv1_with_text_tags(
                            output.take().unwrap(),
                            view.width,
                            view.height,
                            Some(&video),
                            0,
                            0,
                            &metadata,
                            &input.metadata,
                        )
                        .map_err(|e| e.to_string())?,
                    );
                }
                let pts = if let Some((from, _)) = transform.interval {
                    i128::from(view.pts_ns) - i128::from(origin) - i128::from(from) * 1000
                } else {
                    i128::from(view.pts_ns)
                };
                let pts = u64::try_from(pts)
                    .map_err(|_| "Matroska output timestamp underflow or overflow")?;
                let writer = writer.as_mut().unwrap();
                writer
                    .write_packet(
                        0,
                        pts,
                        view.duration_ns.ok_or("FFV1 packet has no duration")?,
                        true,
                        &packet,
                    )
                    .map_err(|e| e.to_string())?;
                if let Some(hook) = &options.progress {
                    hook.emit(writer.event());
                }
                Ok(())
            };
            let result = decode::decode_ffv1(source, transform, Some(&mut visitor), Some(options))
                .map_err(mkv::Error)?;
            drop(visitor);
            let (stats, consumed) = result
                .ok_or_else(|| mkv::Error("unsupported owned FFV1 export capability".into()))?;
            let event = writer
                .ok_or_else(|| mkv::Error("Matroska has no selected frames".into()))?
                .finish()?;
            Ok((stats, event, consumed))
        },
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvid_media_info::LosslessTransform;
    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
    }
    #[test]
    fn public_export_preserves_gray_samples_source_pts_and_interval_preroll() {
        for depth in [8u8, 10, 16] {
            for (case, source_name, interval, step, indices, pts) in [
                (
                    "full",
                    format!("ffv1-gray-{depth}.mkv"),
                    None,
                    None,
                    vec![0, 1],
                    vec![0, 40_000_000],
                ),
                (
                    "step",
                    format!("ffv1-gray-{depth}.mkv"),
                    None,
                    Some("2".into()),
                    vec![0],
                    vec![0],
                ),
                (
                    "range",
                    format!("ffv1-gray-{depth}.mkv"),
                    Some((40_000, 80_000)),
                    Some("2".into()),
                    vec![1],
                    vec![0],
                ),
            ] {
                let source = root().join(source_name);
                let output = std::env::temp_dir().join(format!(
                    "fvid-ffv1-export-{}-{depth}-{case}.mkv",
                    std::process::id()
                ));
                let _ = std::fs::remove_file(&output);
                let transform = LosslessTransform {
                    interval,
                    framestep: step,
                    horizontal_flip: true,
                    ..Default::default()
                };
                let options = CopyOptions {
                    metadata_set: vec![("title".into(), "owned FFV1".into())],
                    ..Default::default()
                };
                let plan =
                    crate::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
                assert!(plan.notes[0].starts_with("backend: owned"));
                let stats =
                    crate::transcode_lossless(&source, &output, transform, &options).unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(stats.decoded_frames, 2);
                assert_eq!(stats.video_frames, indices.len() as u64);
                let mut mux = input(&output).unwrap();
                assert_eq!(mux.tags.title, "owned FFV1");
                assert_eq!(mux.packets.len(), indices.len());
                let mut decoder =
                    crate::owned_ffv1_decoder::Decoder::new(4, 3, usize::MAX).unwrap();
                for (i, index) in indices.into_iter().enumerate() {
                    assert_eq!(mux.packets[i].pts_ns, pts[i]);
                    assert_eq!(mux.packets[i].duration_ns, Some(40_000_000));
                    let packet = mux.read_packet(i).unwrap();
                    let frame = decoder.decode(&packet).unwrap();
                    assert_eq!(decoder.monochrome(), Some(true));
                    let raw = std::fs::read(root().join(format!("ffv1-gray-{depth}-{index}.gray")))
                        .unwrap();
                    let bytes = if depth == 8 { 1 } else { 2 };
                    let expected: Vec<u8> = raw
                        .chunks_exact(4 * bytes)
                        .flat_map(|row| row.chunks_exact(bytes).rev().flatten().copied())
                        .collect();
                    assert_eq!(&frame.frame.data[..raw.len()], expected);
                }
                std::fs::remove_file(output).unwrap();
            }
        }
        let source = root().join("ffv1-positive-start.mkv");
        let output = std::env::temp_dir().join(format!(
            "fvid-ffv1-export-positive-{}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        crate::transcode_lossless(&source, &output, Default::default(), &Default::default())
            .unwrap();
        assert_eq!(
            input(&output)
                .unwrap()
                .packets
                .iter()
                .map(|p| p.pts_ns)
                .collect::<Vec<_>>(),
            [200_000_000, 240_000_000]
        );
        std::fs::remove_file(output).unwrap();
    }
    #[test]
    fn failed_or_cancelled_export_never_publishes_partial_output() {
        let directory =
            std::env::temp_dir().join(format!("fvid-ffv1-export-failures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        for (name, fixture, options, message) in [
            (
                "damage",
                "ffv1-invalid-range-header.mkv",
                CopyOptions::default(),
                "invalid FFV1 range header",
            ),
            (
                "limit",
                "ffv1-gray-8.mkv",
                CopyOptions {
                    max_packets: Some(1),
                    ..Default::default()
                },
                "FFV1 input packet count exceeds limit",
            ),
        ] {
            let destination = directory.join(format!("{name}.mkv"));
            let error = crate::transcode_lossless(
                &root().join(fixture),
                &destination,
                LosslessTransform {
                    reverse: Some(String::new()),
                    ..Default::default()
                },
                &options,
            )
            .unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(!destination.exists());
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        }
        let cancel = fvid_control::CancelFlag::new();
        let signal = cancel.clone();
        let options = CopyOptions {
            cancel: Some(cancel),
            progress: Some(fvid_control::ProgressHook::new(move |e| {
                assert!(!e.done);
                signal.cancel();
            })),
            ..Default::default()
        };
        let destination = directory.join("cancel.mkv");
        let error = crate::transcode_lossless(
            &root().join("ffv1-gray-8.mkv"),
            &destination,
            LosslessTransform {
                reverse: Some(String::new()),
                ..Default::default()
            },
            &options,
        )
        .unwrap_err();
        assert!(error.contains("cancelled"));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn variable_frame_durations_aspect_and_fractional_selection_survive_export() {
        let source = root().join("ffv1-vfr.mkv");
        for (case, interval, scale, expected_pts, expected_durations) in [
            (
                "full",
                None,
                None,
                vec![0, 73_000_000],
                vec![73_000_000, 27_000_000],
            ),
            (
                "range",
                Some((50_000, 100_000)),
                Some(fvid_media_info::ScaleSize {
                    width: 8,
                    height: 6,
                }),
                vec![23_000_000],
                vec![27_000_000],
            ),
        ] {
            let output = std::env::temp_dir()
                .join(format!("fvid-ffv1-vfr-{}-{case}.mkv", std::process::id()));
            let _ = std::fs::remove_file(&output);
            let transform = LosslessTransform {
                interval,
                scale,
                ..Default::default()
            };
            crate::plan_transcode_lossless(&source, &transform, &Default::default(), None).unwrap();
            crate::transcode_lossless(&source, &output, transform, &Default::default()).unwrap();
            let mux = input(&output).unwrap();
            assert_eq!(
                mux.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
                expected_pts
            );
            assert_eq!(
                mux.packets
                    .iter()
                    .map(|p| p.duration_ns.unwrap())
                    .collect::<Vec<_>>(),
                expected_durations
            );
            assert_eq!(mux.tracks[0].pixel_aspect(), (2, 1));
            assert_eq!(
                (mux.tracks[0].width, mux.tracks[0].height),
                if scale.is_some() { (8, 6) } else { (4, 3) }
            );
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn compressed_temporal_export_preserves_source_payload_order_and_forward_positions() {
        let source = root().join("ffv1-six-frames.mkv");
        for (case, reverse, shuffle, step, interval, indices, times) in [
            (
                "reverse",
                true,
                None,
                None,
                None,
                vec![5, 4, 3, 2, 1, 0],
                vec![0, 40, 80, 120, 160, 200],
            ),
            (
                "range",
                true,
                None,
                None,
                Some((40_000, 200_000)),
                vec![4, 3, 2, 1],
                vec![0, 40, 80, 120],
            ),
            (
                "compose",
                true,
                Some("2|1|0".into()),
                Some("2".into()),
                None,
                vec![0, 2, 4],
                vec![0, 80, 160],
            ),
            (
                "shuffle",
                false,
                Some("2|0|1".into()),
                None,
                None,
                vec![2, 0, 1, 5, 3, 4],
                vec![0, 40, 80, 120, 160, 200],
            ),
            (
                "tail",
                true,
                Some("2|1|0".into()),
                None,
                Some((40_000, 240_000)),
                vec![1, 2, 3],
                vec![0, 40, 80],
            ),
            (
                "drops",
                true,
                Some("2|-1|2".into()),
                None,
                None,
                vec![5, 5, 2, 2],
                vec![0, 80, 120, 200],
            ),
        ] {
            let output = std::env::temp_dir().join(format!(
                "fvid-ffv1-temporal-{}-{case}.mkv",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            let transform = LosslessTransform {
                reverse: reverse.then(String::new),
                shuffleframes: shuffle,
                framestep: step,
                interval,
                ..Default::default()
            };
            let decoded = crate::decode_video_transformed(
                &source,
                DecodeTransform {
                    reverse: transform.reverse.clone(),
                    shuffleframes: transform.shuffleframes.clone(),
                    framestep: transform.framestep.clone(),
                    interval: transform.interval,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(decoded.backend, "owned Matroska FFV1 decode");
            assert_eq!(decoded.video_frames, indices.len() as u64);
            let plan =
                crate::plan_transcode_lossless(&source, &transform, &Default::default(), None)
                    .unwrap();
            assert!(plan.notes[0].starts_with("backend: owned"));
            let stats = crate::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
            assert_eq!(stats.video_frames, indices.len() as u64);
            assert_eq!(stats.decoded_frames, if case == "range" { 5 } else { 6 });
            let mut mux = input(&output).unwrap();
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 3, usize::MAX).unwrap();
            for (i, index) in indices.into_iter().enumerate() {
                assert_eq!(mux.packets[i].pts_ns, times[i] * 1_000_000);
                assert_eq!(mux.packets[i].duration_ns, Some(40_000_000));
                let frame = decoder.decode(&mux.read_packet(i).unwrap()).unwrap();
                let expected: Vec<u8> = (0..12u32)
                    .map(|pixel| match pixel {
                        0 => 0,
                        1 => 255,
                        2 => 128,
                        _ => ((pixel * 977 + index * 1237) & 255) as u8,
                    })
                    .collect();
                assert_eq!(&frame.frame.data[..12], expected, "{case} frame {i}");
            }
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn variable_duration_reverse_keeps_position_timing_while_shuffle_maps_source_duration() {
        let source = root().join("ffv1-vfr.mkv");
        for (case, reverse, shuffle, durations) in [
            (
                "reverse",
                Some(String::new()),
                None,
                [73_000_000, 27_000_000],
            ),
            (
                "shuffle",
                None,
                Some("1|0".into()),
                [27_000_000, 73_000_000],
            ),
        ] {
            let output = std::env::temp_dir().join(format!(
                "fvid-ffv1-temporal-vfr-{}-{case}.mkv",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            crate::transcode_lossless(
                &source,
                &output,
                LosslessTransform {
                    reverse,
                    shuffleframes: shuffle,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let mut mux = input(&output).unwrap();
            assert_eq!(
                mux.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
                [0, 73_000_000]
            );
            assert_eq!(
                mux.packets
                    .iter()
                    .map(|p| p.duration_ns.unwrap())
                    .collect::<Vec<_>>(),
                durations
            );
            let mut decoder = crate::owned_ffv1_decoder::Decoder::new(4, 3, usize::MAX).unwrap();
            for (i, index) in [1, 0].into_iter().enumerate() {
                let frame = decoder.decode(&mux.read_packet(i).unwrap()).unwrap();
                let raw = std::fs::read(root().join(format!("ffv1-gray-8-{index}.gray"))).unwrap();
                assert_eq!(&frame.frame.data[..12], raw);
            }
            std::fs::remove_file(output).unwrap();
        }
    }
    #[test]
    fn owned_export_preserves_unlisted_file_tags_and_applies_canonical_overrides() {
        let source = root().join("ffv1-custom-tags.mkv");
        let output =
            std::env::temp_dir().join(format!("fvid-ffv1-custom-tags-{}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let options = CopyOptions {
            metadata_set: vec![("title".into(), "New title".into())],
            ..Default::default()
        };
        assert!(supports(&source, &Default::default()));
        crate::transcode_lossless(&source, &output, Default::default(), &options).unwrap();
        let info = crate::owned_probe::probe(&output).unwrap();
        assert_eq!(info.metadata["FVID_TEST_NOTE"], "own container metadata");
        assert_eq!(info.metadata["ENCODER"], "synthetic source");
        assert_eq!(info.metadata["title"], "New title");
        std::fs::remove_file(output).unwrap();
    }
    #[test]
    fn scoped_tags_are_an_explicit_export_refusal_until_preserved_by_owned_writer() {
        let source = root().join("ffv1-track-tags.mkv");
        let reader = input(&source).unwrap();
        assert!(!reader.metadata_complete);
        assert!(!supports(&source, &Default::default()));
        // Refusal preserves the existing backend; this is not acceptance for scoped-tag export.
    }
}
