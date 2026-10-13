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
fn rgb_lossless_export_remains_an_explicit_refusal_until_ffv1_rgb_is_enabled() {
    for extension in ["mp4", "webm"] {
        let source = root().join(format!("av1-rgb-depth8.{extension}"));
        let target = std::env::temp_dir().join(format!(
            "fvid-av1-rgb-refusal-{}-{extension}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&target);
        let error = fvid_media::transcode_lossless(
            &source,
            &target,
            Default::default(),
            &Default::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            format!(
                "owned {} lossless export does not yet support this codec, format or transform",
                if extension == "mp4" {
                    "MP4"
                } else {
                    "Matroska"
                }
            )
        );
        assert!(!target.exists());
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
