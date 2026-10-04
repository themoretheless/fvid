use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::{owned_frame::GeometryFrame, owned_tmix::TemporalMix};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn owned_tmix_numeric_warmup_weights_masks_before_selection() {
    for (case, args, expected) in [
        (
            0,
            "",
            [
                (64, 100, 150),
                (69, 101, 149),
                (80, 102, 147),
                (96, 104, 144),
                (112, 106, 141),
                (128, 108, 138),
                (144, 110, 135),
                (160, 112, 132),
            ],
        ),
        (
            1,
            "frames=3:weights=1 2 3",
            [
                (64, 100, 150),
                (72, 101, 148),
                (85, 103, 146),
                (101, 105, 143),
                (117, 107, 140),
                (133, 109, 137),
                (149, 111, 134),
                (165, 113, 131),
            ],
        ),
        (3, "frames=1024", [(64, 100, 150); 8]),
        (
            2,
            "frames=3:planes=0",
            [
                (64, 100, 150),
                (64, 100, 150),
                (64, 100, 150),
                (80, 102, 147),
                (96, 104, 144),
                (112, 106, 141),
                (128, 108, 138),
                (144, 110, 135),
            ],
        ),
    ] {
        let source = fixture("playback-errors/tmix-ramp-8.y4m");
        let encoded =
            std::env::temp_dir().join(format!("fvid-tmix-base-{}-{case}.mkv", std::process::id()));
        fvid::media::transcode_lossless(
            &source,
            &encoded,
            Default::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        for (source_case, input) in [&source, &encoded].into_iter().enumerate() {
            for library in [false, true] {
                for step in [1usize, 2] {
                    let output = std::env::temp_dir().join(format!(
                        "fvid-tmix-{}-{case}-{source_case}-{library}-{step}.mkv",
                        std::process::id()
                    ));
                    let transform = LosslessTransform {
                        tmix: Some(args.into()),
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
                    for (n, &(y, u, v)) in expected.iter().enumerate().step_by(step) {
                        let frame = reader.read_frame_raw().unwrap().unwrap();
                        assert_eq!(
                            VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                            [vec![y; 16], vec![u; 4], vec![v; 4]].concat(),
                            "{args} {n} {source_case} {library}"
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
fn mix_rewind_clipping_validation_and_scalar_refusal() {
    let filter = TemporalMix::parse("frames=2").unwrap();
    for _ in 0..2 {
        for (n, input, expected) in [(0, 0, 0), (1, 1, 1), (2, 2, 2)] {
            let mut frame = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([1, 1]),
                data: vec![input; 3],
            };
            filter.apply(&mut frame, 8, n).unwrap();
            assert_eq!(frame.data, [expected; 3]);
        }
    }
    let mut invalid = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![0; 2],
    };
    assert!(filter.apply(&mut invalid, 8, 0).is_err());
    let mut frame = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![10; 3],
    };
    filter.apply(&mut frame, 8, 3).unwrap();
    assert_eq!(frame.data, [6; 3]);
    let header =
        fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420").unwrap();
    assert!(
        fvid_media::owned_y4m_decode::transform_frame_requested(
            &header,
            &[16; 24],
            &DecodeTransform {
                tmix: Some("".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    for args in [
        "frames=0",
        "frames=1025",
        "frames=1.5",
        "scale=-1",
        "planes=16",
        "weights=NaN",
        "weights=1 -1:frames=2",
        "enable=0",
        "unknown=1",
    ] {
        assert!(TemporalMix::parse(args).is_err(), "{args}");
    }
}
#[test]
fn tmix_sources_and_cli_use_owned_processing() {
    for name in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
        "playback-errors/tmix-ramp-10.y4m",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(name),
            DecodeTransform {
                tmix: Some("frames=2:weights=1 3".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("playback-errors/tmix-ramp-8.y4m");
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-tmix-cli-{}-{command}.{}",
            std::process::id(),
            if command == "export-y4m" {
                "y4m"
            } else {
                "mkv"
            }
        ));
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        cmd.args(["media", command]).arg(&source);
        if command != "decode" {
            cmd.arg(&output);
        }
        let result = cmd
            .args(["--tmix", "frames=2:weights=1 3"])
            .output()
            .unwrap();
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
fn ten_bit_mix_retains_source_precision_and_clip_restarts_warmup() {
    for (case, source_name, interval, depth, expected) in [
        (
            0,
            "playback-errors/tmix-ramp-10.y4m",
            None,
            10,
            vec![
                (256u16, 400u16, 600u16),
                (277, 403, 596),
                (320, 408, 588),
                (384, 416, 576),
                (448, 424, 564),
                (512, 432, 552),
                (576, 440, 540),
                (640, 448, 528),
            ],
        ),
        (
            1,
            "playback-errors/tmix-ramp-8.y4m",
            Some((80000, 200000)),
            8,
            vec![(96, 104, 144), (101, 105, 143), (112, 106, 141)],
        ),
    ] {
        for library in [false, true] {
            let output = std::env::temp_dir().join(format!(
                "fvid-tmix-precision-{}-{case}-{library}.mkv",
                std::process::id()
            ));
            let transform = LosslessTransform {
                tmix: Some("".into()),
                interval,
                ..Default::default()
            };
            let source = fixture(source_name);
            let stats = if library {
                fvid_media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            } else {
                fvid::media::transcode_lossless(
                    &source,
                    &output,
                    transform,
                    &CopyOptions::default(),
                )
            }
            .unwrap();
            assert_eq!(stats.video_frames, expected.len() as u64);
            let mut reader =
                NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                    .unwrap();
            for &(y, u, v) in &expected {
                let samples = [vec![y; 16], vec![u; 4], vec![v; 4]].concat();
                let pixels: Vec<u8> = if depth == 8 {
                    samples.into_iter().map(|v| v as u8).collect()
                } else {
                    samples.into_iter().flat_map(u16::to_le_bytes).collect()
                };
                let frame = reader.read_frame_raw().unwrap().unwrap();
                assert_eq!(
                    VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                    pixels,
                    "{case} {library}"
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            std::fs::remove_file(output).unwrap();
        }
    }
}
