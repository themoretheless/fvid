//! Authored VP9 identity headers and independently calculated GBR sample oracles.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("vp9-rgb.json")).unwrap()).unwrap()
}
#[test]
fn native_rgb_vp9_preserves_channels_depth_display_and_seek() {
    for case in cases()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let reference =
            std::fs::read(root().join(case["files"]["rgb"]["file"].as_str().unwrap())).unwrap();
        let packet =
            std::fs::read(root().join(case["files"]["packet"]["file"].as_str().unwrap())).unwrap();
        let decoded = fvid::codec::vp9_decoder::Decoder::new(32 << 20)
            .decode(&packet)
            .unwrap();
        assert_eq!(decoded.header.picture.format.color_space, 7);
        assert_eq!(decoded.header.picture.format.subsampling, [false, false]);
        assert!(decoded.header.picture.format.full_range);
        for extension in ["mp4", "webm"] {
            let input =
                std::fs::read(root().join(case["files"][extension]["file"].as_str().unwrap()))
                    .unwrap();
            let mut reader =
                fvid::playback_native::NativeReader::software(Cursor::new(input), 32 << 20)
                    .unwrap();
            for _ in 0..2 {
                for _ in 0..2 {
                    let raw = reader.read_frame_raw().unwrap().unwrap();
                    let fvid::playback_native::RawFrame::Planar(p) = &raw else {
                        panic!("RGB precision lost")
                    };
                    assert_eq!(p.depth, depth);
                    assert_eq!(p.frame.subsampling, None);
                    assert_eq!(p.frame.data, reference);
                    let maximum = (1u32 << depth) - 1;
                    let expected = if depth == 8 {
                        reference.clone()
                    } else {
                        reference
                            .chunks_exact(2)
                            .map(|p| {
                                ((u32::from(u16::from_le_bytes([p[0], p[1]])) * 255 + maximum / 2)
                                    / maximum) as u8
                            })
                            .collect()
                    };
                    assert_eq!(raw.into_rgb(32 << 20).unwrap(), expected);
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                reader.rewind().unwrap();
            }
            let fvid::playback_native::RawFrame::Planar(p) =
                reader.seek_raw(std::time::Duration::ZERO).unwrap().unwrap()
            else {
                panic!("seek lost RGB")
            };
            assert_eq!(p.frame.data, reference);
            reader.rewind().unwrap();
            assert!(reader.read_frame().unwrap());
            let maximum = (1u32 << depth) - 1;
            let display: Vec<u8> = if depth == 8 {
                reference.clone()
            } else {
                reference
                    .chunks_exact(2)
                    .map(|p| {
                        ((u32::from(u16::from_le_bytes([p[0], p[1]])) * 255 + maximum / 2)
                            / maximum) as u8
                    })
                    .collect()
            };
            assert_eq!(reader.rgb(), display);
        }
    }
}
#[test]
fn rgb_vp9_exports_exact_samples_with_owned_ffv1_and_negate() {
    for case in cases()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let original =
            std::fs::read(root().join(case["files"]["rgb"]["file"].as_str().unwrap())).unwrap();
        for extension in ["mp4", "webm"] {
            for negate in [false, true] {
                let mut expected = original.clone();
                if negate {
                    if depth == 8 {
                        for v in &mut expected {
                            *v = 255 - *v;
                        }
                    } else {
                        for s in expected.chunks_exact_mut(2) {
                            let v = ((1u16 << depth) - 1) - u16::from_le_bytes([s[0], s[1]]);
                            s.copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                let target = std::env::temp_dir().join(format!(
                    "fvid-vp9-rgb-{}-{depth}-{extension}-{negate}.mkv",
                    std::process::id()
                ));
                let _ = std::fs::remove_file(&target);
                fvid_media::transcode_lossless(
                    &root().join(case["files"][extension]["file"].as_str().unwrap()),
                    &target,
                    fvid::media_info::LosslessTransform {
                        negate: negate.then(String::new),
                        ..Default::default()
                    },
                    &Default::default(),
                )
                .unwrap();
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(&target).unwrap()),
                    32 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let fvid::playback_native::RawFrame::Planar(p) =
                        reader.read_frame_raw().unwrap().unwrap()
                    else {
                        panic!("export lost RGB")
                    };
                    assert_eq!(p.depth, depth);
                    assert_eq!(p.frame.subsampling, None);
                    assert_eq!(p.frame.data, expected);
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                std::fs::remove_file(target).unwrap();
            }
        }
    }
}

#[test]
fn mp4_vp9_configuration_mismatch_fails_before_output_publication() {
    for (i, case) in cases()["invalid"].as_array().unwrap().iter().enumerate() {
        let target =
            std::env::temp_dir().join(format!("fvid-vp9-config-{}-{i}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&target);
        let error = fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &target,
            Default::default(),
            &Default::default(),
        )
        .unwrap_err();
        assert_eq!(error, case["error"].as_str().unwrap());
        assert!(!target.exists());
    }
}

#[test]
fn mp4_vp9_yuv_exports_preserve_all_existing_chroma_and_depth_oracles() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("vp9-chroma.json")).unwrap()).unwrap();
    for (i, case) in manifest["cases"].as_array().unwrap().iter().enumerate() {
        let Some(file) = case["mp4"].as_str() else {
            continue;
        };
        let depth = case["depth"].as_u64().unwrap() as u8;
        let oracle = std::fs::read(root().join(case["oracle"].as_str().unwrap())).unwrap();
        let expected = if depth == 8 {
            oracle.chunks_exact(2).map(|p| p[0]).collect()
        } else {
            oracle
        };
        let target =
            std::env::temp_dir().join(format!("fvid-vp9-yuv-{}-{i}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&target);
        fvid_media::transcode_lossless(
            &root().join(file),
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
        let fvid::playback_native::RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap()
        else {
            panic!("VP9 export lost precision")
        };
        assert_eq!(p.depth, depth);
        assert!(p.frame.subsampling.is_some());
        assert_eq!(p.frame.data, expected, "{file}");
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(target).unwrap();
    }
}
