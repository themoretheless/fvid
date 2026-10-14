//! Owned identity-matrix AV1 samples and independently saved channel samples.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_sequence::Sequence};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("av1-rgb.json")).unwrap()).unwrap()
}
#[test]
fn authored_rgb_headers_decode_to_saved_gbr_samples() {
    for case in manifest()["cases"].as_array().unwrap() {
        let raw =
            std::fs::read(root().join(case["files"]["obu"]["file"].as_str().unwrap())).unwrap();
        let seq = Obus::new(&raw)
            .map(Result::unwrap)
            .find(|o| o.kind == 1)
            .unwrap();
        let sequence = Sequence::parse(seq.payload).unwrap();
        let color = &sequence.color;
        assert_eq!((color.primaries, color.transfer, color.matrix), (1, 13, 0));
        assert_eq!(color.subsampling, [false, false]);
        assert!(color.full_range && !color.monochrome);
        assert_eq!(color.depth, case["depth"].as_u64().unwrap() as u8);
        let expected = std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap();
        let mut decoder = Decoder::new(32 << 20);
        let mut actual = Vec::new();
        let mut count = 0;
        for frame in decoder.decode_packet(&raw).unwrap() {
            if !frame.show {
                continue;
            }
            count += 1;
            let p = &frame.picture;
            for y in 0..p.size[1] as usize {
                for x in 0..p.size[0] as usize {
                    for i in [2, 0, 1] {
                        let plane = &p.planes[i];
                        let value = plane.samples[y * plane.width + x];
                        if p.depth == 8 {
                            actual.push(value as u8)
                        } else {
                            actual.extend_from_slice(&value.to_le_bytes())
                        }
                    }
                }
            }
        }
        decoder.finish().unwrap();
        assert_eq!(count, 2);
        assert!(actual == expected);
    }
}

fn presentation(samples: &[u8], depth: u8) -> Vec<u8> {
    if depth == 8 {
        return samples.to_vec();
    }
    let max = (1u32 << depth) - 1;
    samples
        .chunks_exact(2)
        .map(|v| ((u32::from(u16::from_le_bytes([v[0], v[1]])) * 255 + max / 2) / max) as u8)
        .collect()
}
#[test]
fn rgb_mp4_and_webm_preserve_source_depth_pixels_rewind_and_seek() {
    for case in manifest()["cases"].as_array().unwrap() {
        for extension in ["mp4", "webm"] {
            let file = case["files"][extension]["file"].as_str().unwrap();
            let depth = case["depth"].as_u64().unwrap() as u8;
            let expected = std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap();
            let frame_bytes = 64 * 48 * 3 * if depth == 8 { 1 } else { 2 };
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(root().join(file)).unwrap()),
                32 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                for (index, wanted) in expected.chunks_exact(frame_bytes).enumerate() {
                    let frame = reader.read_frame_raw().unwrap().unwrap();
                    let fvid::playback_native::RawFrame::Planar(p) = &frame else {
                        panic!("RGB raw samples must retain depth")
                    };
                    assert_eq!(p.depth, depth);
                    assert_eq!(p.frame.subsampling, None);
                    assert!(p.frame.data == wanted, "{file}/{index}: exact samples");
                    let converted = frame.into_rgb(32 << 20).unwrap();
                    assert!(
                        converted == presentation(wanted, depth),
                        "{file}/{index}: RGB display"
                    );
                    let (start, end, scale) = reader.frame_interval().unwrap();
                    assert_eq!(
                        (
                            start * 1_000_000_000 / u128::from(scale),
                            end * 1_000_000_000 / u128::from(scale)
                        ),
                        (index as u128 * 20_000_000, (index as u128 + 1) * 20_000_000)
                    );
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                reader.rewind().unwrap();
            }
            for wanted in expected.chunks_exact(frame_bytes) {
                assert!(reader.read_frame().unwrap());
                assert!(reader.rgb() == presentation(wanted, depth));
            }
            let raw = reader.seek_raw(std::time::Duration::ZERO).unwrap().unwrap();
            let fvid::playback_native::RawFrame::Planar(p) = &raw else {
                panic!("RGB seek lost precision")
            };
            assert!(p.frame.data == expected[..frame_bytes]);
            let identity = fvid::native_geometry::VideoGeometry::default()
                .apply(&raw, 64, 48)
                .unwrap();
            assert_eq!(identity.subsampling, None);
            assert_eq!(identity.data, p.frame.data);
            let pixel_bytes = 3 * if depth == 8 { 1 } else { 2 };
            let mut rotated = vec![0u8; frame_bytes];
            for y in 0..48 {
                for x in 0..64 {
                    let source = (y * 64 + x) * pixel_bytes;
                    let target = (x * 48 + (47 - y)) * pixel_bytes;
                    rotated[target..target + pixel_bytes]
                        .copy_from_slice(&p.frame.data[source..source + pixel_bytes]);
                }
            }
            let flipped = fvid::native_geometry::VideoGeometry {
                horizontal_flip: true,
                ..Default::default()
            }
            .apply(&raw, 64, 48)
            .unwrap();
            for y in 0..48 {
                for x in 0..64 {
                    let source = (y * 64 + x) * pixel_bytes;
                    let target = (y * 64 + (63 - x)) * pixel_bytes;
                    assert_eq!(
                        &flipped.data[target..target + pixel_bytes],
                        &p.frame.data[source..source + pixel_bytes]
                    );
                }
            }
            let padded = fvid::native_geometry::VideoGeometry {
                pad: Some([66, 50, 1, 1]),
                ..Default::default()
            }
            .apply(&raw, 64, 48)
            .unwrap();
            assert_eq!(padded.subsampling, None);
            for y in 0..50 {
                for x in 0..66 {
                    let at = (y * 66 + x) * pixel_bytes;
                    let actual = &padded.data[at..at + pixel_bytes];
                    if x == 0 || y == 0 || x == 65 || y == 49 {
                        assert!(actual.iter().all(|&v| v == 0));
                    } else {
                        let source = ((y - 1) * 64 + x - 1) * pixel_bytes;
                        assert_eq!(actual, &p.frame.data[source..source + pixel_bytes]);
                    }
                }
            }
            let turned = p.rotated(90).unwrap();
            assert_eq!((turned.frame.width, turned.frame.height), (48, 64));
            assert_eq!(turned.frame.subsampling, None);
            assert!(turned.frame.data == rotated);
        }
    }
}
#[test]
fn rgb_lossless_export_preserves_all_coded_samples() {
    for case in manifest()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        for extension in ["mp4", "webm"] {
            let source = root().join(case["files"][extension]["file"].as_str().unwrap());
            let target = std::env::temp_dir().join(format!(
                "fvid-av1-rgb-export-{}-{depth}-{extension}.mkv",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&target);
            fvid_media::transcode_lossless(
                &source,
                &target,
                Default::default(),
                &Default::default(),
            )
            .unwrap();
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&target).unwrap()),
                32 << 20,
            )
            .unwrap();
            let mut actual = Vec::new();
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                let fvid::playback_native::RawFrame::Planar(p) = raw else {
                    panic!("RGB export narrowed samples")
                };
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.subsampling, None);
                actual.extend_from_slice(&p.frame.data);
            }
            assert_eq!(
                actual,
                std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap()
            );
            std::fs::remove_file(target).unwrap();
        }
    }
}
#[test]
fn rgb_export_geometry_and_negate_preserve_independent_channel_oracles() {
    use fvid::media_info::{
        CropRect, LosslessTransform, PadRect, RotateAngle, ScaleSize, TransposeMode,
    };
    for case in manifest()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let (w, h) = (
            case["width"].as_u64().unwrap() as usize,
            case["height"].as_u64().unwrap() as usize,
        );
        let pixel = if depth == 8 { 3 } else { 6 };
        let reference = std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap();
        for extension in ["mp4", "webm"] {
            for mode in 0..7 {
                let transform = match mode {
                    0 => LosslessTransform {
                        horizontal_flip: true,
                        vertical_flip: true,
                        negate: Some(String::new()),
                        ..Default::default()
                    },
                    1 => LosslessTransform {
                        rotate: Some(RotateAngle { degrees: 90.0 }),
                        ..Default::default()
                    },
                    2 => LosslessTransform {
                        crop: Some(CropRect {
                            x: 1,
                            y: 1,
                            width: 4,
                            height: 2,
                        }),
                        horizontal_flip: true,
                        transpose: Some(TransposeMode::Clock),
                        pad: Some(PadRect {
                            width: 6,
                            height: 6,
                            x: 2,
                            y: 2,
                        }),
                        scale: Some(ScaleSize {
                            width: 12,
                            height: 12,
                        }),
                        ..Default::default()
                    },
                    3 => LosslessTransform {
                        colorchannelmixer: Some("rr=0:rb=1:bb=0:br=1".into()),
                        ..Default::default()
                    },
                    4 => LosslessTransform {
                        curves: Some("all=0/1 1/1:enable=eq(n,0)".into()),
                        ..Default::default()
                    },
                    5 => LosslessTransform {
                        colorlevels: Some("romax=0:gomax=0:bomax=0".into()),
                        ..Default::default()
                    },
                    _ => LosslessTransform {
                        curves: Some("all=0/0 1/0".into()),
                        colorbalance: Some("rs=1".into()),
                        ..Default::default()
                    },
                };
                let (ow, oh) = match mode {
                    0 => (w, h),
                    1 => (h, w),
                    2 => (12, 12),
                    _ => (w, h),
                };
                let target = std::env::temp_dir().join(format!(
                    "fvid-rgb-geometry-{}-{depth}-{extension}-{mode}.mkv",
                    std::process::id()
                ));
                let _ = std::fs::remove_file(&target);
                fvid_media::transcode_lossless(
                    &root().join(case["files"][extension]["file"].as_str().unwrap()),
                    &target,
                    transform,
                    &Default::default(),
                )
                .unwrap();
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(&target).unwrap()),
                    32 << 20,
                )
                .unwrap();
                let mut count = 0;
                for source in reference.chunks_exact(w * h * pixel) {
                    let mut expected = vec![0; ow * oh * pixel];
                    for y in 0..oh {
                        for x in 0..ow {
                            let position = match mode {
                                0 => Some((w - 1 - x, h - 1 - y)),
                                1 => Some((y, h - 1 - x)),
                                2 => {
                                    let (px, py) = (x / 2, y / 2);
                                    if (2..4).contains(&px) && (2..6).contains(&py) {
                                        Some((4 - (py - 2), 2 - (px - 2)))
                                    } else {
                                        None
                                    }
                                }
                                _ => Some((x, y)),
                            };
                            if let Some((sx, sy)) = position {
                                let from = (sy * w + sx) * pixel;
                                let to = (y * ow + x) * pixel;
                                expected[to..to + pixel]
                                    .copy_from_slice(&source[from..from + pixel]);
                            }
                        }
                    }
                    if mode == 0 {
                        if depth == 8 {
                            for v in &mut expected {
                                *v = 255 - *v;
                            }
                        } else {
                            for v in expected.chunks_exact_mut(2) {
                                let value =
                                    ((1u16 << depth) - 1) - u16::from_le_bytes([v[0], v[1]]);
                                v.copy_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                    if mode == 4 && count == 0 {
                        if depth == 8 {
                            expected.fill(255);
                        } else {
                            for sample in expected.chunks_exact_mut(2) {
                                sample.copy_from_slice(&((1u16 << depth) - 1).to_le_bytes());
                            }
                        }
                    }
                    if mode == 3 {
                        let bytes = pixel / 3;
                        for p in expected.chunks_exact_mut(pixel) {
                            for b in 0..bytes {
                                p.swap(b, 2 * bytes + b);
                            }
                        }
                    }
                    if mode == 5 || mode == 6 {
                        expected.fill(0);
                        if mode == 6 {
                            let red =
                                (0.7f32 * ((1u32 << depth) - 1) as f32).round_ties_even() as u16;
                            for p in expected.chunks_exact_mut(pixel) {
                                if depth == 8 {
                                    p[0] = red as u8;
                                } else {
                                    p[..2].copy_from_slice(&red.to_le_bytes());
                                }
                            }
                        }
                    }
                    let fvid::playback_native::RawFrame::Planar(p) =
                        reader.read_frame_raw().unwrap().unwrap()
                    else {
                        panic!("RGB geometry narrowed samples")
                    };
                    assert_eq!(
                        (p.frame.width, p.frame.height, p.depth, p.frame.subsampling),
                        (ow, oh, depth, None)
                    );
                    assert_eq!(
                        p.frame.data, expected,
                        "depth {depth}, {extension}, mode {mode}, frame {count}"
                    );
                    count += 1;
                }
                assert_eq!(count, case["frames"].as_u64().unwrap());
                assert!(reader.read_frame_raw().unwrap().is_none());
                std::fs::remove_file(target).unwrap();
            }
        }
    }
}
#[test]
fn high_precision_rgb_rotation_preserves_channels_without_narrowing() {
    for depth in [8, 10, 12] {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(format!("av1-rgb-depth{depth}.mp4"))).unwrap()),
            32 << 20,
        )
        .unwrap();
        let raw = reader.read_frame_raw().unwrap().unwrap();
        let fvid::playback_native::RawFrame::Planar(p) = &raw else {
            panic!("RGB lost precision")
        };
        for rotation in [0, 90, 180, 270] {
            let actual = fvid::native_geometry::VideoGeometry {
                rotate: Some(fvid::media_info::RotateAngle {
                    degrees: f64::from(rotation),
                }),
                ..Default::default()
            }
            .apply(&raw, 64, 48)
            .unwrap();
            let expected = p.rotated(rotation).unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.frame.width, expected.frame.height)
            );
            assert_eq!(actual.subsampling, None);
            assert_eq!(
                actual.data, expected.frame.data,
                "depth {depth}, angle {rotation}"
            );
        }
    }
}

#[test]
fn rgb_container_display_rotation_preserves_raw_precision_and_display_codes() {
    for case in manifest()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let file = case["files"]["rotated"]["file"].as_str().unwrap();
        let expected = std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap();
        let frame_bytes = 64 * 48 * 3 * if depth == 8 { 1 } else { 2 };
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(file)).unwrap()),
            32 << 20,
        )
        .unwrap();
        assert_eq!(reader.rotation(), 90);
        for wanted in expected.chunks_exact(frame_bytes) {
            let rgb = presentation(wanted, depth);
            let mut rotated = vec![0; rgb.len()];
            for y in 0..48 {
                for x in 0..64 {
                    let source = (y * 64 + x) * 3;
                    let dest = (x * 48 + 47 - y) * 3;
                    rotated[dest..dest + 3].copy_from_slice(&rgb[source..source + 3]);
                }
            }
            assert!(reader.read_frame().unwrap());
            assert_eq!(reader.dimensions(), [48, 64]);
            assert!(reader.rgb() == rotated, "{file}: display");
        }
        let raw = reader.seek_raw(std::time::Duration::ZERO).unwrap().unwrap();
        let fvid::playback_native::RawFrame::Planar(p) = &raw else {
            panic!("RGB rotation narrowed raw samples")
        };
        assert_eq!(p.depth, depth);
        assert_eq!((p.frame.width, p.frame.height), (64, 48));
        assert_eq!(p.frame.data, &expected[..frame_bytes]);
        let upright = fvid::native_geometry::VideoGeometry::default()
            .apply_display(&raw, 48, 64, 90)
            .unwrap();
        let turned = p.rotated(90).unwrap();
        assert_eq!(upright.data, turned.frame.data);
    }
}
