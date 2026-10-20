use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-matroska-field-timing")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&fs::read(root().join("manifest.json")).unwrap())
        .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn native_matroska_paff_uses_first_field_timestamp_and_combined_duration() {
    use fvid::playback_native::NativeReader;
    for case in cases() {
        let mut reader = NativeReader::software(
            std::io::Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let count = case["frames"].as_array().unwrap().len();
        assert_eq!(
            reader.duration().unwrap(),
            std::time::Duration::from_millis(count as u64 * 20)
        );
        for _ in 0..2 {
            for index in 0..count {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify_native(&raw, &case, index);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (
                        index as u128 * 20_000_000,
                        index as u128 * 20_000_000 + 20_000_000,
                        1_000_000_000
                    ),
                    "{}",
                    case["file"]
                );
                let rgb = raw.into_rgb(32 << 20).unwrap();
                let expected = rgb_oracle(&case, index);
                assert_eq!(rgb.len(), expected.len());
                assert!(
                    rgb.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1),
                    "{} RGB",
                    case["file"]
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            assert_eq!(
                reader.duration().unwrap(),
                std::time::Duration::from_millis(count as u64 * 20)
            );
            reader.rewind().unwrap();
        }
        for index in [count - 1, 0, count - 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index as u64 * 20))
                .unwrap()
                .unwrap();
            verify_native(&raw, &case, index);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (
                    index as u128 * 20_000_000,
                    index as u128 * 20_000_000 + 20_000_000,
                    1_000_000_000
                )
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn owned_matroska_paff_acceptance() {
    for case in cases().iter() {
        let stats = fvid_media::decode_video(&root().join(case["file"].as_str().unwrap()))
            .unwrap_or_else(|e| panic!("{}: {e}", case["file"]));
        assert_eq!(
            stats.video_frames as usize,
            case["frames"].as_array().unwrap().len()
        );
    }
}

fn source_root(case: &serde_json::Value) -> PathBuf {
    root()
        .parent()
        .unwrap()
        .join(case["source_folder"].as_str().unwrap())
}
fn verify_native(raw: &fvid::playback_native::RawFrame, case: &serde_json::Value, index: usize) {
    let fvid::playback_native::RawFrame::Avc { picture, .. } = raw else {
        panic!("component precision narrowed")
    };
    assert_eq!(picture.dimensions(), (32, 32));
    assert_eq!(
        picture.bit_depth,
        case["luma_depth"].as_u64().unwrap() as u8
    );
    assert_eq!(
        picture.chroma_bit_depth,
        case["chroma_depth"].as_u64().unwrap() as u8
    );
    for (plane, key) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
        let data =
            fs::read(source_root(case).join(case["frames"][index][key].as_str().unwrap())).unwrap();
        let expected: Vec<_> = data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(*plane, expected);
    }
}
fn rgb_oracle(case: &serde_json::Value, index: usize) -> Vec<u8> {
    let planes: Vec<Vec<u16>> = ["y", "cb", "cr"]
        .iter()
        .map(|key| {
            fs::read(source_root(case).join(case["frames"][index][key].as_str().unwrap()))
                .unwrap()
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect()
        })
        .collect();
    let ys = (1u32 << (case["luma_depth"].as_u64().unwrap() - 8)) as f64;
    let cs = (1u32 << (case["chroma_depth"].as_u64().unwrap() - 8)) as f64;
    let mut result = Vec::new();
    for row in 0..32 {
        for col in 0..32 {
            let y = (f64::from(planes[0][row * 32 + col]) / ys - 16.) * 255. / 219.;
            let cb = (f64::from(planes[1][row / 2 * 16 + col / 2]) / cs - 128.) * 255. / 224.;
            let cr = (f64::from(planes[2][row / 2 * 16 + col / 2]) / cs - 128.) * 255. / 224.;
            for value in [
                y + 1.402 * cr,
                y - 0.344136 * cb - 0.714136 * cr,
                y + 1.772 * cb,
            ] {
                result.push(value.round().clamp(0., 255.) as u8);
            }
        }
    }
    result
}
#[test]
fn field_export_preserves_promoted_components_and_paired_timing() {
    use fvid_media::owned_webm::{Limits, WebmReader};
    for (index, case) in cases().iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-matroska-field-timing-{}-{index}.mkv",
            std::process::id()
        ));
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut reader = WebmReader::open(
            std::io::Cursor::new(fs::read(&output).unwrap()),
            Limits::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let track = reader.tracks[0].clone();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            32,
            32,
            &track.codec_private,
            32 << 20,
        )
        .unwrap();
        let count = case["frames"].as_array().unwrap().len();
        assert_eq!(reader.packets.len(), count);
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = yd.max(cd);
        for i in 0..count {
            let p = reader.packets[i].clone();
            assert_eq!(p.pts_ns, case["origin_ns"].as_i64().unwrap_or(0) + i as i64 * 20_000_000);
            assert_eq!(
                p.duration_ns.unwrap_or(track.default_duration_ns),
                20_000_000
            );
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            assert_eq!(decoded.depth, depth);
            assert_eq!((decoded.frame.width, decoded.frame.height), (32, 32));
            let mut expected = Vec::new();
            for (key, width, source_depth) in [("y", 32, yd), ("cb", 16, cd), ("cr", 16, cd)] {
                let bytes =
                    fs::read(source_root(case).join(case["frames"][i][key].as_str().unwrap()))
                        .unwrap();
                let values: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]) << (depth - source_depth))
                    .collect();
                for row in values.chunks_exact(width) {
                    for value in row.iter().rev() {
                        if depth == 8 {
                            expected.push(*value as u8)
                        } else {
                            expected.extend(value.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(decoded.frame.data, expected, "{} frame{i}", case["file"]);
        }
        fs::remove_file(output).unwrap();
    }
}
#[test]
fn invalid_field_bindings_refuse_without_publishing_a_frame_or_output() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (index, case) in manifest["invalid"].as_array().unwrap().iter().enumerate() {
        let source = root().join(case["file"].as_str().unwrap());
        let expected = case["error"].as_str().unwrap();
        let mut reader = fvid::playback_native::NativeReader::software(
            std::io::Cursor::new(fs::read(&source).unwrap()),
            32 << 20,
        )
        .unwrap();
        let error = match reader.read_frame_raw() {
            Err(e) => e.to_string(),
            Ok(_) => panic!("invalid field pair was published"),
        };
        assert!(error.contains(expected), "{error}");
        reader.rewind().unwrap();
        assert!(reader.read_frame_raw().is_err());
        let error = fvid_media::decode_video(&source).unwrap_err();
        assert!(error.contains(expected), "{error}");
        let output = std::env::temp_dir().join(format!(
            "fvid-invalid-field-{}-{index}.mkv",
            std::process::id()
        ));
        let _ = fs::remove_file(&output);
        let error = fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert!(!output.exists());
    }
}
#[test]
fn field_packet_limits_count_both_fields_and_publish_only_complete_success() {
    let source = root().join("y8-c14-bottom-p-weighted-explicit.mkv");
    for limit in 1..=4 {
        let output = std::env::temp_dir().join(format!(
            "fvid-pair-packet-limit-{}-{limit}.mkv",
            std::process::id()
        ));
        let options = fvid_control::CopyOptions {
            max_packets: Some(limit),
            ..Default::default()
        };
        let result = fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &options,
        );
        if limit < 4 {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("packet count exceeds limit")
            );
            assert!(!output.exists());
        } else {
            result.unwrap();
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 2);
            fs::remove_file(output).unwrap();
        }
    }
}
