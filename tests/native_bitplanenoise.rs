use fvid_media::{owned_bitplanenoise::BitPlaneNoise, owned_frame::GeometryFrame};
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
fn qualified_pixels_metadata_analysis_and_rewind() {
    let expected = std::fs::read(fixture("bitplanenoise.expected.raw")).unwrap();
    let metadata = std::fs::read_to_string(fixture("bitplanenoise.expected.txt")).unwrap();
    for _ in 0..2 {
        let filter = BitPlaneNoise::parse("bitplane=1:filter=1").unwrap();
        let analysis = BitPlaneNoise::parse("").unwrap();
        let mut result = Vec::new();
        let mut reports = Vec::new();
        for n in 0..4 {
            let mut f = frame(n);
            let mut original = frame(n);
            let a = analysis
                .apply(&mut original, 8, n as u64, Some(n as f64 / 25.))
                .unwrap()
                .unwrap();
            assert_eq!(original.data, f.data);
            let b = filter
                .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                .unwrap()
                .unwrap();
            assert_eq!(a.metadata(), b.metadata());
            assert_eq!(filter.last_report().unwrap().metadata(), b.metadata());
            reports.extend(b.metadata().into_iter().map(|(k, v)| format!("{k}={v}")));
            result.extend(f.data);
        }
        assert_eq!(result, expected);
        assert_eq!(reports.join("\n") + "\n", metadata);
    }
}
#[test]
fn boundaries_depth_and_transactional_refusal() {
    let filter = BitPlaneNoise::parse("16:1").unwrap();
    for depth in [8, 9, 10, 12, 14, 16] {
        let mut p = if depth == 8 {
            vec![7]
        } else {
            7u16.to_le_bytes().to_vec()
        };
        let r = filter
            .apply_plane(&mut p, 1, 1, depth, 0, 0, None)
            .unwrap()
            .unwrap();
        assert_eq!(r.matching, 1);
        assert_eq!(r.score, 0.);
        assert_eq!(r.precise_score(), 0.);
        let maximum = ((1u32 << depth) - 1) as u16;
        assert_eq!(
            p,
            if depth == 8 {
                vec![255]
            } else {
                maximum.to_le_bytes().to_vec()
            }
        );
    }
    let mut invalid = vec![255, 255];
    let saved = invalid.clone();
    assert!(
        filter
            .apply_plane(&mut invalid, 1, 1, 10, 0, 0, None)
            .is_err()
    );
    assert_eq!(invalid, saved);
    for args in ["0", "17", "1.5", "1:2", "unknown=1", "1:1:1"] {
        assert!(BitPlaneNoise::parse(args).is_err(), "{args}");
    }
    let disabled = BitPlaneNoise::parse("1:1:enable='eq(n,2)'").unwrap();
    let mut f = frame(0);
    assert!(disabled.apply(&mut f, 8, 0, None).unwrap().is_none());
    assert_eq!(f.data, frame(0).data);
    assert!(disabled.last_report().is_none());
    let mut rgb = GeometryFrame {
        width: 2,
        height: 1,
        subsampling: None,
        data: vec![0, 1, 0, 1, 1, 0],
    };
    BitPlaneNoise::parse("1:1")
        .unwrap()
        .apply(&mut rgb, 8, 0, None)
        .unwrap();
    assert_eq!(rgb.data, vec![0, 255, 255, 0, 255, 255]);
}
#[test]
fn root_and_library_exports_preserve_visualization_and_timeline() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("bitplanenoise.expected.raw")).unwrap();
    for library in [false, true] {
        for step in [1usize, 2] {
            let output = std::env::temp_dir().join(format!(
                "fvid-bitplane-{}-{library}-{step}.mkv",
                std::process::id()
            ));
            let transform = LosslessTransform {
                bitplanenoise: Some(
                    if step == 1 {
                        "1:1"
                    } else {
                        "1:1:enable='eq(n,2)'"
                    }
                    .into(),
                ),
                framestep: Some(step.to_string()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("bitplanenoise.y4m"),
                    &output,
                    transform,
                    &CopyOptions::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("bitplanenoise.y4m"),
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
#[test]
fn cli_decode_export_and_lossless_use_owned_filter() {
    let source = fixture("bitplanenoise.y4m");
    let expected = std::fs::read(fixture("bitplanenoise.expected.raw")).unwrap();
    let y4m = std::env::temp_dir().join(format!("fvid-bitplane-cli-{}.y4m", std::process::id()));
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
        process.args(["--bitplanenoise", "1:1"]);
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
    let base =
        std::env::temp_dir().join(format!("fvid-bitplane-source-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &fixture("bitplanenoise.y4m"),
        &base,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    let expected = std::fs::read(fixture("bitplanenoise.expected.raw")).unwrap();
    for library in [false, true] {
        let request = DecodeTransform {
            bitplanenoise: Some("1:1".into()),
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
            "fvid-bitplane-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            bitplanenoise: Some("1:1:enable='eq(n,0)*gte(t,.04)'".into()),
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
            bitplanenoise: Some("1:1".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}
