use fvid_media::{owned_frame::GeometryFrame, owned_yaepblur::YaepBlur};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn frame(n: usize) -> GeometryFrame {
    GeometryFrame {
        width: 16,
        height: 12,
        subsampling: Some([2, 2]),
        data: (0..288)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect(),
    }
}
#[test]
fn qualified_default_strong_masks_and_rewind_use_owned_statistics() {
    for (args, name) in [("", "default"), ("r=4:p=7:s=1024", "strong")] {
        let expected = std::fs::read(fixture(&format!("yaepblur-{name}.expected.raw"))).unwrap();
        let program = YaepBlur::parse(args).unwrap();
        for _ in 0..2 {
            let mut result = Vec::new();
            for n in 0..4 {
                let mut f = frame(n);
                program
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                result.extend(f.data);
            }
            assert_eq!(result, expected);
        }
    }
    let mut a = frame(0);
    let mut b = frame(0);
    YaepBlur::parse("4:7:1024")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    YaepBlur::parse("radius=4:planes=7:sigma=1024")
        .unwrap()
        .apply(&mut b, 8, 0, None)
        .unwrap();
    assert_eq!(a.data, b.data);
    for args in ["radius=0", "planes=0", "planes=8", "enable='eq(n,2)'"] {
        let mut f = frame(0);
        YaepBlur::parse(args)
            .unwrap()
            .apply(&mut f, 8, 0, None)
            .unwrap();
        assert_eq!(f.data, frame(0).data);
    }
}
#[test]
fn explicit_rgb_alpha_high_depth_and_transactional_refusals() {
    let mut rgb = GeometryFrame {
        width: 2,
        height: 2,
        subsampling: None,
        data: vec![0, 0, 0, 100, 255, 200, 50, 0, 60, 70, 255, 80],
    };
    YaepBlur::parse("r=1:s=100000")
        .unwrap()
        .apply(&mut rgb, 8, 0, None)
        .unwrap();
    assert_eq!(
        rgb.data,
        [0, 109, 0, 100, 144, 200, 50, 109, 60, 70, 144, 80]
    );
    for depth in [8u8, 9, 10, 12, 14, 16] {
        let value = (1u32 << depth) - 1;
        let mut plane = if depth == 8 {
            vec![value as u8; 17 * 13]
        } else {
            (0..17 * 13)
                .flat_map(|_| (value as u16).to_le_bytes())
                .collect()
        };
        let old = plane.clone();
        YaepBlur::parse("r=2147483647:p=8:s=2147483647")
            .unwrap()
            .apply_plane(&mut plane, 17, 13, depth, 3, 0, None)
            .unwrap();
        assert_eq!(old, plane);
    }
    for args in [
        "r=-1",
        "r=2147483648",
        "p=16",
        "p=-1",
        "s=0",
        "s=NaN",
        "r=1.5",
        "unknown=1",
        "1:1:1:1",
    ] {
        assert!(YaepBlur::parse(args).is_err(), "{args}");
    }
    let program = YaepBlur::parse("").unwrap();
    let mut invalid = vec![255, 255];
    let old = invalid.clone();
    assert!(
        program
            .apply_plane(&mut invalid, 1, 1, 10, 0, 0, None)
            .is_err()
    );
    assert_eq!(invalid, old);
    let mut f = GeometryFrame {
        width: 2,
        height: 2,
        subsampling: Some([2, 2]),
        data: vec![1, 2],
    };
    let old = f.data.clone();
    assert!(program.apply(&mut f, 8, 0, None).is_err());
    assert_eq!(old, f.data);
    for library in [false, true] {
        let request = fvid::media::DecodeTransform {
            yaepblur: Some("r=2147483647".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&fixture("yaepblur-small.y4m"), request)
        } else {
            fvid::media::decode_video_transformed(&fixture("yaepblur-small.y4m"), request)
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
    }
}
#[test]
fn root_and_library_exports_preserve_visualization_and_timeline() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("yaepblur-strong.expected.raw")).unwrap();
    for (source, full) in [
        (fixture("yaepblur.y4m"), false),
        (fixture("yaepblur-full.y4m"), true),
    ] {
        for library in [false, true] {
            for step in [1usize, 2] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-yaep-{}-{library}-{step}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    yaepblur: Some(
                        if step == 1 {
                            "r=4:p=7:s=1024"
                        } else {
                            "r=4:p=7:s=1024:enable='eq(n,2)'"
                        }
                        .into(),
                    ),
                    framestep: Some(step.to_string()),
                    ..Default::default()
                };
                let stats = if library {
                    fvid_media::transcode_lossless(
                        &source,
                        &output,
                        transform,
                        &CopyOptions::default(),
                    )
                } else {
                    fvid::media::transcode_lossless(
                        &source,
                        &output,
                        transform,
                        &CopyOptions::default(),
                    )
                }
                .unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(stats.video_frames, 4 / step as u64);
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                assert_eq!(reader.colour().full_range, full);
                for n in (0..4).step_by(step) {
                    let raw = reader.read_frame_raw().unwrap().unwrap();
                    let result = fvid::native_geometry::VideoGeometry::default()
                        .apply(&raw, 16, 12)
                        .unwrap();
                    if step == 2 && n == 0 {
                        assert_eq!(result.data, frame(0).data)
                    } else {
                        assert_eq!(result.data, &expected[n * 288..(n + 1) * 288]);
                    }
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                std::fs::remove_file(output).unwrap();
            }
        }
    }
}
#[test]
fn cli_decode_export_and_lossless_use_owned_filter() {
    let source = fixture("yaepblur.y4m");
    let expected = std::fs::read(fixture("yaepblur-strong.expected.raw")).unwrap();
    let y4m = std::env::temp_dir().join(format!("fvid-yaep-cli-{}.y4m", std::process::id()));
    let mkv = y4m.with_extension("mkv");
    for (command, output) in [
        ("decode", None),
        ("export-y4m", Some(&y4m)),
        ("transcode-lossless", Some(&mkv)),
    ] {
        let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        process.args(["media", command]).arg(&source);
        if let Some(output) = output {
            process.arg(output);
        }
        process.args(["--yaepblur", "r=4:p=7:s=1024"]);
        let result = process.output().unwrap();
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for path in [&y4m, &mkv] {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(path).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut data = Vec::new();
        while let Some(f) = reader.read_frame_raw().unwrap() {
            data.extend(
                fvid::native_geometry::VideoGeometry::default()
                    .apply(&f, 16, 12)
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(data, expected);
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn ffv1_sources_clipping_and_native_codecs_keep_owned_routing() {
    use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
    let base = std::env::temp_dir().join(format!("fvid-yaep-source-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &fixture("yaepblur.y4m"),
        &base,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    let expected = std::fs::read(fixture("yaepblur-strong.expected.raw")).unwrap();
    for library in [false, true] {
        let request = DecodeTransform {
            yaepblur: Some("r=4:p=7:s=1024".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&base, request)
        } else {
            fvid::media::decode_video_transformed(&base, request)
        }
        .unwrap();
        assert_eq!(
            stats.backend,
            if library {
                "owned Matroska FFV1 decode"
            } else {
                "fvid"
            }
        );
        assert_eq!(stats.video_frames, 4);
        let out = base.with_file_name(format!(
            "fvid-yaep-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            yaepblur: Some("r=4:p=7:s=1024:enable='eq(n,0)*gte(t,.04)'".into()),
            interval: Some((40_000, 120_000)),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&base, &out, request, &CopyOptions::default())
        } else {
            fvid::media::transcode_lossless(&base, &out, request, &CopyOptions::default())
        }
        .unwrap();
        assert_eq!(stats.video_frames, 2);
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(&out).unwrap()),
            usize::MAX,
        )
        .unwrap();
        for n in 1..3 {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            let result = fvid::native_geometry::VideoGeometry::default()
                .apply(&raw, 16, 12)
                .unwrap();
            if n == 1 {
                assert_eq!(result.data, &expected[288..576])
            } else {
                assert_eq!(result.data, frame(2).data)
            }
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(out).unwrap();
    }
    std::fs::remove_file(base).unwrap();
    for source in [
        "../video.mp4",
        "../hevc/main10-ipb.mp4",
        "../vp9/adaptive.webm",
        "../av1/ramp.webm",
    ] {
        let request = DecodeTransform {
            yaepblur: Some("r=4:p=7:s=1024".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}
#[test]
fn wide_window_safe_acceptance_is_separate_from_legacy_overflow_reproduction() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("yaepblur-wide.safe.raw")).unwrap();
    let legacy = std::fs::read(fixture("yaepblur-wide.legacy.raw")).unwrap();
    assert_ne!(expected, legacy);
    let centre = (257 * 257 / 2) * 2;
    assert_eq!(
        u16::from_le_bytes([expected[centre], expected[centre + 1]]),
        65532
    );
    assert_eq!(
        u16::from_le_bytes([legacy[centre], legacy[centre + 1]]),
        22072
    );
    let source = fixture("yaepblur-wide.y4m");
    for library in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "fvid-yaep-wide-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            yaepblur: Some("r=2147483647:s=2147483647".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&source, &path, request, &CopyOptions::default())
        } else {
            fvid::media::transcode_lossless(&source, &path, request, &CopyOptions::default())
        }
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 1);
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(&path).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let raw = reader.read_frame_raw().unwrap().unwrap();
        let f = fvid::native_geometry::VideoGeometry::default()
            .apply(&raw, 257, 257)
            .unwrap();
        assert_eq!(f.data, expected);
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn pixelize_then_yaepblur_matches_saved_independent_chain_pixels() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("yaepblur-chain.expected.raw")).unwrap();
    for library in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "fvid-yaep-chain-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            pixelize: Some("4:3:avg:7".into()),
            yaepblur: Some("r=4:p=7:s=1024".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(
                &fixture("yaepblur.y4m"),
                &path,
                request,
                &CopyOptions::default(),
            )
        } else {
            fvid::media::transcode_lossless(
                &fixture("yaepblur.y4m"),
                &path,
                request,
                &CopyOptions::default(),
            )
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(&path).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut output = Vec::new();
        while let Some(raw) = reader.read_frame_raw().unwrap() {
            output.extend(
                fvid::native_geometry::VideoGeometry::default()
                    .apply(&raw, 16, 12)
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(output, expected);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn numeric_syntax_does_not_require_legacy_filter_fallback() {
    let expected = std::fs::read(fixture("yaepblur-strong.expected.raw")).unwrap();
    for args in [
        "r=0x4:p=0X7:s=1Ki",
        "r=4:p=7:s=128B",
        "r=4:p=7:s=1.024k",
        "r=4:p=7:s=1.024E3",
    ] {
        let filter = YaepBlur::parse(args).unwrap();
        let mut pixels = Vec::new();
        for n in 0..4 {
            let mut f = frame(n);
            filter
                .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                .unwrap();
            pixels.extend(f.data);
        }
        assert_eq!(pixels, expected, "{args}");
        for library in [false, true] {
            let output = std::env::temp_dir()
                .join(format!("fvid-number-{}-{library}.mkv", std::process::id()));
            let transform = fvid::media::LosslessTransform {
                yaepblur: Some(args.into()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("numeric-parameters.y4m"),
                    &output,
                    transform,
                    &Default::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("numeric-parameters.y4m"),
                    &output,
                    transform,
                    &Default::default(),
                )
            }
            .unwrap();
            assert_eq!(stats.backend, "fvid");
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let mut actual = Vec::new();
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                actual.extend(
                    fvid::native_geometry::VideoGeometry::default()
                        .apply(&raw, 16, 12)
                        .unwrap()
                        .data,
                );
            }
            assert_eq!(actual, expected, "{args} library={library}");
            std::fs::remove_file(output).unwrap();
        }
    }
}
