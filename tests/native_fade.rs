use fvid::{
    media::{CopyOptions, DecodeTransform, LosslessTransform},
    native_geometry::VideoGeometry,
    playback_native::NativeReader,
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
fn frame_fade_numeric_endpoints_and_precision_validation() {
    use fvid_media::{owned_fade::Fade, owned_frame::GeometryFrame};
    let filter = Fade::parse("out:1:2").unwrap();
    for (n, y, u, v) in [
        (0, 64, 100, 150),
        (1, 80, 100, 150),
        (2, 56, 114, 139),
        (3, 16, 128, 128),
        (5, 16, 128, 128),
    ] {
        let mut frame = GeometryFrame {
            width: 4,
            height: 4,
            subsampling: Some([2, 2]),
            data: [vec![64 + n as u8 * 16; 16], vec![100; 4], vec![150; 4]].concat(),
        };
        filter.apply(&mut frame, 8, false, n).unwrap();
        assert_eq!(frame.data, [vec![y; 16], vec![u; 4], vec![v; 4]].concat());
    }
    let mut rgb = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: None,
        data: vec![255, 128, 0],
    };
    Fade::parse("in:0:2")
        .unwrap()
        .apply(&mut rgb, 8, true, 1)
        .unwrap();
    assert_eq!(rgb.data, [128, 64, 0]);
    let mut invalid = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: Some([1, 1]),
        data: vec![0, 0, 0, 4, 0, 0],
    };
    let before = invalid.data.clone();
    assert!(filter.apply(&mut invalid, 10, false, 0).is_err());
    assert_eq!(invalid.data, before);
    for args in [
        "in:0:0",
        "out:-1:2",
        "type=other",
        "n=2.5",
        "alpha=1",
        "color=not-a-color",
        "d=-1",
    ] {
        assert!(Fade::parse(args).is_err(), "{args}");
    }
}
#[test]
fn own_fade_exports_synthetic_values_and_preserves_input_clock() {
    let source = fixture("playback-errors/fade-six-frames.y4m");
    let output = std::env::temp_dir().join(format!("fvid-fade-{}.mkv", std::process::id()));
    let transform = LosslessTransform {
        fade: Some("out:1:2".into()),
        framestep: Some("2".into()),
        ..Default::default()
    };
    let plan =
        fvid::media::plan_transcode_lossless(&source, &transform, &CopyOptions::default(), None)
            .unwrap();
    assert!(plan.steps.iter().any(|s| s.detail.contains("fade")));
    let stats =
        fvid::media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
            .unwrap();
    assert_eq!(stats.decoded_frames, 6);
    assert_eq!(stats.video_frames, 3);
    let mut reader =
        NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX).unwrap();
    for (index, (y, u, v)) in [(64, 100, 150), (56, 114, 139), (16, 128, 128)]
        .into_iter()
        .enumerate()
    {
        let frame = reader.read_frame_raw().unwrap().unwrap();
        let samples = VideoGeometry::default().apply(&frame, 4, 4).unwrap();
        assert_eq!(samples.data, [vec![y; 16], vec![u; 4], vec![v; 4]].concat());
        let (start, end, scale) = reader.frame_interval().unwrap();
        assert_eq!(
            start * 1_000_000_000 / u128::from(scale),
            index as u128 * 80_000_000
        );
        assert_eq!(
            (end - start) * 1_000_000_000 / u128::from(scale),
            40_000_000
        );
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    let baseline = output.with_extension("baseline.mkv");
    fvid_media::transcode_lossless(
        &source,
        &baseline,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    for input in [&source, &baseline] {
        let library = output.with_extension("library.mkv");
        fvid_media::transcode_lossless(
            input,
            &library,
            LosslessTransform {
                fade: Some("out:1:2".into()),
                framestep: Some("2".into()),
                ..Default::default()
            },
            &CopyOptions::default(),
        )
        .unwrap();
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&library).unwrap()), usize::MAX)
                .unwrap();
        for (y, u, v) in [(64, 100, 150), (56, 114, 139), (16, 128, 128)] {
            let frame = reader.read_frame_raw().unwrap().unwrap();
            assert_eq!(
                VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                [vec![y; 16], vec![u; 4], vec![v; 4]].concat()
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(library).unwrap();
    }
    std::fs::remove_file(baseline).unwrap();
    std::fs::remove_file(output).unwrap();
}
#[test]
fn fade_uses_owned_decode_and_export_across_codecs_and_cli() {
    for name in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
        "playback-errors/fade-six-frames.y4m",
    ] {
        let source = fixture(name);
        let stats = fvid::media::decode_video_transformed(
            &source,
            DecodeTransform {
                fade: Some("out:1:2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
        let destination = std::env::temp_dir().join(format!(
            "fvid-fade-route-{}-{}.mkv",
            std::process::id(),
            name.replace('/', "-")
        ));
        let exported = fvid::media::transcode_lossless(
            &source,
            &destination,
            LosslessTransform {
                fade: Some("out:1:2".into()),
                ..Default::default()
            },
            &CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(exported.video_frames, stats.video_frames);
        assert_eq!(exported.backend, "fvid");
        let mut output = NativeReader::software(
            Cursor::new(std::fs::read(&destination).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut frames = 0;
        while output.read_frame_raw().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, exported.video_frames);
        std::fs::remove_file(destination).unwrap();

        assert!(
            fvid_media::decode_video_transformed(
                &fixture("playback-errors/fade-six-frames.y4m"),
                DecodeTransform {
                    fade: Some("out:1:2".into()),
                    ..Default::default()
                }
            )
            .is_ok()
        );
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(fixture("playback-errors/fade-six-frames.y4m"))
        .args(["--fade", "out:1:2"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["video_frames"],
        6
    );
    for command in ["transcode-lossless", "export-y4m"] {
        let destination = std::env::temp_dir().join(format!(
            "fvid-fade-cli-{}.{extension}",
            std::process::id(),
            extension = if command == "export-y4m" {
                "y4m"
            } else {
                "mkv"
            }
        ));
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", command])
            .arg(fixture("playback-errors/fade-six-frames.y4m"))
            .arg(&destination)
            .args(["--fade", "out:1:2"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::remove_file(destination).unwrap();
    }
}

#[test]
fn chroma_halfway_rounding_has_a_synthetic_video_regression() {
    let source = fixture("playback-errors/fade-chroma-tie.y4m");
    let mut bytes = Cursor::new(Vec::new());
    let filters = fvid::native_pixels::PixelFilters::from_request(&DecodeTransform {
        fade: Some("in:0:2".into()),
        ..Default::default()
    })
    .unwrap();
    fvid::native_lossless_y4m::write(
        &source,
        &mut bytes,
        &VideoGeometry::default(),
        &filters,
        None,
        None,
    )
    .unwrap();
    let mut reader = NativeReader::software(Cursor::new(bytes.into_inner()), usize::MAX).unwrap();
    reader.read_frame_raw().unwrap().unwrap();
    let frame = reader.read_frame_raw().unwrap().unwrap();
    let samples = VideoGeometry::default().apply(&frame, 4, 4).unwrap();
    assert_eq!(
        samples.data,
        [vec![40; 16], vec![128; 4], vec![129; 4]].concat()
    );
}

#[test]
fn colored_rgb_fade_has_exact_endpoints_and_retains_alpha() {
    use fvid_media::owned_fade::Fade;
    let filter = Fade::parse("out:0:2:color=blue").unwrap();
    for (n, expected) in [
        (0, [255, 0, 0, 17]),
        (1, [127, 0, 128, 17]),
        (2, [0, 0, 255, 17]),
    ] {
        let mut pixel = [255, 0, 0, 17];
        filter.apply_rgb(&mut pixel, 8, 4, n).unwrap();
        assert_eq!(pixel, expected);
    }
    let mut pixel = [65535u16, 0, 0, 1234]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    filter.apply_rgb(&mut pixel, 16, 4, 1).unwrap();
    let values = pixel
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect::<Vec<_>>();
    assert_eq!(values, [32767, 0, 32768, 1234]);
    for args in [
        "color=blue@0.5",
        "color=0x0000ff80",
        "color=not-a-color",
        "alpha=1",
    ] {
        assert!(Fade::parse(args).is_err());
    }
    assert!(Fade::parse("color=Blue@1").is_ok());
}

#[test]
fn colored_yuv_fade_exports_known_ten_bit_endpoints() {
    let source = fixture("playback-errors/fade-colour-10.y4m");
    let transform = LosslessTransform {
        fade: Some("out:0:2:color=blue".into()),
        ..Default::default()
    };
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-color-fade-{}-{library}.mkv",
            std::process::id()
        ));
        let stats = if library {
            fvid_media::transcode_lossless(
                &source,
                &output,
                transform.clone(),
                &CopyOptions::default(),
            )
        } else {
            fvid::media::transcode_lossless(
                &source,
                &output,
                transform.clone(),
                &CopyOptions::default(),
            )
        }
        .unwrap();
        assert_eq!(stats.video_frames, 3);
        assert_eq!(stats.pixel_format, "yuv444p10le");
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        for n in 0..3 {
            let frame = reader.read_frame_raw().unwrap().unwrap();
            let data = VideoGeometry::default().apply(&frame, 4, 4).unwrap().data;
            let values = data
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>();
            if n == 0 {
                assert_eq!(values, [vec![256; 16], vec![512; 32]].concat());
            }
            if n == 2 {
                assert_eq!(
                    values,
                    [vec![164; 16], vec![960; 16], vec![439; 16]].concat()
                );
            }
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(output).unwrap();
    }
    for (index, name) in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = std::env::temp_dir().join(format!(
            "fvid-color-fade-codecs-{}-{index}.mkv",
            std::process::id()
        ));
        let stats = fvid::media::transcode_lossless(
            &source,
            &output,
            transform.clone(),
            &CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        let mut count = 0;
        while reader.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, stats.video_frames);
        std::fs::remove_file(output).unwrap();
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(&source)
        .args(["--fade", "out:0:2:color=blue"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}


#[test]
fn time_fade_preserves_gates_and_fractional_clock_before_framestep() {
    for (case, args, expected) in [
        (
            0,
            "out:s=1:d=0.1",
            [
                (64, 100, 150),
                (59, 109, 143),
                (16, 128, 128),
                (16, 128, 128),
                (16, 128, 128),
            ],
        ),
        (
            1,
            "out:st=0.1:d=0.1",
            [
                (64, 100, 150),
                (80, 100, 150),
                (69, 109, 143),
                (16, 128, 128),
                (16, 128, 128),
            ],
        ),
        (
            2,
            "out:st=0.1:n=2",
            [
                (64, 100, 150),
                (80, 100, 150),
                (56, 114, 139),
                (16, 128, 128),
                (16, 128, 128),
            ],
        ),
    ] {
        for (rate_name, rate_num, rate_den) in [("25", 25u128, 1u128), ("ntsc", 30000, 1001)] {
            let source = fixture(&format!("playback-errors/fade-time-{rate_name}.y4m"));
            for library in [false, true] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-time-fade-{}-{case}-{library}-{rate_name}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    fade: Some(args.into()),
                    framestep: Some("2".into()),
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
                assert_eq!(stats.video_frames, 5);
                assert_eq!(stats.decoded_frames, 9);
                let mut reader = NativeReader::software(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                for (n, (y, u, v)) in expected.into_iter().enumerate() {
                    let frame = reader.read_frame_raw().unwrap().unwrap();
                    assert_eq!(
                        VideoGeometry::default().apply(&frame, 4, 4).unwrap().data,
                        [vec![y; 16], vec![u; 4], vec![v; 4]].concat(),
                        "{args} n={n} library={library}"
                    );
                    let (start, end, scale) = reader.frame_interval().unwrap();
                    let actual_start = start * 1_000_000_000 / u128::from(scale);
                    let expected_start = n as u128 * 2 * rate_den * 1_000_000_000 / rate_num;
                    assert!(actual_start.abs_diff(expected_start) <= 1_000_000);
                    let actual_duration = (end - start) * 1_000_000_000 / u128::from(scale);
                    assert!(
                        actual_duration.abs_diff(rate_den * 1_000_000_000 / rate_num) <= 1_000_000
                    );
                }
                std::fs::remove_file(output).unwrap();
            }
        }
    }
}
#[test]
fn time_fade_stream_state_errors_and_rewind_are_explicit() {
    use fvid_media::owned_fade::{Fade, FadeClock, FrameTime};
    let fade = FadeClock::parse("out:s=1:d=0.1").unwrap();
    for pass in 0..2 {
        for (n, expected) in [(0, 255), (1, 255), (2, 170), (3, 85), (4, 0)] {
            let value = fade
                .at(n, Some(FrameTime::new(n as u128, 25).unwrap()))
                .unwrap();
            let mut rgb = [255; 3];
            value.apply_rgb(&mut rgb, 8, 3, n).unwrap();
            assert_eq!(rgb, [expected; 3], "pass {pass} n={n}");
        }
    }
    assert!(fade.at(5, None).is_err());
    assert!(fade.at(5, Some(FrameTime::new(5, 30).unwrap())).is_err());
    let mut rgb = [255; 3];
    assert!(
        Fade::parse("d=1")
            .unwrap()
            .apply_rgb(&mut rgb, 8, 3, 0)
            .is_err()
    );
    assert_eq!(rgb, [255; 3]);
}