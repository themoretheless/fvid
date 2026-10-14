//! Alpha codec/frame/export/native presentation acceptance on authored video fixtures.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-alpha.json")).unwrap()).unwrap();
    let v3: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-v3-alpha.json")).unwrap()).unwrap();
    value["cases"]
        .as_array_mut()
        .unwrap()
        .extend(v3["cases"].as_array().unwrap().iter().cloned());
    value
}
fn decoder(
    case: &serde_json::Value,
    budget: usize,
) -> Result<fvid_media::owned_ffv1_decoder::Decoder, String> {
    let size = case["size"]
        .as_array()
        .map(|a| {
            [
                a[0].as_u64().unwrap() as usize,
                a[1].as_u64().unwrap() as usize,
            ]
        })
        .unwrap_or([4, 2]);
    let private = case["configuration"]
        .as_str()
        .map(|p| std::fs::read(root().join(p)).unwrap())
        .unwrap_or_default();
    fvid_media::owned_ffv1_decoder::Decoder::from_configuration(size[0], size[1], &private, budget)
}
#[test]
fn alpha_contexts_color_transform_and_precision_match_independent_arrays() {
    for case in manifest()["cases"].as_array().unwrap() {
        let mut decoder = decoder(case, 32 << 20).unwrap();
        for _ in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
                let decoded = decoder
                    .decode(&packet)
                    .unwrap_or_else(|e| panic!("{}: {e}", frame["packet"]));
                assert_eq!(
                    decoded.keyframe,
                    frame["keyframe"].as_bool().unwrap_or(i == 0)
                );
                assert_eq!(decoded.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(decoded.frame.subsampling.is_none(), case["layout"] == "rgb");
                assert_eq!(
                    decoded.frame.data,
                    std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap()
                );
                assert_eq!(
                    decoded.alpha.unwrap(),
                    std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap()
                );
            }
            decoder.reset();
        }
    }
}
#[test]
fn alpha_encoders_match_independent_keyframe_packets_without_premultiplication() {
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["coder"] == 1 && c["version"] == 1)
    {
        let depth = case["depth"].as_u64().unwrap() as u8;
        for frame in case["frames"].as_array().unwrap() {
            let data = std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap();
            let alpha = std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap();
            let expected =
                std::fs::read(root().join(frame["keypacket"].as_str().unwrap())).unwrap();
            let geometry = fvid_media::owned_frame::GeometryFrame {
                width: 4,
                height: 2,
                subsampling: if case["layout"] == "rgb" {
                    None
                } else if case["layout"] == "420" {
                    Some([2, 2])
                } else {
                    Some([1, 1])
                },
                data,
            };
            let encoded = if case["layout"] == "gray" {
                fvid_media::owned_ffv1_encoder::encode_gray_with_alpha(
                    4,
                    2,
                    &geometry.data[..8 * if depth == 8 { 1 } else { 2 }],
                    &alpha,
                    depth,
                )
                .unwrap()
            } else {
                fvid_media::owned_ffv1_encoder::encode_with_alpha(&geometry, depth, &alpha).unwrap()
            };
            assert_eq!(encoded, expected, "{}", frame["keypacket"]);
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 32 << 20).unwrap();
            let decoded = decoder.decode(&encoded).unwrap();
            assert_eq!(decoded.frame.data, geometry.data);
            assert_eq!(decoded.alpha.unwrap(), alpha);
        }
    }
}
#[test]
fn alpha_storage_errors_invalidate_history_and_keyframes_recover() {
    let m = manifest();
    let case = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["layout"] == "rgb" && c["depth"] == 8 && c["coder"] == 1 && c["version"] == 1)
        .unwrap();
    let good = std::fs::read(root().join(case["frames"][0]["packet"].as_str().unwrap())).unwrap();
    let next = std::fs::read(root().join(case["frames"][1]["packet"].as_str().unwrap())).unwrap();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 32 << 20).unwrap();
    decoder.decode(&good).unwrap();
    for invalid in m["invalid"].as_array().unwrap() {
        let bad = std::fs::read(root().join(invalid["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            decoder.decode(&bad).err().unwrap(),
            invalid["error"].as_str().unwrap()
        );
        assert_eq!(
            decoder.decode(&next).err().unwrap(),
            "FFV1 frame requires a preceding keyframe"
        );
        decoder.decode(&good).unwrap();
        decoder.decode(&next).unwrap();
    }
    let geometry = fvid_media::owned_frame::GeometryFrame {
        width: 4,
        height: 2,
        subsampling: None,
        data: vec![0; 48],
    };
    assert_eq!(
        fvid_media::owned_ffv1_encoder::encode_with_alpha(&geometry, 10, &[0; 15]).unwrap_err(),
        "invalid FFV1 alpha sample storage"
    );
    let mut alpha = [0; 16];
    alpha[1] = 4;
    assert_eq!(
        fvid_media::owned_ffv1_encoder::encode_with_alpha(&geometry, 10, &alpha).unwrap_err(),
        "invalid FFV1 alpha sample storage"
    );
}
#[test]
fn alpha_api_budget_includes_color_alpha_models_and_rgb_scratch() {
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["depth"] == 16 && c["coder"] == 1)
    {
        let packets: Vec<_> = case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| std::fs::read(root().join(f["packet"].as_str().unwrap())).unwrap())
            .collect();
        let run = |budget| {
            let mut decoder = decoder(case, budget)?;
            let mut result = None;
            for packet in &packets {
                result.take();
                result = Some(decoder.decode(packet)?);
            }
            Ok::<_, String>(result.unwrap())
        };
        let (mut low, mut high) = (0, 32 << 20);
        while low < high {
            let middle = low + (high - low) / 2;
            if run(middle).is_ok() {
                high = middle
            } else {
                low = middle + 1
            }
        }
        assert!(low > 0 && low < 32 << 20);
        assert!(run(low).unwrap().alpha.is_some());
        assert!(run(low - 1).err().unwrap().contains("memory budget"));
    }
}
fn native_alpha_expected(
    case: &serde_json::Value,
    frame: &serde_json::Value,
) -> fvid::playback_native::PackedPlanar {
    let size = case["size"]
        .as_array()
        .map(|a| {
            [
                a[0].as_u64().unwrap() as usize,
                a[1].as_u64().unwrap() as usize,
            ]
        })
        .unwrap_or([4, 2]);
    fvid::playback_native::PackedPlanar::with_alpha(
        fvid::native_geometry::GeometryFrame {
            width: size[0],
            height: size[1],
            subsampling: match case["layout"].as_str().unwrap() {
                "rgb" => None,
                "420" => Some([2, 2]),
                _ => Some([1, 1]),
            },
            data: std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap(),
        },
        case["depth"].as_u64().unwrap() as u8,
        Default::default(),
        Some(std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap()),
    )
    .unwrap()
}
#[test]
fn native_alpha_preserves_raw_samples_timing_rewind_seek_and_explicit_black_presentation() {
    for case in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let mut intervals = Vec::new();
        for pass in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let fvid::playback_native::RawFrame::Planar(p) = &raw else {
                    panic!("alpha lost raw precision")
                };
                let expected = native_alpha_expected(case, frame);
                assert_eq!(p.frame.data, expected.frame.data);
                assert_eq!(p.alpha, expected.alpha);
                assert_eq!(p.depth, expected.depth);
                if pass == 0 {
                    intervals.push(reader.frame_interval().unwrap());
                } else {
                    assert_eq!(reader.frame_interval().unwrap(), intervals[i]);
                }
                let mut rgba = Vec::new();
                expected.to_rgba(&mut rgba, 32 << 20).unwrap();
                assert_eq!(raw.into_rgba(32 << 20).unwrap(), rgba);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for frame in case["frames"].as_array().unwrap() {
            assert!(reader.read_frame().unwrap());
            let mut expected = Vec::new();
            native_alpha_expected(case, frame)
                .to_rgb_on_black(&mut expected, 32 << 20)
                .unwrap();
            assert_eq!(reader.rgb(), expected);
        }
        for i in [intervals.len() - 1, 0, 1] {
            let (start, _, scale) = intervals[i];
            let ns = u64::try_from(start * 1_000_000_000 / u128::from(scale)).unwrap();
            let raw = reader
                .seek_raw(std::time::Duration::from_nanos(ns))
                .unwrap()
                .unwrap();
            let fvid::playback_native::RawFrame::Planar(p) = raw else {
                panic!("seek lost alpha")
            };
            assert_eq!(
                p.alpha,
                native_alpha_expected(case, &case["frames"][i]).alpha
            );
            assert_eq!(reader.frame_interval().unwrap(), intervals[i]);
        }
    }
}

#[test]
fn source_depth_alpha_rotates_with_color_and_rgba_keeps_unassociated_samples() {
    use fvid::playback_native::{AvcColour, PackedPlanar};
    for case in manifest()["cases"].as_array().unwrap() {
        let mut decoder = decoder(case, 32 << 20).unwrap();
        for frame in case["frames"].as_array().unwrap() {
            let decoded = decoder
                .decode(&std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap())
                .unwrap();
            let alpha = decoded.alpha.unwrap();
            let depth = decoded.depth;
            let (w, h) = (decoded.frame.width, decoded.frame.height);
            let bytes = if depth == 8 { 1 } else { 2 };
            let color_only = PackedPlanar::new(
                fvid::native_geometry::GeometryFrame {
                    width: decoded.frame.width,
                    height: decoded.frame.height,
                    subsampling: decoded.frame.subsampling,
                    data: decoded.frame.data.clone(),
                },
                depth,
                AvcColour::default(),
            )
            .unwrap();
            let mut expected_rgb = Vec::new();
            color_only.to_rgb(&mut expected_rgb, w * h * 3).unwrap();
            let packed = PackedPlanar::with_alpha(
                decoded.frame,
                depth,
                AvcColour::default(),
                Some(alpha.clone()),
            )
            .unwrap();
            assert!(packed
                .to_rgb(&mut Vec::new(), usize::MAX)
                .unwrap_err()
                .to_string()
                .contains("cannot discard alpha"));
            assert!(packed
                .to_planar8(usize::MAX)
                .err()
                .unwrap()
                .to_string()
                .contains("cannot discard alpha"));
            let mut rgba = vec![91];
            assert!(packed.to_rgba(&mut rgba, w * h * 7 - 1).is_err());
            assert_eq!(rgba, [91]);
            packed.to_rgba(&mut rgba, w * h * 7).unwrap();
            let max = (1u32 << depth) - 1;
            let mut black = Vec::new();
            packed.to_rgb_on_black(&mut black, w * h * 3).unwrap();
            for (i, pixel) in rgba.chunks_exact(4).enumerate() {
                assert_eq!(&pixel[..3], &expected_rgb[3 * i..3 * i + 3]);
                let sample = if bytes == 1 {
                    u32::from(alpha[i])
                } else {
                    u32::from(u16::from_le_bytes([alpha[2 * i], alpha[2 * i + 1]]))
                };
                assert_eq!(pixel[3], ((sample * 255 + max / 2) / max) as u8);
                for c in 0..3 {
                    assert_eq!(
                        black[3 * i + c],
                        ((u32::from(expected_rgb[3 * i + c]) * sample + max / 2) / max) as u8
                    );
                }
            }
            for rotation in [0, 90, 180, 270] {
                let rotated = packed.rotated(rotation).unwrap();
                let (rw, rh) = if matches!(rotation, 90 | 270) {
                    (h, w)
                } else {
                    (w, h)
                };
                assert_eq!((rotated.frame.width, rotated.frame.height), (rw, rh));
                let mut actual = Vec::new();
                rotated.to_rgba(&mut actual, w * h * 7).unwrap();
                for y in 0..h {
                    for x in 0..w {
                        let (xx, yy) = match rotation {
                            0 => (x, y),
                            90 => (h - 1 - y, x),
                            180 => (w - 1 - x, h - 1 - y),
                            _ => (y, w - 1 - x),
                        };
                        let src = y * w + x;
                        let dst = yy * rw + xx;
                        assert_eq!(
                            &rotated.alpha.as_ref().unwrap()[dst * bytes..(dst + 1) * bytes],
                            &alpha[src * bytes..(src + 1) * bytes]
                        );
                        assert_eq!(
                            &actual[dst * 4..(dst + 1) * 4],
                            &rgba[src * 4..(src + 1) * 4]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn packed_alpha_revalidates_public_storage_before_conversion_or_rotation() {
    assert!(fvid::playback_native::RawFrame::Rgb(vec![1, 2, 3])
        .into_rgba(6)
        .is_err());
    assert_eq!(
        fvid::playback_native::RawFrame::Rgb(vec![1, 2, 3])
            .into_rgba(7)
            .unwrap(),
        [1, 2, 3, 255]
    );
    assert!(fvid::playback_native::RawFrame::Rgb(vec![1, 2])
        .into_rgba(100)
        .is_err());

    use fvid::playback_native::{AvcColour, PackedPlanar};
    let geometry = fvid::native_geometry::GeometryFrame {
        width: 2,
        height: 1,
        subsampling: None,
        data: vec![0; 12],
    };
    assert!(PackedPlanar::with_alpha(
        fvid::native_geometry::GeometryFrame {
            width: geometry.width,
            height: geometry.height,
            subsampling: geometry.subsampling,
            data: geometry.data.clone()
        },
        10,
        AvcColour::default(),
        Some(vec![0; 3])
    )
    .is_err());
    assert!(PackedPlanar::with_alpha(
        fvid::native_geometry::GeometryFrame {
            width: geometry.width,
            height: geometry.height,
            subsampling: geometry.subsampling,
            data: geometry.data.clone()
        },
        10,
        AvcColour::default(),
        Some(vec![0, 4, 0, 0])
    )
    .is_err());
    let mut packed =
        PackedPlanar::with_alpha(geometry, 10, AvcColour::default(), Some(vec![0; 4])).unwrap();
    packed.alpha.as_mut().unwrap().pop();
    assert!(packed.to_rgba(&mut Vec::new(), 100).is_err());
    assert!(packed.rotated(90).is_err());
}

fn flip_samples(data: &[u8], width: usize, height: usize, pixel: usize) -> Vec<u8> {
    let mut output = vec![0; data.len()];
    for y in 0..height {
        for x in 0..width {
            let from = (y * width + x) * pixel;
            let to = (y * width + width - 1 - x) * pixel;
            output[to..to + pixel].copy_from_slice(&data[from..from + pixel]);
        }
    }
    output
}
fn flipped_color(case: &serde_json::Value, frame: &serde_json::Value) -> Vec<u8> {
    let bytes = if case["depth"] == 8 { 1 } else { 2 };
    let (w, h) = case["size"]
        .as_array()
        .map(|a| {
            (
                a[0].as_u64().unwrap() as usize,
                a[1].as_u64().unwrap() as usize,
            )
        })
        .unwrap_or((4, 2));
    let data = std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap();
    if case["layout"] == "rgb" {
        return flip_samples(&data, w, h, 3 * bytes);
    }
    let (cw, ch) = if case["layout"] == "420" {
        (w.div_ceil(2), h.div_ceil(2))
    } else {
        (w, h)
    };
    let mut result = flip_samples(&data[..w * h * bytes], w, h, bytes);
    let chroma = cw * ch * bytes;
    for i in 0..2 {
        let start = w * h * bytes + i * chroma;
        result.extend(flip_samples(&data[start..start + chroma], cw, ch, bytes));
    }
    result
}
fn assert_alpha_export(
    case: &serde_json::Value,
    transform: fvid::media_info::LosslessTransform,
    order: &[usize],
    name: &str,
) {
    let target = std::env::temp_dir().join(format!(
        "fvid-alpha-export-{}-{name}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&target);
    fvid_media::transcode_lossless(
        &root().join(case["file"].as_str().unwrap()),
        &target,
        transform,
        &Default::default(),
    )
    .unwrap();
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&target).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let track = reader.tracks.iter().find(|t| t.kind == 1).unwrap();
    let (w, h) = (track.width as usize, track.height as usize);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        w,
        h,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    assert_eq!(reader.packets.len(), order.len());
    for (packet, &source) in order.iter().enumerate() {
        let decoded = decoder
            .decode(&reader.read_packet(packet).unwrap())
            .unwrap();
        let frame = &case["frames"][source];
        let alpha = std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap();
        let bytes = if decoded.depth == 8 { 1 } else { 2 };
        assert_eq!(
            decoded.frame.data,
            flipped_color(case, frame),
            "{name} frame {packet} color"
        );
        assert_eq!(
            decoded.alpha.unwrap(),
            flip_samples(&alpha, w, h, bytes),
            "{name} frame {packet} alpha"
        );
        assert_eq!(decoded.depth, case["depth"].as_u64().unwrap() as u8);
    }
    std::fs::remove_file(target).unwrap();
}
#[test]
fn alpha_lossless_export_flips_color_and_opacity_at_every_coder_depth_and_version() {
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let order = (0..case["frames"].as_array().unwrap().len()).collect::<Vec<_>>();
        assert_alpha_export(
            case,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &order,
            &format!("flip-{index}"),
        );
    }
}
#[test]
fn alpha_lossless_temporal_spools_keep_the_matching_opacity_frame() {
    for (index, case) in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, c)| c["version"] == 3 && c["coder"] == 1 && c["depth"] == 16)
    {
        for (name, reverse, shuffle, order) in [
            ("reverse", true, None, vec![3, 2, 1, 0]),
            ("shuffle", false, Some("1 0".into()), vec![1, 0, 3, 2]),
            ("both", true, Some("1 0".into()), vec![2, 3, 0, 1]),
        ] {
            assert_alpha_export(
                case,
                fvid::media_info::LosslessTransform {
                    horizontal_flip: true,
                    reverse: reverse.then(String::new),
                    shuffleframes: shuffle,
                    ..Default::default()
                },
                &order,
                &format!("{name}-{index}"),
            );
        }
    }
}

#[test]
fn alpha_geometry_composition_and_color_filters_keep_opacity_independent() {
    for (index, case) in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, c)| c["version"] == 3 && c["coder"] == 1 && c["depth"] == 16)
    {
        let target = std::env::temp_dir().join(format!(
            "fvid-alpha-geometry-{}-{index}.mkv",
            std::process::id()
        ));
        let transform = fvid::media_info::LosslessTransform {
            crop: Some(fvid::media_info::CropRect {
                x: 2,
                y: 0,
                width: 4,
                height: 4,
            }),
            horizontal_flip: true,
            vertical_flip: true,
            transpose: Some(fvid::media_info::TransposeMode::Clock),
            pad: Some(fvid::media_info::PadRect {
                width: 8,
                height: 8,
                x: 2,
                y: 2,
            }),
            scale: Some(fvid::media_info::ScaleSize {
                width: 4,
                height: 4,
            }),
            negate: Some(String::new()),
            ..Default::default()
        };
        let _ = std::fs::remove_file(&target);
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &target,
            transform,
            &Default::default(),
        )
        .unwrap();
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&target).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let t = &reader.tracks[0];
        assert_eq!((t.width, t.height), (4, 4));
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            4,
            4,
            &t.codec_private,
            32 << 20,
        )
        .unwrap();
        for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            let source = std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap();
            let mut expected = vec![255; 32];
            for y in 1..3 {
                for x in 1..3 {
                    let phase = usize::from(case["layout"] != "rgb");
                    let from = ((2 * x - 2 + phase) * 8 + 5 - (2 * y - 2 + phase)) * 2;
                    let to = (y * 4 + x) * 2;
                    expected[to..to + 2].copy_from_slice(&source[from..from + 2]);
                }
            }
            assert_eq!(
                decoded.alpha.unwrap(),
                expected,
                "geometry case {index} frame {i}"
            );
            // Explicit black padding is opaque. Negate changes its color, not opacity.
            assert!(decoded.frame.data[..2].iter().any(|&v| v != 0));
        }
        std::fs::remove_file(target).unwrap();
    }
}

#[test]
fn opaque_synthetic_alpha_stays_opaque_at_oblique_rotation_canvas_edges() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-alpha.json")).unwrap()).unwrap();
    for (index, case) in m["opaque_rotations"].as_array().unwrap().iter().enumerate() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = ((1u32 << depth) - 1) as u16;
        let mut expected = Vec::new();
        for _ in 0..8 {
            if bytes == 1 {
                expected.push(maximum as u8)
            } else {
                expected.extend_from_slice(&maximum.to_le_bytes())
            }
        }
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 32 << 20).unwrap();
        assert_eq!(
            decoder
                .decode(&std::fs::read(root().join(case["packet"].as_str().unwrap())).unwrap())
                .unwrap()
                .alpha
                .unwrap(),
            expected
        );
        // Counterfactual: treating alpha as ordinary RGB fills the new canvas
        // with zero opacity. This fixture exposes precisely that wrong fill.
        let mut data = Vec::new();
        for _ in 0..3 {
            data.extend_from_slice(&expected);
        }
        let wrong = fvid_media::owned_rotate::rotate(
            &fvid::native_geometry::GeometryFrame {
                width: 4,
                height: 2,
                subsampling: None,
                data,
            },
            37.0,
            depth,
            true,
        )
        .unwrap();
        assert!(wrong.data.chunks_exact(bytes).any(|s| if bytes == 1 {
            s[0] != 255
        } else {
            u16::from_le_bytes([s[0], s[1]]) != maximum
        }));
        let target = std::env::temp_dir().join(format!(
            "fvid-alpha-opaque-{}-{index}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&target);
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &target,
            fvid::media_info::LosslessTransform {
                rotate: Some(fvid::media_info::RotateAngle { degrees: 37.0 }),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&target).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let t = &reader.tracks[0];
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            t.width as usize,
            t.height as usize,
            &t.codec_private,
            32 << 20,
        )
        .unwrap();
        let alpha = decoder
            .decode(&reader.read_packet(0).unwrap())
            .unwrap()
            .alpha
            .unwrap();
        assert!(
            alpha.chunks_exact(bytes).all(|s| if bytes == 1 {
                s[0] == 255
            } else {
                u16::from_le_bytes([s[0], s[1]]) == maximum
            }),
            "opaque rotated {index}"
        );
        std::fs::remove_file(target).unwrap();
    }
}

#[test]
fn malformed_alpha_export_fails_without_publishing_a_partial_file() {
    let target =
        std::env::temp_dir().join(format!("fvid-alpha-malformed-{}.mkv", std::process::id()));
    let _ = std::fs::remove_file(&target);
    let error = fvid_media::transcode_lossless(
        &root().join("ffv1-alpha-invalid-rgb-range.mkv"),
        &target,
        fvid::media_info::LosslessTransform {
            horizontal_flip: true,
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("FFV1 alpha reconstruction exceeds sample depth"),
        "{error}"
    );
    assert!(!target.exists());
}

#[test]
fn playback_converter_keeps_rgba_and_grades_color_without_changing_alpha() {
    use fvid::color::hdr::{ColourDescription, HdrMetadata};
    use fvid::color::{Grade, Settings};
    use fvid::playback_thread::{Event, Pixels, Playback};
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        for graded in [false, true] {
            if graded && !(case["layout"] == "rgb" && case["coder"] == 1) {
                continue;
            }
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
                32 << 20,
            )
            .unwrap();
            let first = reader.read_frame_raw().unwrap().unwrap();
            let grade = || {
                Grade::new(
                    ColourDescription {
                        primaries: 1,
                        transfer: 1,
                        matrix: 0,
                        full_range: true,
                    },
                    &HdrMetadata::default(),
                    Settings::default(),
                    None,
                )
            };
            let playback = Playback::start_from_frame(reader, first, graded.then(grade));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut count = 0;
            loop {
                assert!(
                    std::time::Instant::now() < deadline,
                    "alpha playback stalled {index}"
                );
                match playback.poll() {
                    Some(Event::Frame(frame)) => {
                        let Pixels::Rgba(actual) = frame.pixels else {
                            panic!("alpha was flattened before rendering")
                        };
                        let p = native_alpha_expected(case, &case["frames"][count]);
                        let mut expected = Vec::new();
                        p.to_rgba(&mut expected, 32 << 20).unwrap();
                        if graded {
                            let mut rgb = expected
                                .chunks_exact(4)
                                .flat_map(|p| p[..3].iter().copied())
                                .collect::<Vec<_>>();
                            grade().apply(&mut rgb);
                            for (src, dst) in rgb.chunks_exact(3).zip(expected.chunks_exact_mut(4))
                            {
                                dst[..3].copy_from_slice(src);
                            }
                        }
                        assert_eq!(
                            actual, expected,
                            "case {index} graded {graded} frame {count}"
                        );
                        assert_eq!(frame.dimensions, [p.frame.width, p.frame.height]);
                        count += 1;
                    }
                    Some(Event::Ended(_)) => break,
                    Some(Event::Error(e)) => panic!("{e}"),
                    None => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            assert_eq!(count, case["frames"].as_array().unwrap().len());
        }
    }
}

#[test]
fn color_only_geometry_refuses_alpha_instead_of_returning_an_opaque_frame() {
    for file in [
        "ffv1-alpha-rgb-16-coder1-v1.mkv",
        "ffv1-alpha-420-8-coder1-v1.mkv",
    ] {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(file)).unwrap()),
            32 << 20,
        )
        .unwrap();
        reader.read_frame_raw().unwrap().unwrap();
        let raw = reader.read_frame_raw().unwrap().unwrap();
        let fvid::playback_native::RawFrame::Planar(p) = &raw else {
            panic!("raw alpha")
        };
        assert!(p.alpha.is_some());
        let error = fvid::native_geometry::VideoGeometry::default()
            .apply(&raw, p.frame.width, p.frame.height)
            .err()
            .unwrap();
        assert!(error
            .to_string()
            .contains("color-only geometry cannot discard alpha"));
    }
}

#[test]
fn alpha_fixture_matrix_covers_every_advertised_depth_and_entropy_mode() {
    let m = manifest();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 228);
    for depth in 8..=16 {
        for coder in 0..=2 {
            for layout in ["gray", "420", "444", "rgb"] {
                for version in if depth == 8 {
                    vec![0, 1, 3]
                } else {
                    vec![1, 3]
                } {
                    assert_eq!(
                        cases
                            .iter()
                            .filter(|c| c["depth"] == depth
                                && c["coder"] == coder
                                && c["layout"] == layout
                                && c["version"] == version)
                            .count(),
                        1,
                        "depth {depth}, coder {coder}, layout {layout}, version {version}"
                    );
                }
            }
        }
    }
}
