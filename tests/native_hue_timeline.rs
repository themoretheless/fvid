use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::owned_hue::HueProgram;
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
fn animated_hue_keeps_every_filter_input_before_selection_and_clip_time() {
    let original = fixture("playback-errors/hue-time-25.y4m");
    let encoded =
        std::env::temp_dir().join(format!("fvid-hue-baseline-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &original,
        &encoded,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    for (source_case, source) in [&original, &encoded].into_iter().enumerate() {
        for (case, args, interval, first, count) in [
            (0, "h=90*n", None, 0, 5),
            (1, "h=2250*t", None, 0, 5),
            (2, "h=90*n", Some((80000, 280000)), 2, 3),
            (3, "h=2250*t", Some((80000, 280000)), 2, 3),
        ] {
            for library in [false, true] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-hue-time-{}-{source_case}-{case}-{library}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    hue: Some(args.into()),
                    framestep: Some("2".into()),
                    interval,
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
                assert_eq!(stats.video_frames, count);
                let mut reader = NativeReader::software(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                for emitted in 0..count {
                    let index = first + emitted * 2;
                    let rotation_index = if args.contains('t') {
                        index
                    } else {
                        emitted * 2
                    };
                    let (u, v) = if rotation_index % 4 == 0 {
                        (100, 150)
                    } else {
                        (156, 106)
                    };
                    let frame = reader.read_frame_raw().unwrap().unwrap();
                    assert_eq!(
                        VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                        [vec![64 + index as u8 * 8; 16], vec![u; 4], vec![v; 4]].concat(),
                        "{args} {interval:?} {emitted} {library}"
                    );
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                std::fs::remove_file(output).unwrap();
            }
        }
    }
    std::fs::remove_file(encoded).unwrap();
}
#[test]
fn quoted_expressions_and_runtime_refusals_are_explicit() {
    assert!(fvid_media::owned_hue::Hue::parse("h=90*n").is_err());
    let program = HueProgram::parse("h='90*n':s='if(gte(n,2),0,1)':b='t'").unwrap();
    let mut frame = fvid_media::owned_frame::GeometryFrame {
        width: 2,
        height: 2,
        subsampling: Some([2, 2]),
        data: vec![64; 6],
    };
    program
        .at(2, Some(1.))
        .unwrap()
        .apply(&mut frame, 8)
        .unwrap();
    assert_eq!(frame.data, vec![89, 89, 89, 89, 128, 128]);
    for args in [
        "h=unknown",
        "h=if(1,0,unknown)",
        "h='n",
        "h=n:H=n",
        "h=random(0)",
        "r=n",
        "h=pts",
        "h=r",
        "h=tb",
    ] {
        assert!(HueProgram::parse(args).is_err(), "{args}");
    }
    assert!(HueProgram::parse("h=t").unwrap().at(0, None).is_err());
    assert!(HueProgram::parse("b=1/0").unwrap().at(0, Some(0.)).is_err());
}
#[test]
fn animated_hue_routes_native_codecs_and_cli_without_legacy() {
    for name in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
        "playback-errors/hue-time-25.y4m",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(name),
            DecodeTransform {
                hue: Some("h=90*n:s=1-t".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("playback-errors/hue-time-25.y4m");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(&source)
        .args(["--hue", "h=90*n:s=1-t"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for command in ["export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-hue-cli-{}-{command}.{}",
            std::process::id(),
            if command == "export-y4m" {
                "y4m"
            } else {
                "mkv"
            }
        ));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", command])
            .arg(&source)
            .arg(&output)
            .args(["--hue", "h=90*n:s=1-t"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        std::fs::remove_file(output).unwrap();
    }
}
