//! Shared compressed packets feed owned filtering and atomic lossless export.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn raw_frames(path: &Path) -> Vec<Vec<u8>> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        frames.push(match frame {
            fvid::playback_native::RawFrame::Planar(frame) => frame.frame.data.clone(),
            fvid::playback_native::RawFrame::Planar8(frame) => {
                [frame.y.as_slice(), frame.cb.as_slice(), frame.cr.as_slice()].concat()
            }
            _ => panic!("expected planar compressed-video output"),
        });
    }
    frames
}
#[test]
fn compressed_pixel_filters_and_temporal_selection_share_owned_pipeline() {
    for name in [
        "vp9/adaptive.webm",
        "vp9/adaptive10.webm",
        "playback-errors/shared-vp9-stride.webm",
        "av1/ramp.webm",
        "playback-errors/shared-av1-private-sequence.webm",
    ] {
        for request in [
            fvid::media::DecodeTransform {
                negate: Some("".into()),
                ..Default::default()
            },
            fvid::media::DecodeTransform {
                deband: Some("1thr=.5:2thr=.5:3thr=.5".into()),
                ..Default::default()
            },
            fvid::media::DecodeTransform {
                framestep: Some("2".into()),
                ..Default::default()
            },
        ] {
            let root =
                fvid::media::decode_video_transformed(&fixture(name), request.clone()).unwrap();
            let library = fvid_media::decode_video_transformed(&fixture(name), request)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(library.backend, "owned WebM compressed video pipeline");
            assert_eq!(library.video_frames, root.video_frames, "{name}");
            assert_eq!(
                (library.width, library.height),
                (root.width, root.height),
                "{name}"
            );
            assert_eq!(library.pixel_format, root.pixel_format, "{name}");
        }
    }
}
#[test]
fn compressed_sources_export_every_pixel_through_owned_ffv1() {
    for (case, name) in [
        "vp9/adaptive.webm",
        "vp9/adaptive10.webm",
        "playback-errors/shared-vp9-stride.webm",
        "av1/ramp.webm",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = std::env::temp_dir().join(format!(
            "fvid-compressed-pipeline-{}-{case}.mkv",
            std::process::id()
        ));
        let stats = fvid_media::transcode_lossless(
            &source,
            &output,
            Default::default(),
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(stats.encoder, "ffv1");
        assert_eq!(raw_frames(&output), raw_frames(&source), "{name}");
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn hidden_reference_preroll_does_not_shift_the_visible_interval_origin() {
    let source = fixture("playback-errors/shared-vp9-hidden-leading.webm");
    let request = fvid::media::DecodeTransform {
        interval: Some((0, 20_000)),
        ..Default::default()
    };
    let root = fvid::media::decode_video_transformed(&source, request.clone()).unwrap();
    let library = fvid_media::decode_video_transformed(&source, request).unwrap();
    assert_eq!(root.video_frames, 1);
    assert_eq!(library.video_frames, root.video_frames);
    assert_eq!(
        raw_frames(&source),
        raw_frames(&fixture("vp9/motion.webm"))[1..]
    );
}

#[test]
fn configuration_only_av1_sequence_survives_rewind_and_seek() {
    let source = fixture("playback-errors/shared-av1-private-sequence.webm");
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(&source).unwrap()),
        usize::MAX,
    )
    .unwrap();
    for _ in 0..2 {
        assert!(reader.read_frame_raw().unwrap().is_some());
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind().unwrap();
    }
    assert!(
        reader
            .seek_raw(std::time::Duration::ZERO)
            .unwrap()
            .is_some()
    );
}

#[test]
fn compressed_filter_export_changes_pixels_and_reorders_actual_frames() {
    for (case, name, depth) in [
        (0, "vp9/adaptive.webm", 8),
        (1, "playback-errors/shared-vp9-stride.webm", 10),
        (2, "av1/random-access.webm", 8),
    ] {
        let source = fixture(name);
        let originals = raw_frames(&source);
        for (operation, request, expected) in [
            (
                "negate",
                fvid_media::LosslessTransform {
                    negate: Some("".into()),
                    ..Default::default()
                },
                originals
                    .iter()
                    .map(|bytes| {
                        if depth == 8 {
                            bytes.iter().map(|v| 255 - v).collect()
                        } else {
                            bytes
                                .chunks_exact(2)
                                .flat_map(|v| {
                                    (((1u16 << depth) - 1) - u16::from_le_bytes([v[0], v[1]]))
                                        .to_le_bytes()
                                })
                                .collect()
                        }
                    })
                    .collect::<Vec<Vec<u8>>>(),
            ),
            (
                "step",
                fvid_media::LosslessTransform {
                    framestep: Some("2".into()),
                    ..Default::default()
                },
                originals.iter().step_by(2).cloned().collect(),
            ),
            (
                "reverse",
                fvid_media::LosslessTransform {
                    reverse: Some("".into()),
                    ..Default::default()
                },
                originals.iter().rev().cloned().collect(),
            ),
            (
                "shuffle",
                fvid_media::LosslessTransform {
                    shuffleframes: Some("1 0".into()),
                    ..Default::default()
                },
                originals
                    .chunks_exact(2)
                    .flat_map(|frames| frames.iter().rev().cloned())
                    .collect(),
            ),
        ] {
            let output = std::env::temp_dir().join(format!(
                "fvid-compressed-filter-{}-{case}-{operation}.mkv",
                std::process::id()
            ));
            let stats =
                fvid_media::transcode_lossless(&source, &output, request, &Default::default())
                    .unwrap_or_else(|e| panic!("{name}/{operation}: {e}"));
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.video_frames, expected.len() as u64);
            assert_eq!(raw_frames(&output), expected, "{name}/{operation}");
            std::fs::remove_file(output).unwrap();
        }
    }
}

#[test]
fn configuration_hdr_metadata_survives_owned_lossless_export() {
    let source = fixture("playback-errors/shared-av1-hdr-carry.webm");
    let input = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(&source).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let expected = input.hdr();
    assert_eq!(expected.light.max_cll, 1234.0);
    assert_eq!(expected.light.max_fall, 567.0);
    assert!(expected.mastering.is_some());
    let output = std::env::temp_dir().join(format!("fvid-hdr-carry-{}.mkv", std::process::id()));
    fvid_media::transcode_lossless(&source, &output, Default::default(), &Default::default())
        .unwrap();
    let decoded = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(&output).unwrap()),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(decoded.hdr(), expected);
    assert_eq!(raw_frames(&output), raw_frames(&source));
    std::fs::remove_file(output).unwrap();
}
