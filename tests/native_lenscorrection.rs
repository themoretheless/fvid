use fvid_media::{
    owned_frame::GeometryFrame,
    owned_lenscorrection::{Component, LensCorrection},
};
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
fn independent_warp_bilinear_and_identity_pixels_survive_rewind() {
    for (args, name) in [
        ("", "identity"),
        ("k1=.3:k2=.1", "warp"),
        ("k1=-.3:k2=.1:i=bilinear:fc=0xff0050@.4", "bilinear"),
    ] {
        let expected =
            std::fs::read(fixture(&format!("lenscorrection-{name}.expected.raw"))).unwrap();
        let program = LensCorrection::parse(args).unwrap();
        for _ in 0..2 {
            let mut pixels = Vec::new();
            for n in 0..4 {
                let mut f = frame(n);
                program
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                pixels.extend(f.data);
            }
            assert_eq!(pixels, expected);
        }
    }
    let mut a = frame(0);
    let mut b = frame(0);
    LensCorrection::parse(".5:.5:.3:.1:0:black@0")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    LensCorrection::parse("k1=.3:k2=.1")
        .unwrap()
        .apply(&mut b, 8, 0, None)
        .unwrap();
    assert_eq!(a.data, b.data);
}
#[test]
fn small_high_depth_alpha_rgb_and_malformed_inputs_are_explicit() {
    let program = LensCorrection::parse("i=bilinear").unwrap();
    let mut plane = vec![0, 40, 80, 120];
    program
        .apply_plane(&mut plane, 2, 2, 8, Component::Luma, 0, None)
        .unwrap();
    assert_eq!(plane, [60, 80, 100, 120]);
    let outside = LensCorrection::parse("cx=1:cy=1:k1=1:k2=1:fc=white@.5").unwrap();
    for depth in [8u8, 9, 10, 12, 14, 16] {
        let mut sample = if depth == 8 {
            vec![1]
        } else {
            1u16.to_le_bytes().to_vec()
        };
        outside
            .apply_plane(&mut sample, 1, 1, depth, Component::Alpha, 0, None)
            .unwrap();
        let value = 127u16 << (depth - 8);
        assert_eq!(
            sample,
            if depth == 8 {
                vec![127]
            } else {
                value.to_le_bytes().to_vec()
            }
        );
    }
    let mut rgb = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: None,
        data: vec![1, 2, 3],
    };
    LensCorrection::parse("cx=1:cy=1:k1=1:k2=1:fc=AliceBlue")
        .unwrap()
        .apply(&mut rgb, 8, 0, None)
        .unwrap();
    assert_eq!(rgb.data, [240, 248, 255]);
    for args in [
        "cx=-.1",
        "cy=1.1",
        "k1=2",
        "k2=NaN",
        "i=65",
        "i=1.5",
        "fc=unknown",
        "fc=red@2",
        "unknown=1",
    ] {
        assert!(LensCorrection::parse(args).is_err(), "{args}");
    }
    let mut invalid = vec![255, 255];
    let original = invalid.clone();
    assert!(
        outside
            .apply_plane(&mut invalid, 1, 1, 10, Component::Luma, 0, None)
            .is_err()
    );
    assert_eq!(invalid, original);
    let mut disabled = frame(0);
    LensCorrection::parse("k1=1:enable='eq(n,2)'")
        .unwrap()
        .apply(&mut disabled, 8, 0, None)
        .unwrap();
    assert_eq!(disabled.data, frame(0).data);
    for library in [false, true] {
        let request = fvid::media::DecodeTransform {
            lenscorrection: Some("k1=.3:k2=.1".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&fixture("lenscorrection-small.y4m"), request)
        } else {
            fvid::media::decode_video_transformed(&fixture("lenscorrection-small.y4m"), request)
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
    }
}
#[test]
fn root_and_library_exports_preserve_visualization_and_timeline() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("lenscorrection-warp.expected.raw")).unwrap();
    for (source, full) in [
        (fixture("lenscorrection.y4m"), false),
        (fixture("lenscorrection-full.y4m"), true),
    ] {
        for library in [false, true] {
            for step in [1usize, 2] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-lens-{}-{library}-{step}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    lenscorrection: Some(
                        if step == 1 {
                            "k1=.3:k2=.1"
                        } else {
                            "k1=.3:k2=.1:enable='eq(n,2)'"
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
    let source = fixture("lenscorrection.y4m");
    let expected = std::fs::read(fixture("lenscorrection-warp.expected.raw")).unwrap();
    let y4m = std::env::temp_dir().join(format!("fvid-lens-cli-{}.y4m", std::process::id()));
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
        process.args(["--lenscorrection", "k1=.3:k2=.1"]);
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
    let base = std::env::temp_dir().join(format!("fvid-lens-source-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &fixture("lenscorrection.y4m"),
        &base,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    let expected = std::fs::read(fixture("lenscorrection-warp.expected.raw")).unwrap();
    for library in [false, true] {
        let request = DecodeTransform {
            lenscorrection: Some("k1=.3:k2=.1".into()),
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
            "fvid-lens-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let request = LosslessTransform {
            lenscorrection: Some("k1=.3:k2=.1:enable='eq(n,0)*gte(t,.04)'".into()),
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
            lenscorrection: Some("k1=.3:k2=.1".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}
