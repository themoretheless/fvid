use fvid::{native_geometry::GeometryFrame, native_pixelize::Pixelize};
#[test]
fn reductions_edges_and_validation() {
    for (mode, expected) in [
        ("avg", [5, 5, 20]),
        ("min", [0, 0, 20]),
        ("max", [10, 10, 20]),
    ] {
        let mut frame = GeometryFrame {
            width: 3,
            height: 1,
            subsampling: Some([1, 1]),
            data: [vec![0, 10, 20], vec![30; 6]].concat(),
        };
        Pixelize::parse(&format!("2:1:{mode}:1"))
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        assert_eq!(&frame.data[..3], &expected);
        assert_eq!(&frame.data[3..], &[30; 6]);
    }
    for args in ["0", "1025", "1:1:3", "p=16", "foo=1"] {
        assert!(Pixelize::parse(args).is_err());
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn reference_pixels_across_layouts_and_depths() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    for (layout, sub) in [
        ("420", [2, 2]),
        ("422", [2, 1]),
        ("444", [1, 1]),
        ("440", [1, 2]),
        ("411", [4, 1]),
        ("410", [4, 4]),
    ] {
        for depth in [8, 10, 16] {
            if (depth > 8 && matches!(layout, "411" | "410")) || (layout == "440" && depth == 16) {
                continue;
            }
            let n = 17 * 13 + 2 * 17usize.div_ceil(sub[0]) * 13usize.div_ceil(sub[1]);
            let input: Vec<u8> = (0..n)
                .flat_map(|i| {
                    let v = ((i * 173 + i * i * 11) % (1usize << depth)) as u16;
                    if depth == 8 {
                        vec![v as u8]
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let format = if depth == 8 {
                format!("yuv{layout}p")
            } else {
                format!("yuv{layout}p{depth}le")
            };
            for args in [
                "",
                "3:5:avg:7",
                "w=5:h=3:m=min:p=5",
                "1:1:max:1",
                "1024:1024:2:7",
            ] {
                let mut frame = GeometryFrame {
                    width: 17,
                    height: 13,
                    subsampling: Some(sub),
                    data: input.clone(),
                };
                Pixelize::parse(args)
                    .unwrap()
                    .apply(&mut frame, depth)
                    .unwrap();
                let mut child = Command::new(&binary)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        &format,
                        "-video_size",
                        "17x13",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("pixelize={args}"),
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&input).unwrap();
                let result = child.wait_with_output().unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(frame.data, result.stdout, "{format} {args}");
            }
        }
    }
}

#[cfg(feature = "media")]
#[test]
fn decoder_api_and_cli_use_owned_pixelize() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in [
        "video.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/odd10.webm",
        "av1/random-access.webm",
    ] {
        let source = root.join(name);
        let request = fvid::media::DecodeTransform {
            pixelize: Some("3:5:min:7".into()),
            ..Default::default()
        };
        let expected = fvid::native_media::decode_video_request(&source, &request).unwrap();
        let actual = fvid::media::decode_video_transformed(&source, request).unwrap();
        assert_eq!(actual.backend, "fvid");
        assert_eq!(actual.video_frames, expected.video_frames);
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode"])
            .arg(source)
            .args(["--pixelize", "3:5:min:7"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(stats["video_frames"], expected.video_frames);
    }
}

#[test]
fn malformed_planes_fail_before_mutation() {
    for data in [vec![0; 11], [vec![0; 10], vec![0xff, 0xff]].concat()] {
        let mut frame = GeometryFrame {
            width: 2,
            height: 1,
            subsampling: Some([1, 1]),
            data,
        };
        let before = frame.data.clone();
        assert!(Pixelize::parse("").unwrap().apply(&mut frame, 10).is_err());
        assert_eq!(frame.data, before);
    }
}

#[cfg(feature = "media")]
#[test]
fn truncated_source_does_not_publish_pixelized_output() {
    let directory =
        std::env::temp_dir().join(format!("fvid-pixelize-failure-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    let input = directory.join("input.y4m");
    let output = directory.join("output.mkv");
    let mut data = b"YUV4MPEG2 W4 H2 F25:1 Ip C420jpeg\nFRAME\n".to_vec();
    data.extend(vec![128; 12]);
    data.extend(b"FRAME\n");
    data.extend(vec![128; 11]);
    std::fs::write(&input, data).unwrap();
    let transform = fvid::media::LosslessTransform {
        pixelize: Some("3:5:min:7".into()),
        ..Default::default()
    };
    assert!(
        fvid::media::transcode_lossless(&input, &output, transform, &Default::default()).is_err()
    );
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 1);
}
