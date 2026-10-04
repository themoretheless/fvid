use fvid_media::{owned_frame::GeometryFrame, owned_gradfun::GradFun};
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
        width: 96,
        height: 80,
        subsampling: Some([2, 2]),
        data: (0..11520)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect(),
    }
}
#[test]
fn saved_default_strong_and_edge_outputs_accept_rewind() {
    for (args, name, w, h, sampling, size) in [
        ("", "default", 96, 80, [2, 2], 11520),
        ("strength=64:radius=4", "strong", 96, 80, [2, 2], 11520),
        ("strength=2:radius=32", "edge", 65, 65, [1, 1], 12675),
        ("0.51:4", "overflow-safe", 9, 9, [1, 1], 243),
    ] {
        let expected = std::fs::read(fixture(&format!("gradfun-{name}.expected.raw"))).unwrap();
        let program = GradFun::parse(args).unwrap();
        for _ in 0..2 {
            let mut result = Vec::new();
            for n in 0..4 {
                let mut f = GeometryFrame {
                    width: w,
                    height: h,
                    subsampling: Some(sampling),
                    data: (0..size)
                        .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                        .collect(),
                };
                program
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                result.extend(f.data);
            }
            assert_eq!(result, expected, "{name}");
        }
    }
    let clocked = GradFun::parse("0.51:4:enable='eq(n,2)'").unwrap();
    let edge = || GeometryFrame {
        width: 9,
        height: 9,
        subsampling: Some([1, 1]),
        data: (0..243)
            .map(|i| ((i * 37 + 2 * 23 + 101) % 256) as u8)
            .collect(),
    };
    let mut first = edge();
    clocked.apply(&mut first, 8, 2, Some(0.08)).unwrap();
    let mut disabled = edge();
    clocked.apply(&mut disabled, 8, 0, Some(0.)).unwrap();
    assert_eq!(disabled.data, edge().data);
    let mut rewind = edge();
    clocked.apply(&mut rewind, 8, 2, Some(0.08)).unwrap();
    assert_eq!(
        rewind.data, first.data,
        "disabled rewind clears the corner-case history"
    );
    let safe = std::fs::read(fixture("gradfun-overflow-safe.expected.raw")).unwrap();
    let legacy = std::fs::read(fixture("gradfun-overflow-legacy.expected.raw")).unwrap();
    assert_ne!(safe, legacy);
    assert_eq!(safe[85], 174);
    assert_eq!(legacy[85], 255);
}
#[test]
fn high_depth_small_frames_aliases_and_refusals_are_explicit() {
    for args in [
        "strength=.5",
        "strength=65",
        "radius=3",
        "radius=33",
        "radius=4.5",
        "unknown=1",
        "1:4:0",
    ] {
        assert!(GradFun::parse(args).is_err(), "{args}");
    }
    let mut malformed = GeometryFrame {
        width: 4,
        height: 4,
        subsampling: Some([2, 2]),
        data: vec![1, 2],
    };
    let old = malformed.data.clone();
    assert!(
        GradFun::parse("")
            .unwrap()
            .apply(&mut malformed, 8, 0, None)
            .is_err()
    );
    assert_eq!(old, malformed.data);
    for depth in [8, 9, 10, 12, 14, 16] {
        let maximum = (1u32 << depth) - 1;
        let value = maximum / 2;
        let mut plane = if depth == 8 {
            vec![value as u8; 96 * 80]
        } else {
            (0..96 * 80)
                .flat_map(|_| (value as u16).to_le_bytes())
                .collect()
        };
        let original = plane.clone();
        GradFun::parse("64:4")
            .unwrap()
            .apply_plane(&mut plane, 96, 80, depth, [1, 1], 0, None)
            .unwrap();
        assert_eq!(plane, original, "flat depth{depth}");
    }
    let mut a = frame(0);
    let mut b = frame(0);
    GradFun::parse("5:7")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    GradFun::parse("strength=5:radius=8")
        .unwrap()
        .apply(&mut b, 8, 0, None)
        .unwrap();
    assert_eq!(a.data, b.data);
    let disabled = GradFun::parse("64:4:enable='eq(n,2)'").unwrap();
    let mut f = frame(0);
    disabled.apply(&mut f, 8, 0, None).unwrap();
    assert_eq!(f.data, frame(0).data);
    for library in [false, true] {
        let transform = fvid::media::DecodeTransform {
            gradfun: Some("64:4".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&fixture("gradfun-small.y4m"), transform)
        } else {
            fvid::media::decode_video_transformed(&fixture("gradfun-small.y4m"), transform)
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
    }
}
#[test]
fn root_and_library_exports_preserve_visualization_and_timeline() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("gradfun-strong.expected.raw")).unwrap();
    for library in [false, true] {
        for step in [1usize, 2] {
            let output = std::env::temp_dir().join(format!(
                "fvid-gradfun-{}-{library}-{step}.mkv",
                std::process::id()
            ));
            let transform = LosslessTransform {
                gradfun: Some(
                    if step == 1 {
                        "strength=64:radius=4"
                    } else {
                        "strength=64:radius=4:enable='eq(n,2)'"
                    }
                    .into(),
                ),
                framestep: Some(step.to_string()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("gradfun.y4m"),
                    &output,
                    transform,
                    &CopyOptions::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("gradfun.y4m"),
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
                    .apply(&raw, 96, 80)
                    .unwrap();
                if step == 2 && n == 0 {
                    assert_eq!(result.data, frame(0).data)
                } else {
                    assert_eq!(result.data, &expected[n * 11520..(n + 1) * 11520]);
                }
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn cli_decode_export_and_lossless_use_owned_filter() {
    let source = fixture("gradfun.y4m");
    let expected = std::fs::read(fixture("gradfun-strong.expected.raw")).unwrap();
    let y4m = std::env::temp_dir().join(format!("fvid-gradfun-cli-{}.y4m", std::process::id()));
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
        process.args(["--gradfun", "strength=64:radius=4"]);
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
                    .apply(&f, 96, 80)
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
    let base = std::env::temp_dir().join(format!("fvid-gradfun-source-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &fixture("gradfun.y4m"),
        &base,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    let expected = std::fs::read(fixture("gradfun-strong.expected.raw")).unwrap();
    for library in [false, true] {
        let request = DecodeTransform {
            gradfun: Some("strength=64:radius=4".into()),
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
            "fvid-gradfun-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            gradfun: Some("strength=64:radius=4:enable='eq(n,0)*gte(t,.04)'".into()),
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
                .apply(&raw, 96, 80)
                .unwrap();
            if n == 1 {
                assert_eq!(result.data, &expected[11520..23040])
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
            gradfun: Some("strength=64:radius=4".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}
