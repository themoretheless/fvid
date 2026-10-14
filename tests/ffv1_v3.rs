//! Independent v3 sample arrays, custom range/Rice streams and raster adaptation.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-v3-samples.json")).unwrap()).unwrap()
}
#[test]
fn all_v3_coders_depths_and_rasters_preserve_every_sample_and_context_history() {
    for case in manifest()["cases"].as_array().unwrap() {
        let private = std::fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap();
        let w = case["size"][0].as_u64().unwrap() as usize;
        let h = case["size"][1].as_u64().unwrap() as usize;
        let mut decoder =
            fvid_media::owned_ffv1_decoder::Decoder::from_configuration(w, h, &private, 32 << 20)
                .unwrap();
        for _ in 0..2 {
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
                let decoded = decoder
                    .decode(&packet)
                    .unwrap_or_else(|e| panic!("{}: {e}", frame["packet"]));
                assert_eq!(decoded.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(decoded.keyframe, index == 0);
                assert_eq!(decoded.frame.subsampling.is_none(), case["layout"] == "rgb");
                assert_eq!(decoder.monochrome(), Some(case["layout"] == "gray"));
                assert_eq!(
                    decoded.frame.data,
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap(),
                    "{}",
                    frame["packet"]
                );
            }
            decoder.reset();
        }
    }
}
#[test]
fn v3_native_playback_seek_and_rewind_preserve_depth_and_interval() {
    for case in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        for _ in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let fvid::playback_native::RawFrame::Planar(p) = raw else {
                    panic!("v3 source depth lost")
                };
                assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(
                    p.frame.data,
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap()
                );
                let (start, end, scale) = reader.frame_interval().unwrap();
                assert_eq!(
                    (
                        start * 1000 / u128::from(scale),
                        end * 1000 / u128::from(scale)
                    ),
                    (i as u128 * 20, (i as u128 + 1) * 20)
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        let raw = reader
            .seek_raw(std::time::Duration::from_millis(20))
            .unwrap()
            .unwrap();
        let fvid::playback_native::RawFrame::Planar(p) = raw else {
            panic!("v3 seek source depth lost")
        };
        assert_eq!(
            p.frame.data,
            std::fs::read(root().join(case["frames"][1]["reference"].as_str().unwrap())).unwrap()
        );
    }
}
#[test]
fn v3_sources_export_through_owned_ffv1_without_precision_loss() {
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let target =
            std::env::temp_dir().join(format!("fvid-v3-export-{}-{index}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&target);
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
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
        for frame in case["frames"].as_array().unwrap() {
            let fvid::playback_native::RawFrame::Planar(p) =
                reader.read_frame_raw().unwrap().unwrap()
            else {
                panic!("v3 export source depth lost")
            };
            assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
            assert_eq!(
                p.frame.data,
                std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap()
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(target).unwrap();
    }
}

#[test]
fn damaged_v3_frames_fail_exactly_and_invalidate_adaptation_until_keyframe() {
    let manifest = manifest();
    let private =
        std::fs::read(root().join(manifest["invalid_configuration"].as_str().unwrap())).unwrap();
    let good = std::fs::read(root().join(manifest["recovery"].as_str().unwrap())).unwrap();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| {
            c["layout"] == "gray"
                && c["depth"] == 8
                && c["coder"] == 1
                && c["grid"] == serde_json::json!([3, 2])
        })
        .unwrap();
    let nonkey = std::fs::read(root().join(case["frames"][1]["packet"].as_str().unwrap())).unwrap();
    for invalid in manifest["invalid"].as_array().unwrap() {
        let packet = std::fs::read(root().join(invalid["file"].as_str().unwrap())).unwrap();
        let mut decoder =
            fvid_media::owned_ffv1_decoder::Decoder::from_configuration(18, 12, &private, 32 << 20)
                .unwrap();
        decoder.decode(&good).unwrap();
        assert_eq!(
            decoder.decode(&packet).err().unwrap(),
            invalid["error"].as_str().unwrap()
        );
        assert_eq!(decoder.monochrome(), None);
        assert_eq!(
            decoder.decode(&nonkey).err().unwrap(),
            "FFV1 frame requires a preceding keyframe"
        );
        let recovered = decoder.decode(&good).unwrap();
        assert!(recovered.frame.data[..18 * 12].iter().all(|&v| v == 0));
        let target = std::env::temp_dir().join(format!(
            "fvid-v3-invalid-{}-{}.mkv",
            std::process::id(),
            invalid["file"].as_str().unwrap()
        ));
        let _ = std::fs::remove_file(&target);
        assert_eq!(
            fvid_media::transcode_lossless(
                &root().join(invalid["movie"].as_str().unwrap()),
                &target,
                Default::default(),
                &Default::default()
            )
            .unwrap_err(),
            invalid["error"].as_str().unwrap()
        );
        assert!(!target.exists());
    }
}

#[test]
fn v3_context_history_output_and_scratch_obey_the_aggregate_budget() {
    for case in manifest()["cases"].as_array().unwrap().iter().filter(|c| {
        c["depth"] == 16 && c["layout"] == "rgb" && c["grid"] == serde_json::json!([3, 2])
    }) {
        let private = std::fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap();
        let packet =
            std::fs::read(root().join(case["frames"][0]["packet"].as_str().unwrap())).unwrap();
        let next =
            std::fs::read(root().join(case["frames"][1]["packet"].as_str().unwrap())).unwrap();
        let works = |budget| {
            let Ok(mut decoder) = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
                18, 12, &private, budget,
            ) else {
                return false;
            };
            decoder.decode(&packet).is_ok() && decoder.decode(&next).is_ok()
        };
        let (mut low, mut high) = (0, 32 << 20);
        while low < high {
            let middle = low + (high - low) / 2;
            if works(middle) {
                high = middle
            } else {
                low = middle + 1
            }
        }
        assert!(low > 0 && low < 32 << 20);
        assert!(works(low));
        assert!(!works(low - 1));
    }
}

#[test]
fn v3_nonkey_quantizer_change_now_preserves_exact_pixels() {
    let manifest = manifest();
    let private =
        std::fs::read(root().join(manifest["invalid_configuration"].as_str().unwrap())).unwrap();
    let good = std::fs::read(root().join(manifest["recovery"].as_str().unwrap())).unwrap();
    for case in manifest["quantizer_changes"].as_array().unwrap() {
        let packet = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let config =
            fvid_media::owned_ffv1_decoder::Configuration::parse(&private, 32 << 20).unwrap();
        assert_eq!(config.slices(&packet, 32 << 20).unwrap().len(), 6);
        let mut decoder =
            fvid_media::owned_ffv1_decoder::Decoder::from_configuration(18, 12, &private, 32 << 20)
                .unwrap();
        decoder.decode(&good).unwrap();
        assert_eq!(
            decoder.decode(&packet).unwrap().frame.data,
            std::fs::read(root().join(case["reference"].as_str().unwrap())).unwrap()
        );
    }
}
