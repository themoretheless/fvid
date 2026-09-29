use fvid::{
    native_geometry::GeometryFrame,
    native_morphology::{Morphology, MorphologyKind},
};
const KINDS: [MorphologyKind; 2] = [MorphologyKind::Dilation, MorphologyKind::Erosion];
fn frame(depth: u8, sx: usize, sy: usize, rgb: bool) -> GeometryFrame {
    let max = (1u32 << depth) - 1;
    let count = if rgb { 192 } else { 64 + 128 / (sx * sy) };
    let data = (0..count)
        .flat_map(|i| {
            let v = ((i * i * 31 + i * 17) as u32 % (max + 1)) as u16;
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
        subsampling: if rgb { None } else { Some([sx, sy]) },
        data,
    }
}
#[test]
fn masks_thresholds_and_rejection_are_preserved() {
    for kind in KINDS {
        for depth in [8, 10, 16] {
            let mut f = frame(depth, 2, 2, false);
            let original = f.data.clone();
            Morphology::parse(kind, "coordinates=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Morphology::parse(kind, "threshold0=0:threshold1=0:threshold2=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Morphology::parse(kind, "threshold0=3:threshold1=0:threshold2=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            let bytes = if depth == 8 { 1 } else { 2 };
            assert_eq!(&f.data[64 * bytes..], &original[64 * bytes..]);
            let samples = |data: Vec<u8>| {
                data.chunks_exact(bytes)
                    .map(|b| {
                        if bytes == 1 {
                            u16::from(b[0])
                        } else {
                            u16::from_le_bytes([b[0], b[1]])
                        }
                    })
                    .collect::<Vec<_>>()
            };
            for (before, after) in samples(original).into_iter().zip(samples(f.data.clone())) {
                match kind {
                    MorphologyKind::Dilation => assert!(after >= before && after - before <= 3),
                    MorphologyKind::Erosion => assert!(before >= after && before - after <= 3),
                }
            }
            f.data.pop();
            let before = f.data.clone();
            assert!(
                Morphology::parse(kind, "")
                    .unwrap()
                    .apply(&mut f, depth)
                    .is_err()
            );
            assert_eq!(f.data, before);
        }
    }
    for args in [
        "coordinates=256",
        "threshold0=-1",
        "threshold1=65536",
        "threshold2=NaN",
        "what=3",
        "1:2:3:4:5:6",
    ] {
        assert!(Morphology::parse(MorphologyKind::Dilation, args).is_err());
    }
    for kind in KINDS {
        let mut f = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([2, 2]),
            data: vec![17, 23, 99],
        };
        Morphology::parse(kind, "")
            .unwrap()
            .apply(&mut f, 8)
            .unwrap();
        assert_eq!(f.data, [17, 23, 99]);
        let mut f = frame(10, 1, 1, false);
        f.data[..2].copy_from_slice(&1024u16.to_le_bytes());
        let before = f.data.clone();
        assert!(
            Morphology::parse(kind, "")
                .unwrap()
                .apply(&mut f, 10)
                .is_err()
        );
        assert_eq!(before, f.data);
    }
}
#[test]
fn cli_and_api_use_owned_path() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "tests/fixtures/video.mp4",
        "tests/fixtures/hevc/main10-ipb.mp4",
    ] {
        let path = root.join(file);
        let base = fvid::native_media::decode_video(&path).unwrap();
        let request = fvid::media_info::DecodeTransform {
            dilation: Some("coordinates=170:threshold0=16".into()),
            erosion: Some("threshold1=0".into()),
            ..Default::default()
        };
        let actual = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(actual.backend, "fvid");
        assert_eq!(actual.video_frames, base.video_frames);
        assert_eq!(actual.pixel_format, base.pixel_format);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args([
                "media",
                "decode",
                path.to_str().unwrap(),
                "--erosion",
                "threshold1=0",
                "--dilation",
                "coordinates=170:threshold0=16",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["backend"], "fvid");
        assert_eq!(value["video_frames"], base.video_frames);
        #[cfg(feature = "media")]
        assert_eq!(
            fvid::media::decode_video_transformed(&path, request)
                .unwrap()
                .backend,
            "fvid"
        );
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            "missing.mp4",
            "--erosion",
            "",
            "--erosion",
            "",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("duplicate erosion"));
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG; reference executable only"]
fn pixels_match_reference() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let executable = std::env::var("FVID_REFERENCE_FFMPEG").unwrap();
    for (depth, format, sx, sy, rgb) in [
        (8, "yuv444p", 1, 1, false),
        (10, "yuv444p10le", 1, 1, false),
        (16, "yuv444p16le", 1, 1, false),
        (8, "yuv422p", 2, 1, false),
        (10, "yuv422p10le", 2, 1, false),
        (16, "yuv422p16le", 2, 1, false),
        (8, "yuv420p", 2, 2, false),
        (10, "yuv420p10le", 2, 2, false),
        (16, "yuv420p16le", 2, 2, false),
        (8, "rgb24", 1, 1, true),
    ] {
        for kind in KINDS {
            for args in [
                "",
                "coordinates=1",
                "coordinates=2",
                "coordinates=4",
                "coordinates=8",
                "coordinates=16",
                "coordinates=32",
                "coordinates=64",
                "coordinates=128",
                "threshold0=3:threshold1=0:threshold2=17",
                "170:11:5:0:0",
            ] {
                let mut f = frame(depth, sx, sy, rgb);
                let data = f.data.clone();
                Morphology::parse(kind, args)
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
                if let Some(i) = f.data.iter().zip(&out.stdout).position(|(a, b)| a != b) {
                    panic!(
                        "{format} {filter} byte {i}: own={} ref={}",
                        f.data[i], out.stdout[i]
                    );
                }
                assert_eq!(f.data.len(), out.stdout.len());
            }
        }
    }
}
