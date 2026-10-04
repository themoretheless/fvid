use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::{
    owned_eq::{Equalizer, EqualizerProgram},
    owned_frame::GeometryFrame,
};
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
fn eq_frame_and_init_modes_keep_filter_clock_before_selection() {
    let source = fixture("playback-errors/eq-time-25.y4m");
    let encoded = std::env::temp_dir().join(format!("fvid-eq-base-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(
        &source,
        &encoded,
        Default::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    for (case, args, interval, expected, _first) in [
        (
            0,
            "brightness=n/10:eval=frame",
            None,
            vec![64, 130, 197, 255, 255],
            0,
        ),
        (
            1,
            "brightness=2.5*t:eval=frame",
            None,
            vec![64, 130, 197, 255, 255],
            0,
        ),
        (2, "brightness=n/10", None, vec![64, 80, 96, 112, 128], 0),
        (
            3,
            "brightness=n/10:eval=frame",
            Some((80000, 280000)),
            vec![80, 146, 213],
            2,
        ),
        (
            4,
            "brightness=2.5*t:eval=frame",
            Some((80000, 280000)),
            vec![130, 197, 255],
            2,
        ),
    ] {
        for (source_case, input) in [&source, &encoded].into_iter().enumerate() {
            for library in [false, true] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-eq-time-{}-{case}-{source_case}-{library}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    eq: Some(args.into()),
                    framestep: Some("2".into()),
                    interval,
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
                assert_eq!(stats.video_frames, expected.len() as u64);
                let mut demux = fvid_media::owned_webm::WebmReader::open(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    Default::default(),
                )
                .unwrap();
                demux.scan_all().unwrap();
                for (n, packet) in demux.packets.iter().enumerate() {
                    assert_eq!(packet.pts_ns, (2 * n) as i64 * 40_000_000,"{case} {source_case} {library}");
                    assert_eq!(packet.duration_ns, Some(40_000_000));
                }
                let mut reader = NativeReader::software(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                for (n, &y) in expected.iter().enumerate() {
                    let frame = reader.read_frame_raw().unwrap().unwrap();
                    assert_eq!(
                        VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                        [vec![y; 16], vec![100; 4], vec![150; 4]].concat(),
                        "{args} {n} {source_case} {library}"
                    );
                    let (start, end, scale) = reader.frame_interval().unwrap();
                    assert_eq!(
                        start * 1_000_000_000 / u128::from(scale),
                        (2 * n) as u128 * 40_000_000
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
#[test]
fn eq_quotes_nonfinite_refusal_and_rewind_are_explicit() {
    assert!(Equalizer::parse("brightness=n/10:eval=frame").is_err());
    let program = EqualizerProgram::parse("saturation='if(gte(n,1),0,1)':eval=frame").unwrap();
    for _pass in 0..2 {
        for n in 0..3 {
            let mut frame = GeometryFrame {
                width: 1,
                height: 1,
                subsampling: Some([1, 1]),
                data: vec![64, 100, 150],
            };
            program.apply(&mut frame, 8, n, None).unwrap();
            assert_eq!(
                frame.data,
                if n == 0 {
                    vec![64, 100, 150]
                } else {
                    vec![64, 127, 127]
                }
            );
        }
    }
    for args in [
        "eval=no",
        "brightness='n",
        "contrast=unknown",
        "contrast=if(1,1,unknown)",
        "gamma=random(0)",
        "brightness=r",
        "brightness=pos",
    ] {
        assert!(EqualizerProgram::parse(args).is_err(), "{args}");
    }
    let mut frame = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![64, 100, 150],
    };
    assert!(
        EqualizerProgram::parse("brightness=t:eval=frame")
            .unwrap()
            .apply(&mut frame, 8, 0, None)
            .is_err()
    );
    assert_eq!(frame.data, [64, 100, 150]);
}
#[test]
fn eq_timeline_sources_and_cli_use_owned_pipeline() {
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
                eq: Some("brightness=n/10:eval=frame".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("playback-errors/eq-time-25.y4m");
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-eq-cli-{}-{command}.{}",
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
            .args(["--eq", "brightness=n/10:eval=frame"])
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
