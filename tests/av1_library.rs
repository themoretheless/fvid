//! Owned AV1 library dispatch, presentation, filters and lossless export.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn read_frames(path: &Path) -> Vec<Vec<u8>> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        frames.push(match frame {
            fvid::playback_native::RawFrame::Planar8(frame) => {
                [frame.y.as_slice(), frame.cb.as_slice(), frame.cr.as_slice()].concat()
            }
            fvid::playback_native::RawFrame::Planar(frame) => frame.frame.data.clone(),
            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                let mut bytes = Vec::new();
                picture.write_planar(&mut bytes).unwrap();
                bytes
            }
            _ => panic!("expected planar AV1/FFV1 frames"),
        });
    }
    frames
}
fn export_check(file: &str, expected: &[Vec<u8>], all_filters: bool, depth: u8) {
    for operation in if all_filters {
        vec!["plain", "negate", "step", "reverse"]
    } else {
        vec!["plain", "negate"]
    } {
        let request = match operation {
            "negate" => fvid_media::LosslessTransform {
                negate: Some("".into()),
                ..Default::default()
            },
            "step" => fvid_media::LosslessTransform {
                framestep: Some("2".into()),
                ..Default::default()
            },
            "reverse" => fvid_media::LosslessTransform {
                reverse: Some("".into()),
                ..Default::default()
            },
            _ => Default::default(),
        };
        let wanted: Vec<Vec<u8>> = match operation {
            "negate" => expected
                .iter()
                .map(|f| {
                    if depth == 8 {
                        f.iter().map(|v| 255 - v).collect()
                    } else {
                        f.chunks_exact(2)
                            .flat_map(|v| {
                                (((1u16 << depth) - 1) - u16::from_le_bytes([v[0], v[1]]))
                                    .to_le_bytes()
                            })
                            .collect()
                    }
                })
                .collect(),
            "step" => expected.iter().step_by(2).cloned().collect(),
            "reverse" => expected.iter().rev().cloned().collect(),
            _ => expected.to_vec(),
        };
        let source = root().join(file);
        let output = std::env::temp_dir().join(format!(
            "fvid-av1-library-{}-{file}-{operation}.mkv",
            std::process::id()
        ));
        let stats = fvid_media::transcode_lossless(&source, &output, request, &Default::default())
            .unwrap_or_else(|error| panic!("{file}/{operation}: {error}"));
        assert_eq!(
            stats.video_frames,
            wanted.len() as u64,
            "{file}/{operation}"
        );
        assert!(read_frames(&output) == wanted, "{file}/{operation}: pixels");
        if operation == "plain" && file.ends_with(".mp4") {
            let mut native = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&source).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let mut timeline = Vec::new();
            while native.read_frame_raw().unwrap().is_some() {
                let (start, end, scale) = native.frame_interval().unwrap();
                let start = (start * 1_000_000_000 / u128::from(scale)) as i64;
                let end = (end * 1_000_000_000 / u128::from(scale)) as i64;
                timeline.push((start, Some((end - start) as u64)));
            }
            let mut exported = fvid_media::owned_webm::WebmReader::open(
                Cursor::new(std::fs::read(&output).unwrap()),
                Default::default(),
            )
            .unwrap();
            exported.scan_all().unwrap();
            assert_eq!(
                exported
                    .packets
                    .iter()
                    .map(|p| (p.pts_ns, p.duration_ns))
                    .collect::<Vec<_>>(),
                timeline,
                "{file}: presentation"
            );
        }
        if operation == "plain" && file.contains("metadata") {
            let reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            assert_eq!(reader.hdr().light.max_cll, 1234.0);
            assert_eq!(reader.hdr().light.max_fall, 567.0);
        }
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn mp4_av1_edit_lists_stats_filters_and_export_match_independent_pixels() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("av1-library.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join(manifest["reference"].as_str().unwrap())).unwrap();
    let frames: Vec<Vec<u8>> = golden
        .chunks_exact(64 * 48 * 3 / 2)
        .map(<[u8]>::to_vec)
        .collect();
    assert_eq!(frames.len(), 8);
    assert!(
        frames.windows(2).any(|pair| pair[0] != pair[1]),
        "edit oracle must distinguish ordering"
    );
    for case in manifest["cases"].as_array().unwrap() {
        if !case["reference"].is_null() {
            continue;
        }
        let file = case["file"].as_str().unwrap();
        let expected: Vec<Vec<u8>> = case["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| frames[i.as_u64().unwrap() as usize].clone())
            .collect();
        let stats = fvid_media::decode_video(&root().join(file)).unwrap();
        assert_eq!(stats.video_frames, expected.len() as u64);
        assert_eq!((stats.width, stats.height), (64, 48));
        assert_eq!(stats.decode_errors, 0);
        assert!(
            read_frames(&root().join(file)) == expected,
            "{file}: player pixels"
        );
        export_check(file, &expected, true, 8);
    }
}
#[test]
fn implicit_final_size_mp4_and_webm_export_exact_pixels() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("av1-implicit-size.json")).unwrap())
            .unwrap();
    let expected =
        vec![std::fs::read(root().join("av1-separate-tiles-w128-h128-groups2.yuv")).unwrap()];
    for section in ["cases", "webm_cases"] {
        for case in manifest[section].as_array().unwrap() {
            if case["error"].is_string() {
                continue;
            }
            let file = case["file"].as_str().unwrap();
            assert_eq!(
                fvid_media::decode_video(&root().join(file))
                    .unwrap()
                    .video_frames,
                1
            );
            export_check(file, &expected, false, 8);
        }
    }
}
#[test]
fn spatial_svc_library_export_selects_highest_display_layer() {
    let golden = std::fs::read(root().join("av1-spatial-operating-points-op0.yuv")).unwrap();
    let mut at = 0;
    let mut expected = Vec::new();
    for index in 0..8 {
        let length = if index % 2 == 0 {
            32 * 24 * 3 / 2
        } else {
            64 * 48 * 3 / 2
        };
        if index % 2 != 0 {
            expected.push(golden[at..at + length].to_vec());
        }
        at += length;
    }
    for extension in ["mp4", "webm"] {
        export_check(
            &format!("av1-spatial-container-complete.{extension}"),
            &expected,
            false,
            8,
        );
    }
}

#[test]
fn mp4_av1_depth_and_odd_geometry_survive_filters_and_export() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("av1-library.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let Some(reference) = case["reference"].as_str() else {
            continue;
        };
        let file = case["file"].as_str().unwrap();
        let width = case["width"].as_u64().unwrap() as usize;
        let height = case["height"].as_u64().unwrap() as usize;
        let depth = case["depth"].as_u64().unwrap() as u8;
        let data = std::fs::read(root().join(reference)).unwrap();
        let factors = case["subsampling"]
            .as_array()
            .map(|sub| {
                [
                    1usize << sub[0].as_u64().unwrap(),
                    1usize << sub[1].as_u64().unwrap(),
                ]
            })
            .unwrap_or([2, 2]);
        let frame_bytes = (width * height
            + 2 * width.div_ceil(factors[0]) * height.div_ceil(factors[1]))
            * if depth == 8 { 1 } else { 2 };
        let expected: Vec<Vec<u8>> = data.chunks_exact(frame_bytes).map(<[u8]>::to_vec).collect();
        assert_eq!(expected.len(), case["indices"].as_array().unwrap().len());
        let stats = fvid_media::decode_video(&root().join(file)).unwrap();
        assert_eq!(stats.video_frames, expected.len() as u64);
        assert_eq!((stats.width, stats.height), (width as u32, height as u32));
        let actual = read_frames(&root().join(file));
        let first_difference = actual
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
            .position(|(a, b)| a != b);
        assert!(
            actual == expected,
            "{file}: player oracle, first differing byte {first_difference:?}"
        );
        export_check(file, &expected, true, depth);
    }
}

#[test]
fn invalid_av1_configuration_stays_owned_and_export_is_atomic() {
    for (file, error) in [
        (
            "av1-configuration-mismatch-profile.mp4",
            "AV1 configuration fields disagree with sequence header",
        ),
        (
            "av1-configuration-inband-mismatch-level.mp4",
            "AV1 configuration fields disagree with sequence header",
        ),
        (
            "av1-configuration-empty.mp4",
            "MP4 AV1 configuration is absent",
        ),
        (
            "av1-implicit-size-invalid-extension-truncated.mp4",
            "truncated AV1 OBU extension",
        ),
    ] {
        let source = root().join(file);
        assert_eq!(
            fvid_media::decode_video(&source).unwrap_err(),
            error,
            "{file}"
        );
        let output = std::env::temp_dir().join(format!(
            "fvid-av1-library-invalid-{}-{file}.mkv",
            std::process::id()
        ));
        std::fs::write(&output, b"preserve existing output").unwrap();
        assert_eq!(
            fvid_media::transcode_lossless(
                &source,
                &output,
                Default::default(),
                &Default::default()
            )
            .unwrap_err(),
            error,
            "{file}"
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"preserve existing output");
        std::fs::remove_file(output).unwrap();
    }
}
