use fvid::{native_geometry::GeometryFrame, native_pixels::Negate};

fn frame(data: Vec<u8>, rgb: bool) -> GeometryFrame {
    GeometryFrame {
        width: 2,
        height: 2,
        subsampling: if rgb { None } else { Some([2, 2]) },
        data,
    }
}

#[test]
fn inverts_all_planes_and_rgb_channels_without_range_rescaling() {
    for args in ["", "0", "1"] {
        let mut yuv = frame(vec![16, 235, 0, 255, 128, 240], false);
        Negate::parse(args).unwrap().apply(&mut yuv, 8).unwrap();
        assert_eq!(yuv.data, [239, 20, 255, 0, 127, 15]);
        let original = vec![0, 10, 255, 1, 20, 128, 2, 30, 127, 3, 40, 64];
        let mut rgb = frame(original.clone(), true);
        Negate::parse(args).unwrap().apply(&mut rgb, 8).unwrap();
        assert_eq!(
            rgb.data,
            [255, 245, 0, 254, 235, 127, 253, 225, 128, 252, 215, 191]
        );
        Negate::parse(args).unwrap().apply(&mut rgb, 8).unwrap();
        assert_eq!(rgb.data, original);
    }
}

#[test]
fn high_depth_samples_use_their_own_maximum_and_fail_atomically() {
    for depth in 9..=16 {
        let max = ((1u32 << depth) - 1) as u16;
        let samples = [0, 1, max, max - 1, max / 2, 64];
        let mut image = frame(
            samples.into_iter().flat_map(u16::to_le_bytes).collect(),
            false,
        );
        Negate.apply(&mut image, depth).unwrap();
        let actual: Vec<_> = image
            .data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(actual, samples.map(|v| max - v));
    }
    for data in [vec![0, 0, 0, 4], vec![0, 0, 1]] {
        let mut image = frame(data.clone(), false);
        assert!(Negate.apply(&mut image, 10).is_err());
        assert_eq!(image.data, data);
    }
    for args in ["2", "true", "components=y", "\0"] {
        assert!(Negate::parse(args).is_err());
    }
    assert!(Negate.apply(&mut frame(vec![], false), 7).is_err());
    assert!(Negate.apply(&mut frame(vec![], true), 10).is_err());
}

#[test]
fn shared_request_and_cli_use_owned_negate() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    for args in ["0", "1"] {
        let request = fvid::media_info::DecodeTransform {
            negate: Some(args.into()),
            ..Default::default()
        };
        let stats = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 25);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode", path.to_str().unwrap(), "--negate", args])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["backend"],
            "fvid"
        );
        #[cfg(feature = "media")]
        assert_eq!(
            fvid::media::decode_video_transformed(&path, request)
                .unwrap()
                .backend,
            "fvid"
        );
    }
}

/// Explicit reference-only test; FFmpeg is never loaded by production code.
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn pixels_match_independent_reference() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").expect("set reference executable");
    for (format, rgb, depth) in [
        ("rgb24", true, 8),
        ("yuv420p", false, 8),
        ("yuv420p10le", false, 10),
        ("yuv420p16le", false, 16),
    ] {
        let data = if rgb {
            vec![0, 10, 255, 1, 20, 128, 2, 30, 127, 3, 40, 64]
        } else if depth == 8 {
            vec![16, 235, 0, 255, 128, 240]
        } else {
            [0u16, 64, 128, 511, 512, 1023]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        let mut own = frame(data.clone(), rgb);
        Negate.apply(&mut own, depth).unwrap();
        for filter in ["negate", "negate=negate_alpha=1"] {
            let mut child = Command::new(&ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "rawvideo",
                    "-pixel_format",
                    format,
                    "-video_size",
                    "2x2",
                    "-i",
                    "pipe:0",
                    "-vf",
                    filter,
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
            child.stdin.take().unwrap().write_all(&data).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(own.data, out.stdout, "{format} {filter}");
        }
    }
}
