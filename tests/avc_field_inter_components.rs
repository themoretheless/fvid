use fvid::codec::avc_decoder::AvcDecoder;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-field-inter-components")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&fs::read(root().join("manifest.json")).unwrap())
        .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn check(case: &serde_json::Value) {
    let mut decoder = AvcDecoder::new(
        &fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap(),
        32 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        for (index, packet) in case["packets"].as_array().unwrap().iter().enumerate() {
            let picture = decoder
                .decode_order(&fs::read(root().join(packet.as_str().unwrap())).unwrap())
                .unwrap_or_else(|e| panic!("{} packet{index}: {e}", case["file"]));
            if index % 2 == 0 {
                assert!(picture.is_none());
                assert!(decoder.has_pending_field());
                continue;
            }
            let picture = picture.unwrap();
            assert!(decoder.output_is_field_pair());
            assert_eq!(picture.dimensions(), (32, 32));
            assert_eq!(
                picture.component_depths(),
                [
                    case["luma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8
                ]
            );
            for (plane, key) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let data = fs::read(
                    root().join(
                        case["frames"][case["decode_order"][index / 2].as_u64().unwrap() as usize]
                            [key]
                            .as_str()
                            .unwrap(),
                    ),
                )
                .unwrap();
                let expected: Vec<_> = data
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                assert_eq!(*plane, expected, "{} {key}", case["file"]);
            }
        }
        assert!(!decoder.has_pending_field());
        decoder.reset();
    }
}
#[test]
fn equal_depth_field_controls() {
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] == c["chroma_depth"])
    {
        check(case)
    }
}
#[test]
fn independent_depth_field_acceptance() {
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        check(case)
    }
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
        let data = fs::read(root().join(case["frames"][index][key].as_str().unwrap())).unwrap();
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
            fs::read(root().join(case["frames"][index][key].as_str().unwrap()))
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
fn native_field_pairs_preserve_components_rgb_seek_and_eof() {
    use fvid::playback_native::NativeReader;
    for case in cases()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        let mut reader = NativeReader::software(
            std::io::Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let count = case["frames"].as_array().unwrap().len();
        for _ in 0..2 {
            for index in 0..count {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify_native(&raw, case, index);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (index as u128 * 2, index as u128 * 2 + 2, 100)
                );
                let rgb = raw.into_rgb(32 << 20).unwrap();
                let expected = rgb_oracle(case, index);
                assert_eq!(rgb.len(), expected.len());
                assert!(
                    rgb.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1),
                    "{} frame{index}",
                    case["file"]
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [count - 1, 0, count - 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index as u64 * 20))
                .unwrap()
                .unwrap();
            verify_native(&raw, case, index);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (index as u128 * 2, index as u128 * 2 + 2, 100)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn field_resource_failure_requires_reset_and_recovers() {
    let case = cases()
        .into_iter()
        .find(|c| c["luma_depth"] == 8 && c["chroma_depth"] == 14)
        .unwrap();
    let config = fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap();
    let first = fs::read(root().join(case["packets"][0].as_str().unwrap())).unwrap();
    let mut decoder = AvcDecoder::new(&config, first.len() * 2).unwrap();
    assert!(
        decoder
            .decode_order(&first)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert!(
        decoder
            .decode_order(&first)
            .unwrap_err()
            .to_string()
            .contains("reset")
    );
    decoder.reset();
    assert!(decoder.decode_order(&first).is_err());
    check(&case);
}
#[test]
fn field_export_preserves_promoted_components_and_paired_timing() {
    use fvid_media::owned_webm::{Limits, WebmReader};
    for (index, case) in cases().iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-field-inter-components-{}-{index}.mkv",
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
            assert_eq!(p.pts_ns, i as i64 * 20_000_000);
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
                    fs::read(root().join(case["frames"][i][key].as_str().unwrap())).unwrap();
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
