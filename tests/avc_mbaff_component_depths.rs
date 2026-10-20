use fvid::codec::avc_decoder::AvcDecoder;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mbaff-component-depths")
}
fn manifest() -> Vec<serde_json::Value> {
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
        for frame in case["frames"].as_array().unwrap() {
            let picture = decoder
                .decode(&bytes(&frame["packet"]))
                .unwrap_or_else(|e| panic!("{}: {e}", case["file"]))
                .unwrap();
            assert_eq!(
                picture.component_depths(),
                [
                    case["luma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8
                ]
            );
            for (plane, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let expected: Vec<u16> = bytes(&frame[name])
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                assert_eq!(plane, &expected, "{} {name}", case["file"]);
            }
        }
        decoder.reset();
    }
}
#[test]
fn equal_depth_frame_and_field_pair_controls_decode_exact_samples() {
    for case in manifest()
        .iter()
        .filter(|c| c["luma_depth"] == c["chroma_depth"])
    {
        check(case);
    }
}
#[test]
fn mixed_depth_frame_and_field_pairs_decode_exact_samples() {
    for case in manifest()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        check(case);
    }
}

#[test]
fn native_mp4_playback_keeps_component_samples_clocks_seek_and_eof() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for case in manifest()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        let mut reader =
            NativeReader::software(std::io::Cursor::new(bytes(&case["file"])), 32 << 20).unwrap();
        let verify = |raw: &RawFrame, index: usize| {
            let RawFrame::Avc { picture, .. } = raw else {
                panic!("mixed precision was narrowed")
            };
            for (plane, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let expected: Vec<u16> = bytes(&case["frames"][index][name])
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                assert_eq!(plane, &expected, "{} {name}", case["file"]);
            }
        };
        let mut clocks = Vec::new();
        for pass in 0..2 {
            for i in 0..2 {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify(&raw, i);
                let clock = reader.frame_interval().unwrap();
                assert_eq!(clock, (i as u128, i as u128 + 1, 50));
                if pass == 0 {
                    clocks.push(clock);
                } else {
                    assert_eq!(clock, clocks[i]);
                }
                let rgb = raw.into_rgb(32 << 20).unwrap();
                let expected = reference_rgb(case, i);
                assert_eq!(rgb.len(), expected.len());
                assert!(
                    rgb.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 1),
                    "{} RGB",
                    case["file"]
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for i in [1, 0, 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(i as u64 * 20))
                .unwrap()
                .unwrap();
            verify(&raw, i);
            assert_eq!(reader.frame_interval().unwrap(), clocks[i]);
        }
    }
}

fn reference_rgb(case: &serde_json::Value, index: usize) -> Vec<u8> {
    let planes: Vec<Vec<u16>> = ["y", "cb", "cr"]
        .into_iter()
        .map(|name| {
            bytes(&case["frames"][index][name])
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
fn mixed_depth_mbaff_lossless_export_preserves_every_component_bit() {
    for (index, case) in manifest().iter().enumerate().filter(|(_, c)| {
        matches!(
            (
                c["luma_depth"].as_u64().unwrap(),
                c["chroma_depth"].as_u64().unwrap()
            ),
            (8, 14) | (14, 8)
        )
    }) {
        let output = std::env::temp_dir().join(format!(
            "fvid-mbaff-components-{}-{index}.mkv",
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
        assert_eq!(reader.packets.len(), 2);
        for i in 0..2 {
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
            assert_eq!(decoded.frame.data, expected, "{}", case["file"]);
        }
        fs::remove_file(output).unwrap();
    }
}

#[test]
fn previously_refused_inter_companions_preserve_reference_samples() {
    for case in manifest().iter().filter(|c| {
        c["mode"] == "pcm"
            && matches!(
                (
                    c["luma_depth"].as_u64().unwrap(),
                    c["chroma_depth"].as_u64().unwrap()
                ),
                (8, 14) | (14, 8)
            )
    }) {
        let name = case["file"]
            .as_str()
            .unwrap()
            .trim_end_matches(".mp4")
            .to_owned()
            + "-inter-unsupported.mp4";
        let mut input = fvid::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(fs::read(root().join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(input.tracks()[0].samples.len(), 2);
        let mut decoder = AvcDecoder::new(&input.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        let first = decoder.decode(&packet).unwrap().unwrap();
        input.read_packet(0, 1, &mut packet).unwrap();
        let second = decoder.decode(&packet).unwrap().unwrap();
        assert_eq!(second.component_depths(), first.component_depths());
        assert_eq!(second.y, first.y);
        assert_eq!(second.cb, first.cb);
        assert_eq!(second.cr, first.cr);
    }
}

#[test]
fn mixed_depth_mbaff_cannot_bypass_decoder_budget_or_packet_framing() {
    let case = manifest()
        .into_iter()
        .find(|c| c["luma_depth"] == 8 && c["chroma_depth"] == 14 && c["mode"] == "pcm")
        .unwrap();
    let config = bytes(&case["configuration"]);
    let packet = bytes(&case["frames"][0]["packet"]);
    let mut limited = AvcDecoder::new(&config, 1).unwrap();
    assert!(limited.decode(&packet).is_err());
    for removed in [1, 2, 17, packet.len() / 2] {
        let mut decoder = AvcDecoder::new(&config, 32 << 20).unwrap();
        assert!(decoder.decode(&packet[..packet.len() - removed]).is_err());
        assert!(
            decoder
                .decode(&packet)
                .err()
                .unwrap()
                .to_string()
                .contains("reset")
        );
        decoder.reset();
        assert!(decoder.decode(&packet).unwrap().is_some());
    }
}
