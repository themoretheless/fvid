use fvid_media::{owned_frame::GeometryFrame, owned_perspective::Perspective};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
const LINEAR: &str = "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2";
const CUBIC: &str = "x0=1:y0=2:x1=W-2:y1=1:x2=2:y2=H-1:x3=W-1:y3=H-2:interpolation=cubic";
const DESTINATION: &str = "x0=2:y0=1:x1=W-1:y1=2:x2=1:y2=H-2:x3=W-2:y3=H-1:sense=destination";
const FRAME: &str = "x0=in/3:y0=on/4:eval=frame";
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn frame(n: usize, depth: u8) -> GeometryFrame {
    let sub = if depth == 8 { [2, 2] } else { [1, 1] };
    let mut data = Vec::new();
    for p in 0..3 {
        let (w, h) = if p == 0 {
            (17, 13)
        } else {
            (17usize.div_ceil(sub[0]), 13usize.div_ceil(sub[1]))
        };
        for y in 0..h {
            for x in 0..w {
                let value =
                    ((((x * 7 + y * 11 + p * 53 + n * 13) % 192) + 32) as u16) << (depth - 8);
                if depth == 8 {
                    data.push(value as u8)
                } else {
                    data.extend(value.to_le_bytes());
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
fn qualified_projection_pixels_rewind_and_init() {
    for (name, args) in [
        ("linear", LINEAR),
        ("cubic", CUBIC),
        ("destination", DESTINATION),
        ("frame", FRAME),
    ] {
        let program = Perspective::parse(args).unwrap();
        let expected = std::fs::read(fixture(&format!("perspective-{name}.expected.raw"))).unwrap();
        for _ in 0..2 {
            let mut output = Vec::new();
            for n in 0..4 {
                let mut f = frame(n, 8);
                program
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                output.extend(f.data);
            }
            assert_eq!(output, expected, "{name}");
        }
    }
    let init = Perspective::parse("x0=in/3:y0=on/4:eval=init").unwrap();
    let fixed = Perspective::parse("x0=1/3:y0=1/4").unwrap();
    for n in 0..4 {
        let mut a = frame(n, 8);
        let mut b = frame(n, 8);
        init.apply(&mut a, 8, n as u64, None).unwrap();
        fixed.apply(&mut b, 8, n as u64, None).unwrap();
        assert_eq!(a.data, b.data);
    }
}
#[test]
fn degeneracy_nonfinite_overflow_and_storage_refusals_are_transactional() {
    for (args, reason) in [
        ("x0=0:y0=0:x1=0:y1=0:x2=0:y2=0:x3=0:y3=0", "singular"),
        ("x0=1/0", "nonfinite"),
        ("x0=1e12:x1=W+1e12:x2=1e12:x3=W+1e12", "coordinate"),
    ] {
        let mut f = frame(0, 8);
        let original = f.data.clone();
        let error = Perspective::parse(args)
            .unwrap()
            .apply(&mut f, 8, 0, None)
            .unwrap_err();
        assert!(error.contains(reason), "{args}: {error}");
        assert_eq!(f.data, original);
    }
    for args in [
        "x0=unknown",
        "interpolation=bad",
        "sense=bad",
        "eval=bad",
        "unknown=1",
    ] {
        assert!(Perspective::parse(args).is_err(), "{args}");
    }
    let mut f = frame(0, 8);
    f.data.pop();
    let old = f.data.clone();
    assert!(
        Perspective::parse(LINEAR)
            .unwrap()
            .apply(&mut f, 8, 0, None)
            .is_err()
    );
    assert_eq!(old, f.data);
    let mut f = frame(0, 8);
    let old = f.data.clone();
    Perspective::parse(&format!("{LINEAR}:enable='eq(n,2)'"))
        .unwrap()
        .apply(&mut f, 8, 0, None)
        .unwrap();
    assert_eq!(f.data, old);
}
#[test]
fn full_precision_translation_has_independent_half_sample_expectation() {
    for depth in [8, 9, 10, 12, 14, 16] {
        let source = [0u16, ((1u32 << depth) - 1) as u16, 0];
        let mut data: Vec<u8> = source
            .iter()
            .flat_map(|v| {
                if depth == 8 {
                    vec![*v as u8]
                } else {
                    v.to_le_bytes().to_vec()
                }
            })
            .collect();
        Perspective::parse("x0=.5:x1=W+.5:x2=.5:x3=W+.5")
            .unwrap()
            .apply_gray(&mut data, 3, 1, depth, 0, None)
            .unwrap();
        let middle = (1u32 << (depth - 1)) as u16;
        let expected: Vec<u8> = [middle, middle, 0]
            .iter()
            .flat_map(|v| {
                if depth == 8 {
                    vec![*v as u8]
                } else {
                    v.to_le_bytes().to_vec()
                }
            })
            .collect();
        assert_eq!(data, expected, "depth{depth}");
        let mut f = frame(0, if depth == 8 { 8 } else { 16 });
        let old = f.data.clone();
        Perspective::parse("")
            .unwrap()
            .apply(&mut f, if depth == 8 { 8 } else { 16 }, 0, None)
            .unwrap();
        assert_eq!(f.data, old);
    }
}
#[test]
fn root_and_library_ffv1_exports_keep_projection_and_source_clock() {
    use fvid::media::{DecodeTransform, LosslessTransform};
    let expected = std::fs::read(fixture("perspective-linear.expected.raw")).unwrap();
    for library in [false, true] {
        for step in [1usize, 2] {
            let output = std::env::temp_dir().join(format!(
                "fvid-perspective-{}-{library}-{step}.mkv",
                std::process::id()
            ));
            let args = if step == 1 {
                LINEAR.to_owned()
            } else {
                format!("{LINEAR}:enable='eq(n,2)'")
            };
            let transform = LosslessTransform {
                perspective: Some(args),
                framestep: Some(step.to_string()),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("perspective.y4m"),
                    &output,
                    transform,
                    &Default::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("perspective.y4m"),
                    &output,
                    transform,
                    &Default::default(),
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
                    assert_eq!(actual.data, frame(0, 8).data)
                } else {
                    assert_eq!(actual.data, &expected[n * 347..(n + 1) * 347]);
                }
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            let request = DecodeTransform {
                perspective: Some(CUBIC.into()),
                ..Default::default()
            };
            let decoded = if library {
                fvid_media::decode_video_transformed(&output, request)
            } else {
                fvid::media::decode_video_transformed(&output, request)
            }
            .unwrap();
            assert_eq!(decoded.video_frames, 4 / step as u64);
            std::fs::remove_file(output).unwrap();
        }
    }
}
#[test]
fn cli_export_and_native_codec_routes_accept_owned_projection() {
    for source in [
        "../video.mp4",
        "../hevc/main10-ipb.mp4",
        "../vp9/adaptive.webm",
        "../av1/ramp.webm",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(source),
            fvid::media::DecodeTransform {
                perspective: Some(LINEAR.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let expected = std::fs::read(fixture("perspective-cubic.expected.raw")).unwrap();
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-perspective-cli-{}-{command}.mkv",
            std::process::id()
        ));
        let mut process = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        process
            .args(["media", command])
            .arg(fixture("perspective.y4m"));
        if command != "decode" {
            process.arg(&output);
        }
        let result = process.args(["--perspective", CUBIC]).output().unwrap();
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
fn high_depth_export_uses_full_precision_half_sample_model() {
    let mut expected = Vec::new();
    for n in 0..4 {
        let source = frame(n, 16).data;
        for p in 0..3 {
            for y in 0..13 {
                for x in 0..17 {
                    let read = |x: usize| {
                        let at = 2 * (p * 221 + y * 17 + x);
                        u16::from_le_bytes([source[at], source[at + 1]]) as u32
                    };
                    let average = (read(x) + read((x + 1).min(16)) + 1) / 2;
                    expected.extend((average as u16).to_le_bytes());
                }
            }
        }
    }
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-perspective-depth-{}-{library}.mkv",
            std::process::id()
        ));
        let request = fvid::media::LosslessTransform {
            perspective: Some("x0=.5:x1=W+.5:x2=.5:x3=W+.5".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(
                &fixture("perspective-depth.y4m"),
                &output,
                request,
                &Default::default(),
            )
        } else {
            fvid::media::transcode_lossless(
                &fixture("perspective-depth.y4m"),
                &output,
                request,
                &Default::default(),
            )
        }
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 4);
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
