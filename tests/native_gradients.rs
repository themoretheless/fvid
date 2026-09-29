use fvid::{
    native_geometry::GeometryFrame,
    native_pixels::{Gradient, GradientKind},
};
const KINDS: [GradientKind; 5] = [
    GradientKind::Sobel,
    GradientKind::Prewitt,
    GradientKind::Roberts,
    GradientKind::Kirsch,
    GradientKind::Scharr,
];
fn frame(depth: u8) -> GeometryFrame {
    let max = (1u32 << depth) - 1;
    let data = (0..192)
        .flat_map(|i| {
            let v = ((i * i * 31 + i * 17) % (max + 1)) as u16;
            if depth == 8 {
                vec![v as u8]
            } else {
                v.to_le_bytes().to_vec()
            }
        })
        .collect();
    GeometryFrame {
        width: 8,
        height: 8,
        subsampling: Some([1, 1]),
        data,
    }
}
#[test]
fn masks_constants_and_invalid_input() {
    for kind in KINDS {
        for depth in [8, 10, 16] {
            let mut f = frame(depth);
            let original = f.data.clone();
            Gradient::parse(kind, "planes=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Gradient::parse(kind, "planes=1:scale=0:delta=42")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            let bytes = if depth == 8 { 1 } else { 2 };
            assert_eq!(&f.data[64 * bytes..], &original[64 * bytes..]);
            for sample in f.data[..64 * bytes].chunks_exact(bytes) {
                assert_eq!(sample[0], 42);
                if bytes == 2 {
                    assert_eq!(sample[1], 0);
                }
            }
            f.data.pop();
            let before = f.data.clone();
            assert!(
                Gradient::parse(kind, "")
                    .unwrap()
                    .apply(&mut f, depth)
                    .is_err()
            );
            assert_eq!(f.data, before);
        }
        for args in [
            "planes=16",
            "scale=NaN",
            "scale=-1",
            "delta=inf",
            "unknown=1",
            "1:1:0:1",
        ] {
            assert!(Gradient::parse(kind, args).is_err());
        }
        let mut flat = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![20, 30, 40],
        };
        Gradient::parse(kind, "")
            .unwrap()
            .apply(&mut flat, 8)
            .unwrap();
        assert_eq!(flat.data, [0, 0, 0]);
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG; reference-only executable"]
fn pixels_match_reference() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let executable = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy) in [
        (8, "yuv444p", 1, 1),
        (10, "yuv444p10le", 1, 1),
        (16, "yuv444p16le", 1, 1),
        (8, "yuv422p", 2, 1),
        (10, "yuv422p10le", 2, 1),
        (16, "yuv422p16le", 2, 1),
        (8, "yuv420p", 2, 2),
        (10, "yuv420p10le", 2, 2),
        (16, "yuv420p16le", 2, 2),
    ] {
        for kind in KINDS {
            for args in ["", "planes=1:scale=0.125:delta=3"] {
                let mut f = frame(depth);
                f.subsampling = Some([sx, sy]);
                f.data
                    .truncate((64 + 128 / (sx * sy)) * if depth == 8 { 1 } else { 2 });
                let data = f.data.clone();
                Gradient::parse(kind, args)
                    .unwrap()
                    .apply(&mut f, depth)
                    .unwrap();
                let filter = if args.is_empty() {
                    kind.name().to_owned()
                } else {
                    format!("{}={args}", kind.name())
                };
                let mut p = Command::new(&executable)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "8x8",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &filter,
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        format,
                        "pipe:1",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                p.stdin.take().unwrap().write_all(&data).unwrap();
                let out = p.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                if let Some(index) = f.data.iter().zip(&out.stdout).position(|(a, b)| a != b) {
                    panic!(
                        "{format} {filter} byte {index}: own={} reference={}",
                        f.data[index], out.stdout[index]
                    );
                }
                assert_eq!(f.data.len(), out.stdout.len());
            }
        }
    }
}

#[test]
fn request_and_cli_use_owned_gradient_pipeline() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    for kind in KINDS {
        let mut request = fvid::media_info::DecodeTransform::default();
        let option = match kind {
            GradientKind::Sobel => &mut request.sobel,
            GradientKind::Prewitt => &mut request.prewitt,
            GradientKind::Roberts => &mut request.roberts,
            GradientKind::Kirsch => &mut request.kirsch,
            GradientKind::Scharr => &mut request.scharr,
        };
        *option = Some("planes=1:scale=0.125:delta=3".into());
        request.interval = Some((0, 120_000));
        request.horizontal_flip = true;
        let stats = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 1);
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args([
                "media",
                "decode",
                path.to_str().unwrap(),
                "--from",
                "0",
                "--to",
                "0.12",
                "--hflip",
                &format!("--{}", kind.name()),
                "planes=1:scale=0.125:delta=3",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(stats["video_frames"], 1);
        #[cfg(feature = "media")]
        assert_eq!(
            fvid::media::decode_video_transformed(&path, request)
                .unwrap()
                .backend,
            "fvid"
        );
    }
}

#[test]
fn request_chain_has_canonical_order_and_cli_rejects_duplicates() {
    use fvid::native_pixels::PixelFilters;
    let request = fvid::media_info::DecodeTransform {
        negate: Some("1".into()),
        scharr: Some("1:0.125:3".into()),
        sobel: Some("1:0.25:0".into()),
        ..Default::default()
    };
    let filters = PixelFilters::from_request(&request).unwrap();
    assert_eq!(
        filters
            .gradients
            .iter()
            .map(|g| g.kind())
            .collect::<Vec<_>>(),
        [GradientKind::Sobel, GradientKind::Scharr]
    );
    let mut actual = frame(8);
    let mut expected = frame(8);
    fvid::native_pixels::Negate.apply(&mut expected, 8).unwrap();
    Gradient::parse(GradientKind::Sobel, "1:0.25:0")
        .unwrap()
        .apply(&mut expected, 8)
        .unwrap();
    Gradient::parse(GradientKind::Scharr, "1:0.125:3")
        .unwrap()
        .apply(&mut expected, 8)
        .unwrap();
    filters.apply(&mut actual, 8).unwrap();
    assert_eq!(actual.data, expected.data);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            "does-not-exist.mp4",
            "--sobel",
            "",
            "--sobel",
            "",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate sobel"));
}

#[test]
fn hevc_main10_pipeline_retains_depth() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main10-ipb.mp4");
    let reference = fvid::native_media::decode_video(&path).unwrap();
    let request = fvid::media_info::DecodeTransform {
        sobel: Some("1:0.125:0".into()),
        scharr: Some("1:0.125:0".into()),
        ..Default::default()
    };
    let actual = fvid::native_media::decode_video_request(&path, &request).unwrap();
    assert_eq!(actual.backend, "fvid");
    assert_eq!(actual.video_frames, reference.video_frames);
    assert_eq!(actual.pixel_format, "yuv420p10le");
    assert_eq!(
        (actual.width, actual.height),
        (reference.width, reference.height)
    );
}

#[cfg(feature = "media")]
#[test]
fn unmigrated_expression_options_keep_existing_media_support() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::DecodeTransform {
        sobel: Some("scale=PI".into()),
        ..Default::default()
    };
    assert!(fvid::native_media::decode_video_request(&path, &request).is_err());
    let actual = fvid::media::decode_video_transformed(&path, request).unwrap();
    assert!(actual.video_frames > 0);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            path.to_str().unwrap(),
            "--sobel",
            "scale=PI",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
