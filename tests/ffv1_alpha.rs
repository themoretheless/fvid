//! Alpha codec API acceptance; presentation/streaming refusal is checked separately.
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
#[test]
fn alpha_presentation_and_filtered_export_remain_explicit_refusals() {
    let m = manifest();
    let v3 = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["version"] == 3)
        .unwrap();
    for (index, case) in [&m["cases"][0], v3].into_iter().enumerate() {
        let data = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let error = match fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20)
        {
            Err(e) => e,
            Ok(mut reader) => reader
                .read_frame_raw()
                .err()
                .expect("alpha presentation must not silently drop opacity"),
        };
        assert!(
            error
                .to_string()
                .contains("FFV1 alpha presentation is not implemented")
        );
        let target = std::env::temp_dir().join(format!(
            "fvid-alpha-refusal-{}-{index}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&target);
        let error = fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &target,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "owned Matroska lossless export does not yet support this codec, format or transform"
        );
        assert!(!target.exists());
    }
}
