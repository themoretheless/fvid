use fvid::codec::avc_decoder::AvcDecoder;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mbaff-inter-components")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&fs::read(root().join("manifest.json")).unwrap())
        .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn bytes(value: &serde_json::Value) -> Vec<u8> {
    fs::read(root().join(value.as_str().unwrap())).unwrap()
}
fn check(case: &serde_json::Value) {
    let mut decoder = AvcDecoder::new(&bytes(&case["configuration"]), 32 << 20).unwrap();
    for _ in 0..2 {
        for index in case["decode_order"].as_array().unwrap() {
            let index = index.as_u64().unwrap() as usize;
            let frame = &case["frames"][index];
            let picture = decoder
                .decode_order(&bytes(&frame["packet"]))
                .unwrap_or_else(|e| panic!("{} frame{index}: {e}", case["file"]))
                .unwrap();
            assert_eq!(
                picture.component_depths(),
                [
                    case["luma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8
                ]
            );
            for (actual, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let expected: Vec<u16> = bytes(&frame[name])
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                assert_eq!(actual, &expected, "{} frame{index} {name}", case["file"]);
            }
        }
        decoder.reset();
    }
}
#[test]
fn equal_depth_inter_controls_decode_independent_samples() {
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] == c["chroma_depth"])
    {
        check(case);
    }
}
#[test]
fn mixed_depth_inter_acceptance_decodes_independent_samples() {
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        check(case);
    }
}

fn verify_raw(raw: &fvid::playback_native::RawFrame, case: &serde_json::Value, index: usize) {
    let fvid::playback_native::RawFrame::Avc { picture, .. } = raw else {
        panic!("mixed component precision was narrowed")
    };
    assert_eq!(
        picture.component_depths(),
        [
            case["luma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8
        ]
    );
    for (actual, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
        let expected: Vec<u16> = bytes(&case["frames"][index][name])
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        assert_eq!(actual, &expected, "{} frame{index} {name}", case["file"]);
    }
}
fn reference_rgb(case: &serde_json::Value, index: usize) -> Vec<u8> {
    let planes: Vec<Vec<u16>> = ["y", "cb", "cr"]
        .into_iter()
        .map(|p| {
            bytes(&case["frames"][index][p])
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect()
        })
        .collect();
    let ys = (1u32 << (case["luma_depth"].as_u64().unwrap() - 8)) as f64;
    let cs = (1u32 << (case["chroma_depth"].as_u64().unwrap() - 8)) as f64;
    let mut output = Vec::new();
    for row in 0..32 {
        for col in 0..16 {
            let y = (f64::from(planes[0][row * 16 + col]) - 16.0 * ys) / (219.0 * ys);
            let ci = row / 2 * 8 + col / 2;
            let cb = (f64::from(planes[1][ci]) - 128.0 * cs) / (224.0 * cs);
            let cr = (f64::from(planes[2][ci]) - 128.0 * cs) / (224.0 * cs);
            output.extend(
                [
                    y + 1.402 * cr,
                    y - 0.299 * 1.402 / 0.587 * cr - 0.114 * 1.772 / 0.587 * cb,
                    y + 1.772 * cb,
                ]
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8),
            );
        }
    }
    output
}
#[test]
fn native_inter_playback_preserves_pixels_presentation_order_seek_and_eof() {
    use fvid::playback_native::NativeReader;
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        let mut reader =
            NativeReader::software(std::io::Cursor::new(bytes(&case["file"])), 32 << 20).unwrap();
        let count = case["frames"].as_array().unwrap().len();
        for _ in 0..2 {
            for i in 0..count {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify_raw(&raw, case, i);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (i as u128, i as u128 + 1, 50)
                );
                let rgb = raw.into_rgb(32 << 20).unwrap();
                let expected = reference_rgb(case, i);
                assert_eq!(rgb.len(), expected.len());
                assert!(
                    rgb.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1),
                    "{} RGB frame{i}",
                    case["file"]
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for i in [count - 1, 1, 0, count - 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(i as u64 * 20))
                .unwrap()
                .unwrap();
            verify_raw(&raw, case, i);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (i as u128, i as u128 + 1, 50)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn lossless_inter_export_preserves_component_precision_and_reordered_clocks() {
    for (index, case) in cases().iter().enumerate().filter(|(_, c)| {
        matches!(
            (
                c["luma_depth"].as_u64().unwrap(),
                c["chroma_depth"].as_u64().unwrap()
            ),
            (8, 14) | (14, 8)
        )
    }) {
        let output = std::env::temp_dir().join(format!(
            "fvid-mbaff-inter-components-{}-{index}.mkv",
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
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            std::io::Cursor::new(fs::read(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let track = &reader.tracks[0];
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            track.width as usize,
            track.height as usize,
            &track.codec_private,
            32 << 20,
        )
        .unwrap();
        assert_eq!(
            reader.packets.len(),
            case["frames"].as_array().unwrap().len()
        );
        for i in 0..reader.packets.len() {
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            assert_eq!(decoded.depth, 14);
            assert_eq!(reader.packets[i].pts_ns, i as i64 * 20_000_000);
            assert_eq!(reader.packets[i].duration_ns, Some(20_000_000));
            let mut expected = Vec::new();
            for (name, width, depth) in [
                ("y", 16, case["luma_depth"].as_u64().unwrap()),
                ("cb", 8, case["chroma_depth"].as_u64().unwrap()),
                ("cr", 8, case["chroma_depth"].as_u64().unwrap()),
            ] {
                let values: Vec<u16> = bytes(&case["frames"][i][name])
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                for row in values.chunks_exact(width) {
                    expected.extend(
                        row.iter()
                            .rev()
                            .flat_map(|v| (v << (14 - depth)).to_le_bytes()),
                    );
                }
            }
            assert_eq!(decoded.frame.data, expected, "{} frame{i}", case["file"]);
        }
        fs::remove_file(output).unwrap();
    }
}
#[test]
fn inter_export_limits_refuse_without_publishing_partial_video() {
    let case = cases()
        .into_iter()
        .find(|c| {
            c["luma_depth"] == 8
                && c["chroma_depth"] == 14
                && c["mode"] == "b-bi"
                && c["cabac"] == false
        })
        .unwrap();
    for (index, options) in [
        fvid_control::CopyOptions {
            max_packets: Some(1),
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_packets: Some(2),
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let output = std::env::temp_dir().join(format!(
            "fvid-mbaff-inter-limit-{}-{index}.mkv",
            std::process::id()
        ));
        assert!(
            fvid_media::transcode_lossless(
                &root().join(case["file"].as_str().unwrap()),
                &output,
                Default::default(),
                &options
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}

#[test]
fn matroska_inter_playback_keeps_pixels_clocks_references_seek_and_eof() {
    use fvid::playback_native::NativeReader;
    for case in cases().iter().filter(|c| c["matroska"].is_string()) {
        let mut reader =
            NativeReader::software(std::io::Cursor::new(bytes(&case["matroska"])), 32 << 20)
                .unwrap();
        let count = case["frames"].as_array().unwrap().len();
        for _ in 0..2 {
            for index in 0..count {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify_raw(&raw, case, index);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (
                        index as u128 * 20_000_000,
                        (index as u128 + 1) * 20_000_000,
                        1_000_000_000
                    )
                );
                let rgb = raw.into_rgb(32 << 20).unwrap();
                let expected = reference_rgb(case, index);
                assert_eq!(rgb.len(), expected.len());
                assert!(rgb.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1));
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [count - 1, 1, 0, count - 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index as u64 * 20))
                .unwrap()
                .unwrap();
            verify_raw(&raw, case, index);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (
                    index as u128 * 20_000_000,
                    (index as u128 + 1) * 20_000_000,
                    1_000_000_000
                )
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
