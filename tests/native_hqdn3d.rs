use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::{owned_frame::GeometryFrame, owned_hqdn3d::HqDn3d};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn raw_frames(name: &str) -> Vec<Vec<u8>> {
    let data = std::fs::read(fixture(&format!("playback-errors/{name}"))).unwrap();
    let start = data.iter().position(|&v| v == b'\n').unwrap() + 1;
    let size = if name.contains("grid") {
        27
    } else if name.contains("10") {
        48
    } else {
        24
    };
    data[start..]
        .chunks_exact(size + 6)
        .map(|chunk| {
            assert_eq!(&chunk[..6], b"FRAME\n");
            chunk[6..].to_vec()
        })
        .collect()
}
#[test]
fn denoising_matches_saved_pixels_before_frame_selection_and_through_exports() {
    for (case, name, depth, width, height, args) in [
        (0, "hqdn3d-noise-8", 8, 4, 4, ""),
        (1, "hqdn3d-noise-10", 10, 4, 4, ""),
        (2, "hqdn3d-grid-8", 8, 5, 3, "20:15:30:25"),
    ] {
        let source = fixture(&format!("playback-errors/{name}.y4m"));
        let expected =
            std::fs::read(fixture(&format!("playback-errors/{name}.expected.raw"))).unwrap();
        let size = if depth == 10 {
            48
        } else if width == 5 {
            27
        } else {
            24
        };
        let encoded =
            std::env::temp_dir().join(format!("fvid-hqdn-base-{}-{case}.mkv", std::process::id()));
        fvid::media::transcode_lossless(
            &source,
            &encoded,
            Default::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        for input in [&source, &encoded] {
            for library in [false, true] {
                for step in [1usize, 2] {
                    let output = std::env::temp_dir().join(format!(
                        "fvid-hqdn-{}-{case}-{library}-{step}.mkv",
                        std::process::id()
                    ));
                    let transform = LosslessTransform {
                        hqdn3d: Some(args.into()),
                        framestep: Some(step.to_string()),
                        ..Default::default()
                    };
                    let stats = if library {
                        fvid_media::transcode_lossless(
                            input,
                            &output,
                            transform,
                            &CopyOptions::default(),
                        )
                    } else {
                        fvid::media::transcode_lossless(
                            input,
                            &output,
                            transform,
                            &CopyOptions::default(),
                        )
                    }
                    .unwrap();
                    assert_eq!(stats.backend, "fvid");
                    assert_eq!(stats.video_frames, 8 / step as u64);
                    let mut reader = NativeReader::software(
                        Cursor::new(std::fs::read(&output).unwrap()),
                        usize::MAX,
                    )
                    .unwrap();
                    for (n, pixels) in expected.chunks_exact(size).enumerate().step_by(step) {
                        let raw = reader.read_frame_raw().unwrap().unwrap();
                        assert_eq!(
                            VideoGeometry::default()
                                .apply(&raw, width, height)
                                .unwrap()
                                .data,
                            pixels,
                            "{name} {n} {library} {step}"
                        );
                        let (start, end, scale) = reader.frame_interval().unwrap();
                        assert_eq!(
                            start * 1_000_000_000 / u128::from(scale),
                            n as u128 * 40_000_000
                        );
                        assert_eq!(
                            (end - start) * 1_000_000_000 / u128::from(scale),
                            40_000_000
                        );
                    }
                    assert!(reader.read_frame_raw().unwrap().is_none());
                    std::fs::remove_file(output).unwrap();
                }
            }
        }
        std::fs::remove_file(encoded).unwrap();
    }
}
#[test]
fn rewind_disabled_history_clipping_and_refusals_are_explicit() {
    let frames = raw_frames("hqdn3d-noise-8.y4m");
    let expected = std::fs::read(fixture("playback-errors/hqdn3d-noise-8.expected.raw")).unwrap();
    let filter = HqDn3d::parse("").unwrap();
    for _ in 0..2 {
        for (n, data) in frames.iter().enumerate() {
            let mut frame = GeometryFrame {
                width: 4,
                height: 4,
                subsampling: Some([2, 2]),
                data: data.clone(),
            };
            if n == 1 {
                let mut bad = GeometryFrame {
                    width: 4,
                    height: 4,
                    subsampling: Some([2, 2]),
                    data: vec![0; 23],
                };
                assert!(filter.apply(&mut bad, 8, n as u64, None).is_err());
            }
            filter
                .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                .unwrap();
            assert_eq!(frame.data, &expected[n * 24..(n + 1) * 24]);
        }
    }
    let filter = HqDn3d::parse("enable=eq(n,2)").unwrap();
    for (n, data) in frames.iter().take(3).enumerate() {
        let mut frame = GeometryFrame {
            width: 4,
            height: 4,
            subsampling: Some([2, 2]),
            data: data.clone(),
        };
        filter
            .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        assert_eq!(
            frame.data,
            if n == 2 {
                &expected[48..72]
            } else {
                data.as_slice()
            }
        );
    }
    let before = frames[0].clone();
    let mut frame = GeometryFrame {
        width: 4,
        height: 4,
        subsampling: Some([2, 2]),
        data: before.clone(),
    };
    assert!(filter.apply(&mut frame, 8, 2, None).is_err());
    assert_eq!(frame.data, before);
    let header = fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H4 F25:1 Ip C420").unwrap();
    assert!(
        fvid_media::owned_y4m_decode::transform_frame_requested(
            &header,
            &before,
            &DecodeTransform {
                hqdn3d: Some("".into()),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("persistent")
    );
    for args in ["-1", "NaN", "foo=1", "1:2:3:4:5", "enable=unknown"] {
        assert!(HqDn3d::parse(args).is_err());
    }
    let source = fixture("playback-errors/hqdn3d-noise-8.y4m");
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-hqdn-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            hqdn3d: Some("".into()),
            interval: Some((80000, 200000)),
            ..Default::default()
        };
        if library {
            fvid_media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
        } else {
            fvid::media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
        }
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        let raw = reader.read_frame_raw().unwrap().unwrap();
        assert_eq!(
            VideoGeometry::default().apply(&raw, 4, 4).unwrap().data,
            frames[2]
        );
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn native_codecs_and_cli_keep_owned_denoising() {
    for name in [
        "video.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
    ] {
        let path = fixture(name);
        let transform = DecodeTransform {
            hqdn3d: Some("4:3:6:4.5".into()),
            ..Default::default()
        };
        let stats = fvid::native_media::decode_video_request(&path, &transform).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let path = fixture("playback-errors/hqdn3d-noise-8.y4m");
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-hqdn-cli-{}-{command}.{}",
            std::process::id(),
            if command == "export-y4m" {
                "y4m"
            } else {
                "mkv"
            }
        ));
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        cmd.args(["media", command]).arg(&path);
        if command != "decode" {
            cmd.arg(&output);
        }
        cmd.args(["--hqdn3d", "4:3:6:4.5"]);
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if command != "decode" {
            std::fs::remove_file(output).unwrap();
        }
    }
}

#[test]
fn extreme_reference_overflow_is_reproduced_and_owned_pixels_remain_valid() {
    let reference = std::fs::read(fixture(
        "playback-errors/hqdn3d-extreme-14.reference-invalid.raw",
    ))
    .unwrap();
    assert_eq!(u16::from_le_bytes([reference[0], reference[1]]), 16383);
    assert!(u16::from_le_bytes([reference[2], reference[3]]) > 16383);
    let source = fixture("playback-errors/hqdn3d-extreme-14.y4m");
    let data = std::fs::read(&source).unwrap();
    let start = data.iter().position(|&b| b == b'\n').unwrap() + 1;
    let filter = HqDn3d::parse("").unwrap();
    for (n, chunk) in data[start..].as_chunks::<60>().0.iter().enumerate() {
        assert_eq!(&chunk[..6], b"FRAME\n");
        let mut frame = GeometryFrame {
            width: 5,
            height: 3,
            subsampling: Some([2, 2]),
            data: chunk[6..].to_vec(),
        };
        filter
            .apply(&mut frame, 14, n as u64, Some(n as f64 / 25.))
            .unwrap();
        assert!(
            frame
                .data
                .as_chunks::<2>().0.iter()
                .all(|v| u16::from_le_bytes([v[0], v[1]]) <= 16383)
        );
        if n == 0 {
            assert_eq!(
                frame.data[..10]
                    .as_chunks::<2>().0.iter()
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect::<Vec<_>>(),
                [16383, 2, 16381, 2, 16381]
            );
        }
    }
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-hqdn-extreme-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            hqdn3d: Some("".into()),
            ..Default::default()
        };
        if library {
            fvid_media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
        } else {
            fvid::media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
        }
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        for n in 0..8 {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            let frame = VideoGeometry::default().apply(&raw, 5, 3).unwrap();
            assert!(
                frame
                    .data
                    .as_chunks::<2>().0.iter()
                    .all(|v| u16::from_le_bytes([v[0], v[1]]) <= 16383)
            );
            if n == 0 {
                assert_eq!(&frame.data[..10], &[255, 63, 2, 0, 253, 63, 2, 0, 253, 63]);
            }
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(output).unwrap();
    }
}
