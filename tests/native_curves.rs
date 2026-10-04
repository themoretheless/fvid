use fvid::media::{CopyOptions, DecodeTransform, LosslessTransform};
use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
use fvid_media::owned_curves::Curves;
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
fn plateau_and_cache_precision_match_saved_independent_pixels() {
    let filter = Curves::parse("all='0/0 0.25/0 0.5/0.5 0.75/1 1/1':interp=pchip").unwrap();
    for depth in [8, 16, 8] {
        let mut input = std::fs::read(fixture(&format!(
            "playback-errors/curves-plateau.rgb{}",
            depth * 3
        )))
        .unwrap();
        let expected = std::fs::read(fixture(&format!(
            "playback-errors/curves-plateau-{depth}.expected.raw"
        )))
        .unwrap();
        filter.apply_rgb(&mut input, depth, 3).unwrap();
        assert_eq!(input, expected);
        if depth == 8 {
            assert_eq!(input[7], 255);
        }
    }
    let mut rgba = vec![0, 0, 0, 17, 255, 255, 255, 203];
    Curves::parse("negative")
        .unwrap()
        .apply_rgb(&mut rgba, 8, 4)
        .unwrap();
    assert_eq!(rgba, [255, 255, 255, 17, 0, 0, 0, 203]);
    for args in [
        "all='0/0 0/1'",
        "all=NaN/1",
        "all=2/0",
        "all=0.5",
        "interp=linear",
        "preset=nope",
        "r='0/0",
    ] {
        assert!(Curves::parse(args).is_err());
    }
    let filter = Curves::parse("all='0/0 0.001/1 1/1'").unwrap();
    let mut data = vec![17; 3];
    let before = data.clone();
    assert!(
        filter
            .apply_rgb(&mut data, 8, 3)
            .unwrap_err()
            .contains("too close")
    );
    assert_eq!(data, before);
    let mut invalid = vec![255; 6];
    let before = invalid.clone();
    assert!(
        Curves::parse("")
            .unwrap()
            .apply_rgb(&mut invalid, 10, 3)
            .is_err()
    );
    assert_eq!(invalid, before);
}
#[test]
fn acv_priority_plot_and_explicit_timeline_use_owned_contracts() {
    let file = fixture("playback-errors/curves-negative.acv");
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(
        fvid_media::owned_curves::parse_acv(&bytes).unwrap()[3],
        [(0., 1.), (1., 0.)]
    );
    for end in [0, 1, 3, 5, bytes.len() - 1] {
        assert!(fvid_media::owned_curves::parse_acv(&bytes[..end]).is_err());
    }
    for version in [1u16, 4] {
        let mut data = bytes.clone();
        data[..2].copy_from_slice(&version.to_be_bytes());
        assert!(fvid_media::owned_curves::parse_acv(&data).is_ok());
    }
    let plot = std::env::temp_dir().join(format!("fvid-curves-{}.gnuplot", std::process::id()));
    let filter = Curves::parse(&format!(
        "all='0/0 1/1':psfile='{}':preset=vintage:r='0/1 1/0':plot='{}'",
        file.display(),
        plot.display()
    ))
    .unwrap();
    let mut data = vec![0, 0, 0, 255, 255, 255];
    filter.apply_rgb(&mut data, 8, 3).unwrap();
    assert_eq!(data, [0, 255, 255, 255, 0, 0]);
    let script = std::fs::read_to_string(&plot).unwrap();
    assert!(script.contains("plot '-' using 1:2"));
    assert!(script.contains("# knot 3 0 1"));
    assert_eq!(script.lines().filter(|&l| l == "e").count(), 4);
    std::fs::remove_file(plot).unwrap();
    let filter = Curves::parse("negative:enable='between(n,1,2)'").unwrap();
    let mut data = vec![10, 20, 30];
    assert!(
        filter
            .apply_rgb(&mut data, 8, 3)
            .unwrap_err()
            .contains("clock")
    );
    filter
        .apply_rgb_clock(&mut data, 8, 3, 1, 1, 0, Some(0.))
        .unwrap();
    assert_eq!(data, [10, 20, 30]);
    filter
        .apply_rgb_clock(&mut data, 8, 3, 1, 1, 1, Some(0.04))
        .unwrap();
    assert_eq!(data, [245, 235, 225]);
    let mut malformed = vec![0; 2];
    assert!(
        filter
            .apply_rgb_clock(&mut malformed, 8, 3, 1, 1, 0, None)
            .is_err()
    );
}
#[test]
fn grayscale_inversion_accepts_library_and_root_exports_before_selection() {
    for depth in [8, 10] {
        let source = fixture(&format!("playback-errors/curves-gray-{depth}.y4m"));
        let base = std::env::temp_dir().join(format!(
            "fvid-curves-base-{}-{depth}.mkv",
            std::process::id()
        ));
        fvid::media::transcode_lossless(
            &source,
            &base,
            Default::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        for input in [&source, &base] {
            for library in [false, true] {
                for step in [1usize, 2] {
                    let output = std::env::temp_dir().join(format!(
                        "fvid-curves-{}-{depth}-{library}-{step}.mkv",
                        std::process::id()
                    ));
                    let transform = LosslessTransform {
                        curves: Some("negative".into()),
                        framestep: Some(step.to_string()),
                        ..Default::default()
                    };
                    let stats = if library {
                        fvid_media::transcode_lossless(
                            input,
                            &output,
                            transform,
                            &Default::default(),
                        )
                    } else {
                        fvid::media::transcode_lossless(
                            input,
                            &output,
                            transform,
                            &Default::default(),
                        )
                    }
                    .unwrap();
                    assert_eq!(stats.video_frames, 4 / step as u64);
                    assert_eq!(stats.backend, "fvid");
                    let mut reader = NativeReader::software(
                        Cursor::new(std::fs::read(&output).unwrap()),
                        usize::MAX,
                    )
                    .unwrap();
                    for n in (0..4).step_by(step) {
                        let raw = reader.read_frame_raw().unwrap().unwrap();
                        let data = VideoGeometry::default().apply(&raw, 4, 4).unwrap().data;
                        let saved = std::fs::read(fixture(&format!(
                            "playback-errors/curves-gray-{depth}.expected.raw"
                        )))
                        .unwrap();
                        let size = if depth == 8 { 48 } else { 96 };
                        assert_eq!(
                            data,
                            &saved[n * size..(n + 1) * size],
                            "{depth} {n} {library}"
                        );
                        let (start, _, scale) = reader.frame_interval().unwrap();
                        assert_eq!(
                            start * 1_000_000_000 / u128::from(scale),
                            n as u128 * 40_000_000
                        );
                    }
                    assert!(reader.read_frame_raw().unwrap().is_none());
                    std::fs::remove_file(output).unwrap();
                }
            }
        }
        std::fs::remove_file(base).unwrap();
    }
}
#[test]
fn native_sources_cli_and_clip_clock_accept_curves() {
    for name in [
        "playback-errors/curves-matrix-unspecified.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/framestep-opus.mkv",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(name),
            DecodeTransform {
                curves: Some("vintage".into()),
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("playback-errors/curves-gray-8.y4m");
    for command in ["decode", "export-y4m", "transcode-lossless"] {
        let output = std::env::temp_dir().join(format!(
            "fvid-curves-cli-{}-{command}.{}",
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
        cmd.args(["--curves", "negative"]);
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
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-curves-clock-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            curves: Some("negative:enable='eq(n,0)*gte(t,0.04)'".into()),
            interval: Some((40000, 120000)),
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
        for n in 0..2 {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            let data = VideoGeometry::default().apply(&raw, 4, 4).unwrap().data;
            if n == 0 {
                let saved =
                    std::fs::read(fixture("playback-errors/curves-gray-8.expected.raw")).unwrap();
                assert_eq!(&data, &saved[48..96]);
            } else {
                assert_eq!(&data, [vec![16; 16], vec![128; 32]].concat().as_slice());
            }
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn absent_matrix_matches_player_default_and_accepts_short_avc_source() {
    assert!(matches!(
        fvid_media::owned_yuv_rgb::Matrix::from_code(0).unwrap(),
        fvid_media::owned_yuv_rgb::Matrix::Bt601
    ));
    let path = fixture("playback-errors/curves-matrix-unspecified.mp4");
    let reader =
        NativeReader::software(Cursor::new(std::fs::read(&path).unwrap()), usize::MAX).unwrap();
    assert_eq!(reader.colour().matrix, 0);
    let stats = fvid::media::decode_video_transformed(
        &path,
        DecodeTransform {
            curves: Some("negative".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.video_frames, 5);
    assert_eq!(stats.backend, "fvid");
}
