//! Independently authored Rice bitstreams, sample arrays and adaptation traces.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-rice.json")).unwrap()).unwrap()
}
#[test]
fn rice_run_scalar_escape_and_persistent_contexts_decode_exact_samples() {
    let mut escapes = 0;
    let mut runs = 0;
    let mut halves = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(
            case["size"][0].as_u64().unwrap() as usize,
            case["size"][1].as_u64().unwrap() as usize,
            32 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
                let expected =
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap();
                let actual = decoder
                    .decode(&packet)
                    .unwrap_or_else(|e| panic!("{}: {e}", frame["packet"]));
                assert_eq!(actual.keyframe, index == 0);
                assert_eq!(actual.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(actual.frame.subsampling.is_none(), case["layout"] == "rgb");
                assert_eq!(actual.frame.data, expected, "{}", frame["packet"]);
                for trace in frame["trace"].as_array().unwrap() {
                    escapes += usize::from(trace["escape"] == true);
                    runs += usize::from(trace["kind"] == "run");
                    halves += usize::from(trace["state"]["count"] == 128);
                }
            }
            decoder.reset();
        }
    }
    assert!(escapes > 0 && runs > 0 && halves > 0);
}
#[test]
fn rice_matroska_playback_rewind_and_seek_keep_every_sample() {
    for case in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let expected =
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap();
                let fvid::playback_native::RawFrame::Planar(p) =
                    reader.read_frame_raw().unwrap().unwrap()
                else {
                    panic!("Rice depth lost")
                };
                assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(p.frame.data, expected);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        let fvid::playback_native::RawFrame::Planar(p) = reader
            .seek_raw(std::time::Duration::from_millis(
                if case["frames"].as_array().unwrap().len() == 1 {
                    0
                } else {
                    20
                },
            ))
            .unwrap()
            .unwrap()
        else {
            panic!("Rice seek depth lost")
        };
        let frame = case["frames"].as_array().unwrap().last().unwrap();
        assert_eq!(
            p.frame.data,
            std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap()
        );
    }
}

#[test]
fn malformed_rice_invalidates_history_and_keyframe_recovers() {
    let manifest = manifest();
    let nonkey = std::fs::read(
        root().join(
            manifest["cases"][0]["frames"][1]["packet"]
                .as_str()
                .unwrap(),
        ),
    )
    .unwrap();
    for case in manifest["invalid"].as_array().unwrap() {
        let bad = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let good = std::fs::read(root().join(case["valid"].as_str().unwrap())).unwrap();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(17, 9, 32 << 20).unwrap();
        decoder.decode(&good).unwrap();
        assert_eq!(
            decoder.decode(&bad).err().unwrap(),
            case["error"].as_str().unwrap()
        );
        assert_eq!(
            decoder.decode(&nonkey).err().unwrap(),
            "FFV1 frame requires a preceding keyframe"
        );
        decoder.decode(&good).unwrap();
        decoder.decode(&nonkey).unwrap();
        let mut bounded = fvid_media::owned_ffv1_decoder::Decoder::new(17, 9, 1).unwrap();
        assert_eq!(
            bounded.decode(&good).err().unwrap(),
            "FFV1 contexts exceed memory budget"
        );
    }
    assert_eq!(manifest["invalid"].as_array().unwrap().len(), 3);
}

#[test]
fn rice_sources_export_through_owned_range_ffv1_without_sample_changes() {
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let target = std::env::temp_dir().join(format!(
            "fvid-rice-export-{}-{index}.mkv",
            std::process::id()
        ));
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
                panic!("Rice export lost precision")
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
fn rice_contexts_and_frame_storage_obey_the_caller_budget() {
    let manifest = manifest();
    for layout in ["gray", "rgb"] {
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["layout"] == layout && c["depth"] == 16)
            .unwrap();
        let frame = &case["frames"][0];
        let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
        let mut low = 0;
        let mut high = 32 << 20;
        while low < high {
            let budget = low + (high - low) / 2;
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(17, 9, budget).unwrap();
            if decoder.decode(&packet).is_ok() {
                high = budget;
            } else {
                low = budget + 1;
            }
        }
        assert!(low > 0 && low < 32 << 20);
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(17, 9, low).unwrap();
        assert_eq!(
            decoder.decode(&packet).unwrap().frame.data,
            std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap()
        );
        let mut below = fvid_media::owned_ffv1_decoder::Decoder::new(17, 9, low - 1).unwrap();
        assert!(
            below
                .decode(&packet)
                .err()
                .unwrap()
                .contains("memory budget")
        );
    }
}
