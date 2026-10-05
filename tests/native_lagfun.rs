use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::{owned_frame::GeometryFrame, owned_lagfun::LagFun};
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
fn lagfun_preserves_fractional_history_before_framestep_and_rewind() {
    let values = [
        (100, 140, 200),
        (95, 133, 190),
        (90, 126, 180),
        (86, 120, 171),
        (81, 114, 163),
        (77, 108, 155),
        (74, 103, 147),
        (70, 98, 140),
    ];
    for depth in [8u8, 10] {
        let source = fixture(&format!("playback-errors/lagfun-dark-{depth}.y4m"));
        let baseline =
            std::env::temp_dir().join(format!("fvid-lag-base-{}-{depth}.mkv", std::process::id()));
        fvid::media::transcode_lossless(
            &source,
            &baseline,
            Default::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        for (source_case, input) in [&source, &baseline].into_iter().enumerate() {
            for step in [1u64, 2] {
                for library in [false, true] {
                    let output = std::env::temp_dir().join(format!(
                        "fvid-lag-{}-{depth}-{source_case}-{step}-{library}.mkv",
                        std::process::id()
                    ));
                    let transform = LosslessTransform {
                        lagfun: Some("".into()),
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
                    assert_eq!(stats.video_frames, 8 / step);
                    let mut reader = NativeReader::software(
                        Cursor::new(std::fs::read(&output).unwrap()),
                        usize::MAX,
                    )
                    .unwrap();
                    let values10 = [
                        (400u16, 560u16, 800u16),
                        (380, 532, 760),
                        (361, 505, 722),
                        (343, 480, 686),
                        (326, 456, 652),
                        (310, 433, 619),
                        (294, 412, 588),
                        (279, 391, 559),
                    ];
                    for (n, &(y, u, v)) in values.iter().enumerate() {
                        if !(n as u64).is_multiple_of(step) {
                            continue;
                        }
                        let expected = if depth == 8 {
                            [vec![y; 16], vec![u; 4], vec![v; 4]].concat()
                        } else {
                            let (y, u, v) = values10[n];
                            [vec![y; 16], vec![u; 4], vec![v; 4]]
                                .concat()
                                .into_iter()
                                .flat_map(u16::to_le_bytes)
                                .collect()
                        };
                        let frame = reader.read_frame_raw().unwrap().unwrap();
                        assert_eq!(
                            VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                            expected,
                            "{depth} {n} {source_case} {library}"
                        );
                    }
                    assert!(reader.read_frame_raw().unwrap().is_none());
                    std::fs::remove_file(output).unwrap();
                }
            }
        }
        std::fs::remove_file(baseline).unwrap();
    }
    let filter = LagFun::parse("decay=0.5:planes=1").unwrap();
    for _pass in 0..2 {
        for (n, y) in [(0, 100), (1, 50), (2, 25)] {
            let mut frame = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([1, 1]),
                data: vec![if n == 0 { 100 } else { 0 }, 20, 30],
            };
            filter.apply(&mut frame, 8, n, None).unwrap();
            assert_eq!(frame.data, [y, 20, 30]);
        }
    }
}
#[test]
fn lagfun_disabled_inputs_update_history_and_invalid_frames_do_not() {
    let filter = LagFun::parse("decay=0.5:enable='gte(n,2)'").unwrap();
    for (n, input, expected) in [(0, 100, 100), (1, 0, 0), (2, 0, 25), (3, 0, 12)] {
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![input; 3],
        };
        filter.apply(&mut frame, 8, n, None).unwrap();
        assert_eq!(frame.data, [expected; 3]);
    }
    let mut invalid = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![0; 2],
    };
    assert!(filter.apply(&mut invalid, 8, 0, None).is_err());
    let mut valid = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![0; 3],
    };
    filter.apply(&mut valid, 8, 4, None).unwrap();
    assert_eq!(valid.data, [6; 3]);
    for args in [
        "decay=-1",
        "decay=2",
        "planes=16",
        "planes=0.5",
        "decay=NaN",
        "unknown=1",
    ] {
        assert!(LagFun::parse(args).is_err(), "{args}");
    }
}
#[test]
fn lagfun_native_source_and_cli_routes_accept_without_legacy() {
    for name in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(name),
            DecodeTransform {
                lagfun: Some("decay=0.5:planes=1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("playback-errors/lagfun-dark-8.y4m");
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-lag-cli-{}-{command}.{}",
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
            .args(["--lagfun", "decay=0.5:planes=1"])
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
fn lagfun_clip_starts_new_history_and_scalar_api_refuses_fake_history() {
    let source = fixture("playback-errors/lagfun-dark-8.y4m");
    let header =
        fvid_media::owned_y4m::Header::parse(b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420").unwrap();
    assert!(
        fvid_media::owned_y4m_decode::transform_frame_requested(
            &header,
            &[16; 24],
            &DecodeTransform {
                lagfun: Some("".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-lag-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            lagfun: Some("".into()),
            interval: Some((40000, 160000)),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        } else {
            fvid::media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        }
        .unwrap();
        assert_eq!(stats.video_frames, 3);
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            assert_eq!(
                VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                [vec![16; 16], vec![64; 8]].concat()
            );
        }
        std::fs::remove_file(output).unwrap();
    }
}
