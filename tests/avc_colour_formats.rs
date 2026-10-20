//! Acceptance and refusal tests for AVC Matroska non-4:2:0 formats,
//! AVC Identity RGB (GBR), and VUI matrix coefficients (FCC, SMPTE 240M, BT.2020).
use fvid::playback_native::{NativeReader, RawFrame};
use std::{io::Cursor, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-colour-formats")
}

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}

fn samples(raw: &RawFrame) -> Vec<u8> {
    match raw {
        RawFrame::Planar(p) => p.frame.data.clone(),
        RawFrame::Planar8(p) => [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()].concat(),
        RawFrame::Avc { picture, .. } => {
            let mut v = Vec::new();
            picture.write_planar(&mut v).unwrap();
            v
        }
        _ => panic!("software sample payload"),
    }
}

#[test]
fn matroska_avc_non420_preserves_chroma_depth_pixels_display_geometry_and_seek() {
    let m = manifest();
    let cases = m["mkv_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 144);
    for case in cases {
        let file = case["file"].as_str().unwrap();
        let mut control = NativeReader::software(
            Cursor::new(bytes(case["source"].as_str().unwrap())),
            32 << 20,
        )
        .unwrap();
        let mut expected_rgb = Vec::new();
        for frame in case["frames"].as_array().unwrap() {
            let raw = control.read_frame_raw().unwrap().unwrap();
            assert_eq!(
                samples(&raw),
                bytes(frame["packed"].as_str().unwrap()),
                "{file}: MP4 control must match saved synthetic samples"
            );
            expected_rgb.push(raw.into_rgb(32 << 20).unwrap());
        }
        assert!(control.read_frame_raw().unwrap().is_none());

        let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
        let rotation = reader.rotation();
        let insets = reader.insets();
        assert_eq!(rotation, case["rotation"].as_u64().unwrap() as u16);
        let depth = case["depth"].as_u64().unwrap() as u8;
        let sub = [
            case["subsampling"][0].as_u64().unwrap() as usize,
            case["subsampling"][1].as_u64().unwrap() as usize,
        ];
        for _ in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let rgb = match &raw {
                    RawFrame::Planar(p) => {
                        let mut rgb = Vec::new();
                        p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                        rgb
                    }
                    _ => panic!("{file}: non-4:2:0 AVC must use RawFrame::Planar"),
                };
                assert_eq!(
                    rgb, expected_rgb[i],
                    "{file}: Matroska RGB differs from qualified MP4 chroma sampling"
                );
                let RawFrame::Planar(p) = raw else {
                    panic!("{file}: AVC non-4:2:0 must retain actual packed sample geometry")
                };
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.subsampling, Some(sub));
                assert_eq!(
                    p.frame.data,
                    bytes(frame["packed"].as_str().unwrap()),
                    "{file}: raw source samples"
                );
                let display = fvid::native_geometry::VideoGeometry::default()
                    .apply_packed_display_media(&p, rotation, insets)
                    .unwrap();
                assert_eq!(
                    display.frame.data,
                    bytes(frame["display"].as_str().unwrap()),
                    "{file}: independent crop/rotation samples"
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [case["frames"].as_array().unwrap().len() - 1, 0] {
            let raw = reader
                .seek_raw(std::time::Duration::from_nanos(
                    index as u64 * 1_000_000_000 / case["rate"].as_u64().unwrap(),
                ))
                .unwrap()
                .unwrap();
            let RawFrame::Planar(p) = raw else {
                panic!("{file}: seek payload")
            };
            assert_eq!(
                p.frame.data,
                bytes(case["frames"][index]["packed"].as_str().unwrap()),
                "{file}: seek samples"
            );
        }
        let stats = fvid_media::decode_video(&root().join(file))
            .unwrap_or_else(|e| panic!("{file} decode_video: {e}"));
        assert_eq!(stats.video_frames, 2);
    }
}

#[test]
fn avc_identity_rgb_preserves_codes_range_geometry_rewind_seek_and_export() {
    let m = manifest();
    let cases = m["rgb_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 128);
    for (case_idx, case) in cases.iter().enumerate() {
        for kind in ["mp4", "file"] {
            let file = case[kind].as_str().unwrap();
            let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
            let rotation = reader.rotation();
            let insets = reader.insets();
            let frames = case["frames"].as_array().unwrap();
            for _ in 0..2 {
                for frame in frames {
                    let raw = reader.read_frame_raw().unwrap().unwrap();
                    let RawFrame::Planar(p) = raw else {
                        panic!("{file}: RGB packed payload required")
                    };
                    assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
                    assert_eq!(p.frame.subsampling, None);
                    assert_eq!(
                        p.frame.data,
                        bytes(frame["rgb"].as_str().unwrap()),
                        "{file}: source RGB codes"
                    );
                    let mut rgb = Vec::new();
                    p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                    assert_eq!(
                        rgb,
                        bytes(frame["rgb8"].as_str().unwrap()),
                        "{file}: range normalization"
                    );
                    if kind == "file" {
                        let display = fvid::native_geometry::VideoGeometry::default()
                            .apply_packed_display_media(&p, rotation, insets)
                            .unwrap();
                        assert_eq!(
                            display.frame.data,
                            bytes(frame["display"].as_str().unwrap()),
                            "{file}: display codes"
                        );
                        display.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                        assert_eq!(
                            rgb,
                            bytes(frame["display8"].as_str().unwrap()),
                            "{file}: display range"
                        );
                    }
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                reader.rewind().unwrap();
            }
            for index in [frames.len() - 1, 0] {
                let raw = reader
                    .seek_raw(std::time::Duration::from_nanos(
                        index as u64 * 1_000_000_000 / case["rate"].as_u64().unwrap(),
                    ))
                    .unwrap()
                    .unwrap();
                let RawFrame::Planar(p) = raw else {
                    panic!("{file}: RGB seek payload required")
                };
                assert_eq!(
                    p.frame.data,
                    bytes(frames[index]["rgb"].as_str().unwrap()),
                    "{file}: seek codes"
                );
                let mut rgb = Vec::new();
                p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                assert_eq!(
                    rgb,
                    bytes(frames[index]["rgb8"].as_str().unwrap()),
                    "{file}: seek range"
                );
            }
            let stats = fvid_media::decode_video(&root().join(file))
                .unwrap_or_else(|e| panic!("{file} decode_video: {e}"));
            assert_eq!(stats.video_frames, 2);
        }

        if case["rotation"] == 0 && case["crop"][0] == 0 {
            let source = root().join(case["mp4"].as_str().unwrap());
            let output = std::env::temp_dir().join(format!(
                "fvid-avc-rgb-{}-{case_idx}.mkv",
                std::process::id()
            ));
            fvid_media::transcode_lossless(
                &source,
                &output,
                fvid::media_info::LosslessTransform {
                    horizontal_flip: true,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let data = std::fs::read(&output).unwrap();
            let mut container =
                fvid_media::owned_webm::WebmReader::open(Cursor::new(data.clone()), Default::default())
                    .unwrap();
            container.scan_all().unwrap();
            let track = &container.tracks[0];
            assert_eq!(track.colour.matrix, 0);
            assert_eq!(track.full_range, Some(case["full_range"].as_bool().unwrap()));
            let depth = case["depth"].as_u64().unwrap() as u8;
            let width = case["size"][0].as_u64().unwrap() as usize;
            let unit = if depth == 8 { 1 } else { 2 };
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
                track.width as usize,
                track.height as usize,
                &track.codec_private,
                32 << 20,
            )
            .unwrap();
            let mut player = NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let decoded = decoder.decode(&container.read_packet(i).unwrap()).unwrap();
                assert_eq!(decoded.frame.subsampling, None);
                assert_eq!(decoded.depth, depth);
                let flip = |data: Vec<u8>, b: usize| -> Vec<u8> {
                    data.chunks_exact(width * b * 3)
                        .flat_map(|row| row.chunks_exact(b * 3).rev().flatten().copied())
                        .collect()
                };
                assert_eq!(decoded.frame.data, flip(bytes(frame["rgb"].as_str().unwrap()), unit));
                let RawFrame::Planar(p) = player.read_frame_raw().unwrap().unwrap() else {
                    panic!("RGB playback payload")
                };
                let mut rgb = Vec::new();
                p.to_rgb_on_black(&mut rgb, 32 << 20).unwrap();
                assert_eq!(rgb, flip(bytes(frame["rgb8"].as_str().unwrap()), 1));
            }
            std::fs::remove_file(output).unwrap();
        }
    }
}

#[test]
fn avc_vui_matrix_coefficients_and_invalid_identity_refusals() {
    let m = manifest();
    let matrix_cases = m["matrix_cases"].as_array().unwrap();
    assert_eq!(matrix_cases.len(), 56);
    for case in matrix_cases {
        let kr = case["kr"].as_f64().unwrap();
        let kb = case["kb"].as_f64().unwrap();
        let full = case["full_range"].as_bool().unwrap();
        for kind in ["mp4", "mkv"] {
            let file = case[kind].as_str().unwrap();
            let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
            let mut count = 0;
            while let Some(raw) = reader.read_frame_raw().unwrap_or_else(|e| panic!("{file}: {e}")) {
                let colour = match &raw {
                    RawFrame::Planar(p) => p.colour,
                    RawFrame::Planar8(p) => p.colour,
                    RawFrame::Avc { colour, .. } => *colour,
                    _ => panic!("{file}: unexpected frame variant"),
                };
                assert!((colour.kr - kr).abs() < 1e-9, "{file} kr");
                assert!((colour.kb - kb).abs() < 1e-9, "{file} kb");
                assert_eq!(colour.full, full, "{file} full_range");
                let rgb = raw.into_rgb(32 << 20).unwrap();
                assert_eq!(rgb.len(), 32 * 16 * 3);
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }

    for refusal in m["refusals"].as_array().unwrap() {
        let expected = refusal["error"].as_str().unwrap();
        for kind in ["mp4", "mkv"] {
            let file = refusal[kind].as_str().unwrap();
            let mut reader = NativeReader::software(Cursor::new(bytes(file)), 32 << 20).unwrap();
            let err = match reader.read_frame_raw() {
                Ok(_) => panic!("{file}: expected refusal '{expected}'"),
                Err(e) => e.to_string(),
            };
            assert!(err.contains(expected), "{file}: got {err}");
            let decode_err = fvid_media::decode_video(&root().join(file)).unwrap_err();
            assert!(decode_err.contains(expected), "{file} decode_video: got {decode_err}");
        }
    }
}
