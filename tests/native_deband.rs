use fvid_media::{owned_deband::Deband, owned_frame::GeometryFrame};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn frame(n: usize, sub: [usize; 2], depth: u8) -> GeometryFrame {
    let mut data = Vec::new();
    for p in 0..3 {
        let (w, h) = if p == 0 {
            (17, 13)
        } else {
            (17usize.div_ceil(sub[0]), 13usize.div_ceil(sub[1]))
        };
        for y in 0..h {
            for x in 0..w {
                let v = ((((x * 7 + y * 11 + p * 53 + n * 13) % 192) + 32) as u16) << (depth - 8);
                if depth == 8 {
                    data.push(v as u8)
                } else {
                    data.extend(v.to_le_bytes())
                }
            }
        }
    }
    GeometryFrame {
        width: 17,
        height: 13,
        subsampling: Some(sub),
        data,
    }
}
#[test]
fn qualified_pixels_and_rewind() {
    for (name, args, sub, depth) in [
        ("default", "", [2, 2], 8),
        ("strong", "1thr=.5:2thr=.5:3thr=.5:4thr=.5", [2, 2], 8),
        ("no-blur", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:b=0", [2, 2], 8),
        ("coupled", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1", [1, 1], 8),
        ("depth", "1thr=.5:2thr=.5:3thr=.5:4thr=.5:c=1", [1, 1], 16),
    ] {
        let expected = std::fs::read(fixture(&format!("deband-{name}.expected.raw"))).unwrap();
        let program = Deband::parse(args).unwrap();
        for _ in 0..2 {
            let mut output = Vec::new();
            let mut input: Vec<u8> = Vec::new();
            for n in 0..4 {
                let mut f = frame(n, sub, depth);
                input.extend(&f.data);
                program
                    .apply(&mut f, depth, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                output.extend(f.data);
            }
            assert_eq!(output, expected, "{name}");
            assert_ne!(output, input, "fixture exercises filter");
        }
    }
}
#[test]
fn fixed_extreme_range_uses_clamped_source_endpoints() {
    let program = Deband::parse("r=-2147483648:d=0:1thr=.5:2thr=.5:3thr=.5").unwrap();
    for depth in [8, 16] {
        let mut f = frame(0, [1, 1], depth);
        let source = f.data.clone();
        program.apply(&mut f, depth, 0, None).unwrap();
        let read = |data: &[u8], i: usize| -> u32 {
            if depth == 8 {
                data[i] as u32
            } else {
                u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) as u32
            }
        };
        let threshold = (((1u32 << depth) - 1) as f32 * 0.5) as u32;
        for p in 0..3 {
            for y in 0..13 {
                let row = p * 221 + y * 17;
                let avg = (read(&source, row) + read(&source, row + 16)) / 2;
                for x in 0..17 {
                    let old = read(&source, row + x);
                    assert_eq!(
                        read(&f.data, row + x),
                        if old.abs_diff(avg) < threshold {
                            avg
                        } else {
                            old
                        }
                    );
                }
            }
        }
    }
}
#[test]
fn timeline_noop_validation_and_coupled_subsampling_refusal() {
    let mut f = frame(0, [2, 2], 8);
    let old = f.data.clone();
    Deband::parse("r=0")
        .unwrap()
        .apply(&mut f, 8, 0, None)
        .unwrap();
    assert_eq!(f.data, old);
    Deband::parse("1thr=.5:enable='eq(n,2)'")
        .unwrap()
        .apply(&mut f, 8, 0, None)
        .unwrap();
    assert_eq!(f.data, old);
    let error = Deband::parse("c=1")
        .unwrap()
        .apply(&mut f, 8, 0, None)
        .unwrap_err();
    assert!(error.to_string().contains("equal-sized"));
    assert_eq!(f.data, old);
    for args in [
        "1thr=0",
        "2thr=.6",
        "r=1.5",
        "r=2147483648",
        "d=7",
        "b=maybe",
        "unknown=1",
    ] {
        assert!(Deband::parse(args).is_err(), "{args}");
    }
    f.data.pop();
    let malformed = f.data.clone();
    assert!(
        Deband::parse("")
            .unwrap()
            .apply(&mut f, 8, 0, None)
            .is_err()
    );
    assert_eq!(f.data, malformed);
}
#[test]
fn root_and_library_exports_keep_source_clock_before_framestep() {
    use fvid::media::{CopyOptions, LosslessTransform};
    let expected = std::fs::read(fixture("deband-strong.expected.raw")).unwrap();
    for library in [false, true] {
        for step in [1usize, 2] {
            let output = std::env::temp_dir().join(format!(
                "fvid-deband-{}-{library}-{step}.mkv",
                std::process::id()
            ));
            let args = if step == 1 {
                "1thr=.5:2thr=.5:3thr=.5"
            } else {
                "1thr=.5:2thr=.5:3thr=.5:enable='eq(n,2)'"
            };
            let transform = LosslessTransform {
                deband: Some(args.into()),
                framestep: Some(step.to_string()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("deband.y4m"),
                    &output,
                    transform,
                    &CopyOptions::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("deband.y4m"),
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
                let actual = fvid::native_geometry::VideoGeometry::default()
                    .apply(&raw, 17, 13)
                    .unwrap();
                if step == 2 && n == 0 {
                    assert_eq!(actual.data, frame(n, [2, 2], 8).data)
                } else {
                    assert_eq!(actual.data, &expected[n * 347..(n + 1) * 347])
                }
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn coupled_full_resolution_and_high_depth_export_are_owned() {
    for (source, golden) in [
        ("deband-coupled.y4m", "deband-coupled.expected.raw"),
        ("deband-depth.y4m", "deband-depth.expected.raw"),
    ] {
        for library in [false, true] {
            let output = std::env::temp_dir().join(format!(
                "fvid-deband-{}-{source}-{library}.mkv",
                std::process::id()
            ));
            let transform = fvid::media::LosslessTransform {
                deband: Some("1thr=.5:2thr=.5:3thr=.5:c=1".into()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture(source),
                    &output,
                    transform,
                    &Default::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture(source),
                    &output,
                    transform,
                    &Default::default(),
                )
            }
            .unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.video_frames, 4);
            let request = fvid::media::DecodeTransform {
                deband: Some("1thr=.5:2thr=.5:3thr=.5:c=1".into()),
                ..Default::default()
            };
            let decoded = if library {
                fvid_media::decode_video_transformed(&output, request)
            } else {
                fvid::media::decode_video_transformed(&output, request)
            }
            .unwrap();
            assert_eq!(decoded.video_frames, 4);
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let mut pixels = Vec::new();
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                pixels.extend(
                    fvid::native_geometry::VideoGeometry::default()
                        .apply(&raw, 17, 13)
                        .unwrap()
                        .data,
                );
            }
            assert_eq!(pixels, std::fs::read(fixture(golden)).unwrap());
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn cli_and_native_codec_routes() {
    for source in [
        "deband.y4m",
        "../video.mp4",
        "../hevc/main10-ipb.mp4",
        "../vp9/adaptive.webm",
        "../av1/ramp.webm",
    ] {
        for library in [false, true] {
            let transform = fvid::media::DecodeTransform {
                deband: Some("1thr=.5:2thr=.5:3thr=.5".into()),
                ..Default::default()
            };
            if library && source != "deband.y4m" {
                // Existing standalone dispatch supports Y4M/FFV1, not these codecs yet.
                let error =
                    fvid_media::decode_video_transformed(&fixture(source), transform).unwrap_err();
                assert!(error.contains("does not yet support"), "{source}: {error}");
                continue;
            }
            let stats = if library {
                fvid_media::decode_video_transformed(&fixture(source), transform)
            } else {
                fvid::media::decode_video_transformed(&fixture(source), transform)
            }
            .unwrap_or_else(|e| panic!("{source}, library={library}: {e}"));
            assert!(stats.video_frames > 0, "{source}");
        }
    }
    let expected = std::fs::read(fixture("deband-strong.expected.raw")).unwrap();
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-deband-cli-{}-{command}.mkv",
            std::process::id()
        ));
        let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        process.args(["media", command]).arg(fixture("deband.y4m"));
        if command != "decode" {
            process.arg(&output);
        }
        let result = process
            .args(["--deband", "1thr=.5:2thr=.5:3thr=.5"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        if command != "decode" {
            let mut reader = fvid::playback_native::NativeReader::software(
                Cursor::new(std::fs::read(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let mut pixels = Vec::new();
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                pixels.extend(
                    fvid::native_geometry::VideoGeometry::default()
                        .apply(&raw, 17, 13)
                        .unwrap()
                        .data,
                );
            }
            assert_eq!(pixels, expected);
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn strict_threshold_and_coupled_decision_read_original_components() {
    // A fixed horizontal neighborhood on a three-pixel gray row averages endpoints.
    let mut row = vec![10, 20, 30];
    Deband::parse("r=-1:d=0:1thr=10/255")
        .unwrap()
        .apply_gray(&mut row, 3, 1, 8, 0, None)
        .unwrap();
    assert_eq!(row, [15, 20, 25]);
    let mut row = vec![0, 10, 40];
    Deband::parse("r=-1:d=0:1thr=10/255")
        .unwrap()
        .apply_gray(&mut row, 3, 1, 8, 0, None)
        .unwrap();
    assert_eq!(row, [5, 10, 40], "equality with threshold retains source");
    let mut rgba = vec![10, 10, 10, 0, 20, 20, 20, 255, 30, 30, 30, 0];
    let old = rgba.clone();
    Deband::parse("r=-1:d=0:c=1:1thr=.5:2thr=.5:3thr=.5:4thr=.00003")
        .unwrap()
        .apply_rgb(&mut rgba, 3, 1, 8, 4, 0, None)
        .unwrap();
    assert_eq!(rgba, old, "alpha refusal retains all coupled channels");
}
