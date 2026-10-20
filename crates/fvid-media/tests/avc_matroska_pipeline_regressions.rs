use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/avc-mbaff-inter-components")
}
#[test]
fn authored_avc_matroska_decodes_through_owned_pipeline() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let Some(name) = case["matroska"].as_str() else {
            continue;
        };
        let stats =
            fvid_media::decode_video(&root().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            stats.video_frames as usize,
            case["frames"].as_array().unwrap().len(),
            "{name}"
        );
        assert_eq!((stats.width, stats.height), (16, 32));
        assert_eq!(stats.decode_errors, 0);
    }
}
#[test]
fn matroska_avc_export_preserves_authored_components_and_presentation_clocks() {
    use fvid_media::owned_webm::{Limits, WebmReader};
    use std::io::Cursor;
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (index, case) in manifest["cases"].as_array().unwrap().iter().enumerate() {
        let Some(name) = case["matroska"].as_str() else {
            continue;
        };
        let output = std::env::temp_dir().join(format!(
            "fvid-avc-matroska-{}-{index}.mkv",
            std::process::id()
        ));
        fvid_media::transcode_lossless(
            &root().join(name),
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut reader =
            WebmReader::open(Cursor::new(fs::read(&output).unwrap()), Limits::default()).unwrap();
        reader.scan_all().unwrap();
        let track = reader.tracks[0].clone();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            16,
            32,
            &track.codec_private,
            32 << 20,
        )
        .unwrap();
        let frames = case["frames"].as_array().unwrap();
        assert_eq!(reader.packets.len(), frames.len(), "{name}");
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = yd.max(cd);
        for (i, expected) in frames.iter().enumerate() {
            let packet = reader.packets[i].clone();
            assert_eq!(packet.pts_ns, i as i64 * 20_000_000, "{name}");
            assert_eq!(
                packet.duration_ns.unwrap_or(track.default_duration_ns),
                20_000_000
            );
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            assert_eq!(decoded.depth, depth);
            assert_eq!(decoded.frame.subsampling, Some([2, 2]));
            assert_eq!((decoded.frame.width, decoded.frame.height), (16, 32));
            let mut oracle = Vec::new();
            for (key, width, source_depth) in [("y", 16, yd), ("cb", 8, cd), ("cr", 8, cd)] {
                let bytes = fs::read(root().join(expected[key].as_str().unwrap())).unwrap();
                let values: Vec<_> = bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]) << (depth - source_depth))
                    .collect();
                for row in values.chunks_exact(width) {
                    for value in row.iter().rev() {
                        if depth == 8 {
                            oracle.push(*value as u8);
                        } else {
                            oracle.extend(value.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(decoded.frame.data, oracle, "{name} frame{i}");
        }
        fs::remove_file(output).unwrap();
    }
}
fn exported_frames(path: &Path) -> Vec<(Vec<u8>, i64, u64)> {
    use fvid_media::owned_webm::{Limits, WebmReader};
    let mut reader = WebmReader::open(
        std::io::Cursor::new(fs::read(path).unwrap()),
        Limits::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let track = reader.tracks[0].clone();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        16,
        32,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    (0..reader.packets.len())
        .map(|i| {
            let p = reader.packets[i].clone();
            (
                decoder
                    .decode(&reader.read_packet(i).unwrap())
                    .unwrap()
                    .frame
                    .data,
                p.pts_ns,
                p.duration_ns.unwrap_or(track.default_duration_ns),
            )
        })
        .collect()
}
#[test]
fn reordered_avc_interval_and_reverse_keep_presentation_bindings() {
    for (index, name) in ["y8-c14-frame-cavlc-b-bi.mkv", "y14-c8-field-cabac-b-bi.mkv"]
        .iter()
        .enumerate()
    {
        let source = root().join(name);
        let output =
            std::env::temp_dir().join(format!("fvid-avc-order-{}-{index}.mkv", std::process::id()));
        fvid_media::transcode_lossless(&source, &output, Default::default(), &Default::default())
            .unwrap();
        let reference = exported_frames(&output);
        fs::remove_file(&output).unwrap();
        assert_eq!(reference.len(), 3);
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                interval: Some((20_000, 40_000)),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let clipped = exported_frames(&output);
        fs::remove_file(&output).unwrap();
        assert_eq!(clipped, vec![(reference[1].0.clone(), 0, 20_000_000)]);
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                reverse: Some(String::new()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let reversed = exported_frames(&output);
        fs::remove_file(&output).unwrap();
        assert_eq!(reversed.len(), 3);
        for (i, frame) in reversed.iter().enumerate() {
            assert_eq!(frame.0, reference[2 - i].0);
            assert_eq!((frame.1, frame.2), (i as i64 * 20_000_000, 20_000_000));
        }
    }
}
#[test]
fn avc_policy_refusal_does_not_publish_output() {
    let source = root().join("y8-c14-frame-cavlc-b-bi.mkv");
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    for (index, options) in [
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
        fvid_control::CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        },
    ]
    .iter()
    .enumerate()
    {
        let output = std::env::temp_dir().join(format!(
            "fvid-avc-policy-{}-{index}.mkv",
            std::process::id()
        ));
        let _ = fs::remove_file(&output);
        assert!(
            fvid_media::transcode_lossless(
                &source,
                &output,
                fvid_media::LosslessTransform {
                    horizontal_flip: true,
                    ..Default::default()
                },
                options
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}
#[test]
fn missing_avc_durations_are_inferred_in_presentation_order() {
    let folder = root().parent().unwrap().join("avc-matroska-pipeline");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("manifest.json")).unwrap()).unwrap();
    for (index, case) in manifest["cases"].as_array().unwrap().iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-avc-untimed-{}-{index}.mkv",
            std::process::id()
        ));
        let control = root().join(case["control"].as_str().unwrap());
        fvid_media::transcode_lossless(&control, &output, Default::default(), &Default::default())
            .unwrap();
        let expected = exported_frames(&output);
        fs::remove_file(&output).unwrap();
        fvid_media::transcode_lossless(
            &folder.join(case["file"].as_str().unwrap()),
            &output,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(exported_frames(&output), expected);
        fs::remove_file(&output).unwrap();
    }
}
