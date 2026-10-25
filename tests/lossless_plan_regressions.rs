use fvid::media_info::{LosslessTransform, RotateAngle, ScaleSize};
use std::path::Path;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/source-8x4.y4m")
}

#[test]
fn lossless_planner_emits_filter_steps_for_all_supported_pixel_filters() {
    let source = fixture();
    let cases = [
        (
            "tmix",
            LosslessTransform {
                tmix: Some("frames=2".into()),
                ..Default::default()
            },
        ),
        (
            "hqdn3d",
            LosslessTransform {
                hqdn3d: Some("4:3:6:4.5".into()),
                ..Default::default()
            },
        ),
        (
            "smartblur",
            LosslessTransform {
                smartblur: Some("lr=1.5:ls=-0.5".into()),
                ..Default::default()
            },
        ),
        (
            "sab",
            LosslessTransform {
                sab: Some("lr=1.5:cr=1.5".into()),
                ..Default::default()
            },
        ),
        (
            "cas",
            LosslessTransform {
                cas: Some("strength=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "vignette",
            LosslessTransform {
                vignette: Some("angle=PI/4".into()),
                ..Default::default()
            },
        ),
        (
            "curves",
            LosslessTransform {
                curves: Some("preset=lighter".into()),
                ..Default::default()
            },
        ),
        (
            "colorbalance",
            LosslessTransform {
                colorbalance: Some("rs=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "colorlevels",
            LosslessTransform {
                colorlevels: Some("rimin=0.05".into()),
                ..Default::default()
            },
        ),
        (
            "colorchannelmixer",
            LosslessTransform {
                colorchannelmixer: Some("rr=0.9:gg=1.0:bb=1.0".into()),
                ..Default::default()
            },
        ),
        (
            "monochrome",
            LosslessTransform {
                monochrome: Some("cb=0.1:cr=-0.1".into()),
                ..Default::default()
            },
        ),
        (
            "grayworld",
            LosslessTransform {
                grayworld: Some(String::new()),
                ..Default::default()
            },
        ),
        (
            "drawbox",
            LosslessTransform {
                drawbox: Some("x=0:y=0:w=4:h=2:color=white".into()),
                ..Default::default()
            },
        ),
        (
            "drawgrid",
            LosslessTransform {
                drawgrid: Some("w=2:h=2:color=white".into()),
                ..Default::default()
            },
        ),
        (
            "lagfun",
            LosslessTransform {
                lagfun: Some("decay=0.9".into()),
                ..Default::default()
            },
        ),
        (
            "bitplanenoise",
            LosslessTransform {
                bitplanenoise: Some("bitplane=1:filter=1".into()),
                ..Default::default()
            },
        ),
        (
            "deband",
            LosslessTransform {
                deband: Some("1thr=0.02".into()),
                ..Default::default()
            },
        ),
        (
            "gradfun",
            LosslessTransform {
                gradfun: Some("strength=1.2".into()),
                ..Default::default()
            },
        ),
        (
            "lenscorrection",
            LosslessTransform {
                lenscorrection: Some("k1=-0.1".into()),
                ..Default::default()
            },
        ),
        (
            "removegrain",
            LosslessTransform {
                removegrain: Some("m0=1".into()),
                ..Default::default()
            },
        ),
        (
            "yaepblur",
            LosslessTransform {
                yaepblur: Some("r=3".into()),
                ..Default::default()
            },
        ),
        (
            "vibrance",
            LosslessTransform {
                vibrance: Some("intensity=0.2".into()),
                ..Default::default()
            },
        ),
        (
            "colorize",
            LosslessTransform {
                colorize: Some("hue=120:saturation=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "exposure",
            LosslessTransform {
                exposure: Some("exposure=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "colorcontrast",
            LosslessTransform {
                colorcontrast: Some("rc=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "colorcorrect",
            LosslessTransform {
                colorcorrect: Some("rl=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "lutyuv",
            LosslessTransform {
                lutyuv: Some("y=val".into()),
                ..Default::default()
            },
        ),
        (
            "colorhold",
            LosslessTransform {
                colorhold: Some("color=red:similarity=0.2".into()),
                ..Default::default()
            },
        ),
        (
            "perspective",
            LosslessTransform {
                perspective: Some(String::new()),
                ..Default::default()
            },
        ),
    ];

    for (name, transform) in cases {
        let plan = fvid::native_plan::transcode_lossless(&source, &transform)
            .unwrap_or_else(|e| panic!("{name} plan failed: {e}"));
        let filter_steps: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.action == "filter")
            .collect();
        assert_eq!(
            filter_steps.len(),
            1,
            "{name} expected 1 filter step in transcode_lossless plan, got {filter_steps:?}"
        );
        assert!(
            filter_steps[0].detail.starts_with(&format!("FVid {name}=")),
            "{name} unexpected filter step detail: {}",
            filter_steps[0].detail
        );
    }
}

#[test]
fn lossless_supports_rejects_nonempty_zscale_and_planner_records_framestep() {
    let source = fixture();
    let with_zscale = LosslessTransform {
        zscale: Some("matrix=bt709".into()),
        ..Default::default()
    };
    assert!(
        !fvid::native_lossless::supports(&with_zscale),
        "LosslessTransform with zscale must not be claimed as supported by native_lossless"
    );
    assert!(
        fvid::native_plan::transcode_lossless(&source, &with_zscale).is_err(),
        "transcode_lossless must refuse unsupported zscale"
    );

    let with_framestep = LosslessTransform {
        framestep: Some("2".into()),
        ..Default::default()
    };
    assert!(fvid::native_lossless::supports_framestep(&with_framestep));
    let plan = fvid::native_plan::transcode_lossless(&source, &with_framestep)
        .expect("transcode_lossless should plan supported framestep");
    assert!(
        plan.steps
            .iter()
            .any(|s| s.action == "filter" && s.detail.contains("framestep=2")),
        "expected framestep filter step in plan: {:?}",
        plan.steps
    );
}

#[test]
fn rotated_and_scaled_y4m_export_computes_pixel_aspect_from_post_rotation_canvas() {
    let source = fixture(); // 8x4, A1:1
    let temp = std::env::temp_dir().join(format!("fvid-rotated-aspect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let out = temp.join("rotated-scaled.y4m");

    let transform = LosslessTransform {
        rotate: Some(RotateAngle::parse("90").unwrap()),
        scale: Some(ScaleSize {
            width: 4,
            height: 8,
        }),
        ..Default::default()
    };
    assert_eq!(transform.rotate.unwrap().size(8, 4), (4, 8));
    let (geometry, filters) = fvid::native_lossless::configuration(&transform).unwrap();
    fvid::native_export::export_y4m_pipeline(&source, &out, None, &geometry, &filters).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let header_end = bytes.iter().position(|&b| b == b'\n').unwrap();
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
    // 8x4 rotated by 90 deg becomes 4x8; scaling to 4x8 is 1:1 on the rotated canvas,
    // so pixel aspect must remain A1:1 (not A4:1 from pre-rotation 8x4)!
    assert!(
        header.contains(" W4 H8 ") && header.contains(" A1:1 "),
        "unexpected Y4M header after 90-degree rotate + 4x8 identity scale: {header}"
    );

    std::fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn native_lossless_planner_validates_first_frame_presence_and_spatial_geometry_on_y4m_matroska_and_mp4() {
    let source_y4m = fixture();
    let zero_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/zero-frames-8x4.y4m");
    let source_mkv = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let source_mp4 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/shared-mp4-av-multiple.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-native-lossless-plan-geom-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let default_opts = fvid::media::CopyOptions::default();

    // 1. 0-frame Y4M source (`zero-frames-8x4.y4m`) must be rejected by both `fvid::native_plan::transcode_lossless`,
    //    `fvid::media::plan_transcode_lossless`, and `fvid::media::transcode_lossless` with "input has no video frames".
    let out_zero = temp.join("zero.mkv");
    let exec_zero = fvid::media::transcode_lossless(
        &zero_y4m,
        &out_zero,
        LosslessTransform::default(),
        &default_opts,
    )
    .expect_err("transcode_lossless must reject 0-frame Y4M");
    assert!(
        exec_zero.contains("input has no video frames"),
        "unexpected transcode_lossless error on 0-frame Y4M: {exec_zero}"
    );
    assert!(!out_zero.exists());

    let plan_zero = fvid::media::plan_transcode_lossless(
        &zero_y4m,
        &LosslessTransform::default(),
        &default_opts,
        None,
    )
    .expect_err("plan_transcode_lossless must reject 0-frame Y4M under default CopyOptions");
    assert!(
        plan_zero.contains("input has no video frames"),
        "unexpected plan_transcode_lossless error on 0-frame Y4M: {plan_zero}"
    );

    // 2. Out-of-bounds crop (100x100 on 8x4 Y4M, 4x3 Matroska, and 32x16 MP4) must be rejected
    //    by `fvid::media::plan_transcode_lossless` under default CopyOptions with the same geometry error
    //    as `fvid::media::transcode_lossless`.
    let bad_crop = LosslessTransform {
        crop: Some(fvid::media_info::CropRect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        }),
        ..Default::default()
    };
    for (label, src) in [("y4m", &source_y4m), ("mkv", &source_mkv), ("mp4", &source_mp4)] {
        let out_crop = temp.join(format!("crop-bad-{label}.mkv"));
        let exec_crop = fvid::media::transcode_lossless(src, &out_crop, bad_crop.clone(), &default_opts)
            .expect_err(&format!("{label}: transcode_lossless must reject out-of-bounds 100x100 crop"));
        assert!(!out_crop.exists());

        let plan_crop = fvid::media::plan_transcode_lossless(src, &bad_crop, &default_opts, None)
            .expect_err(&format!("{label}: plan_transcode_lossless must reject out-of-bounds 100x100 crop"));
        assert_eq!(plan_crop, exec_crop, "{label}: plan and exec crop error mismatch");
    }

    // 3. Odd chroma crop (`width: 3, height: 2` on 8x4 YUV420 Y4M) and undersized pad (`width: 2, height: 2` on 8x4 Y4M)
    //    must be rejected by `fvid::media::plan_transcode_lossless` under default CopyOptions with the same error
    //    as `fvid::media::transcode_lossless`.
    let odd_crop = LosslessTransform {
        crop: Some(fvid::media_info::CropRect {
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        }),
        ..Default::default()
    };
    let out_odd = temp.join("odd-crop.mkv");
    let exec_odd = fvid::media::transcode_lossless(&source_y4m, &out_odd, odd_crop.clone(), &default_opts)
        .expect_err("transcode_lossless must reject odd chroma crop on 4:2:0 Y4M");
    let plan_odd = fvid::media::plan_transcode_lossless(&source_y4m, &odd_crop, &default_opts, None)
        .expect_err("plan_transcode_lossless must reject odd chroma crop on 4:2:0 Y4M");
    assert_eq!(plan_odd, exec_odd);

    let small_pad = LosslessTransform {
        pad: Some(fvid::media_info::PadRect {
            width: 2,
            height: 2,
            x: 0,
            y: 0,
        }),
        ..Default::default()
    };
    let out_pad = temp.join("small-pad.mkv");
    let exec_pad = fvid::media::transcode_lossless(&source_y4m, &out_pad, small_pad.clone(), &default_opts)
        .expect_err("transcode_lossless must reject undersized pad canvas");
    let plan_pad = fvid::media::plan_transcode_lossless(&source_y4m, &small_pad, &default_opts, None)
        .expect_err("plan_transcode_lossless must reject undersized pad canvas");
    assert_eq!(plan_pad, exec_pad);

    let _ = std::fs::remove_dir_all(&temp);
}



#[test]
fn cli_decode_supports_shuffleframes_reverse_input_format_and_rejects_zero_selected_frames() {
    let source_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/two-frames-8x4.y4m");
    let zero_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/zero-frames-8x4.y4m");
    let source_mkv = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let zero_mkv = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xfade-subtitle-budget-parity/zero-frames-ffv1.mkv");
    let source_mp4 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/shared-mp4-av-multiple.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-cli-decode-temporal-zero-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. CLI `fvid media decode` with `--shuffleframes "1 0"`, `--reverse ""`, and `--input-format`
    //    must succeed in default builds (`!cfg(feature = "media")`) across Y4M, Matroska, and MP4.
    for (label, path, fmt) in [
        ("y4m", &source_y4m, "y4m"),
        ("mkv", &source_mkv, "matroska"),
        ("mp4", &source_mp4, "mp4"),
    ] {
        for (flag, val) in [
            ("--shuffleframes", "1 0"),
            ("--reverse", ""),
            ("--input-format", fmt),
        ] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "decode", path.to_str().unwrap(), flag, val])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{label} decode {flag} {val} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert!(
                parsed["video_frames"].as_u64().unwrap() > 0,
                "{label} decode {flag} {val} returned 0 frames: {parsed}"
            );
        }
    }

    // 2. CLI `fvid media decode` with `--input-format` mismatch (`mp4` on Y4M) must fail with
    //    an owned diagnostic rather than falling through to `"requires cargo build --release --features media"`.
    let bad_fmt = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            source_y4m.to_str().unwrap(),
            "--input-format",
            "mp4",
        ])
        .output()
        .unwrap();
    assert!(!bad_fmt.status.success());
    let bad_fmt_err = String::from_utf8_lossy(&bad_fmt.stderr);
    assert!(
        !bad_fmt_err.contains("requires cargo build --release --features media"),
        "unexpected legacy feature fallback error: {bad_fmt_err}"
    );

    // 3. 0-frame Y4M and 0-frame Matroska inputs, or a 2-frame input with `--shuffleframes "2 1 0"`
    //    (group of 3 on 2 frames -> 0 emitted frames) or `--shuffleframes "-1"` (drop every frame -> 0 emitted frames)
    //    must be rejected by both `fvid::media::decode_video_transformed` and `fvid media decode`.
    for (label, path, transform, cli_extra) in [
        (
            "zero-y4m",
            &zero_y4m,
            fvid::media_info::DecodeTransform::default(),
            vec![],
        ),
        (
            "zero-mkv",
            &zero_mkv,
            fvid::media_info::DecodeTransform::default(),
            vec![],
        ),
        (
            "shuffle-drop-y4m",
            &source_y4m,
            fvid::media_info::DecodeTransform {
                shuffleframes: Some("2 1 0".into()),
                ..Default::default()
            },
            vec!["--shuffleframes", "2 1 0"],
        ),
        (
            "shuffle-drop-mkv",
            &source_mkv,
            fvid::media_info::DecodeTransform {
                shuffleframes: Some("-1".into()),
                ..Default::default()
            },
            vec!["--shuffleframes", "-1"],
        ),
        (
            "shuffle-drop-mp4",
            &source_mp4,
            fvid::media_info::DecodeTransform {
                shuffleframes: Some("-1".into()),
                ..Default::default()
            },
            vec!["--shuffleframes", "-1"],
        ),
    ] {
        let lib_err = fvid::media::decode_video_transformed(path, transform)
            .expect_err(&format!("{label}: decode_video_transformed must reject 0 emitted frames"));
        assert!(
            lib_err.contains("no decoded video frames") || lib_err.contains("no video frames"),
            "{label}: unexpected decode_video_transformed error: {lib_err}"
        );

        let mut cmd = vec!["media", "decode", path.to_str().unwrap()];
        cmd.extend(cli_extra);
        let cli_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&cmd)
            .output()
            .unwrap();
        assert!(
            !cli_out.status.success(),
            "{label}: CLI decode must fail on 0 emitted frames, got stdout: {}",
            String::from_utf8_lossy(&cli_out.stdout)
        );
    }

    let _ = std::fs::remove_dir_all(&temp);
}



#[test]
fn native_overlay_plan_and_cli_validate_post_geometry_pixel_filters_and_record_rotate_step() {
    let source_y4m = fixture();
    let overlay_y4m = fixture();
    let source_mp4 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/shared-mp4-av-multiple.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-native-overlay-plan-filter-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. `fvid::native_plan::transcode_lossless` with `overlay` AND an oversized `boxblur` radius (`lr=10` on 8x4 Y4M)
    //    or `deband` with `coupling=1` on 4:2:0 Y4M must reject during planning with the same error as execution.
    for bad_transform in [
        LosslessTransform {
            overlay: Some(fvid::media_info::OverlaySpec {
                path: overlay_y4m.clone(),
                x: 0,
                y: 0,
            }),
            boxblur: Some("lr=10".into()),
            ..Default::default()
        },
        LosslessTransform {
            overlay: Some(fvid::media_info::OverlaySpec {
                path: overlay_y4m.clone(),
                x: 0,
                y: 0,
            }),
            deband: Some("coupling=1".into()),
            ..Default::default()
        },
    ] {
        let out_file = temp.join("bad-overlay-filter.mkv");
        let exec_err = fvid::media::transcode_lossless(
            &source_y4m,
            &out_file,
            bad_transform.clone(),
            &fvid::media::CopyOptions::default(),
        )
        .expect_err("transcode_lossless with overlay + incompatible filter must fail");
        assert!(!out_file.exists());

        let plan_err = fvid::native_plan::transcode_lossless(&source_y4m, &bad_transform)
            .expect_err("native_plan::transcode_lossless with overlay + incompatible filter must fail");
        assert_eq!(plan_err, exec_err);
    }

    // 2. CLI `fvid media plan overlay` and `fvid media plan decode --overlay` must also validate
    //    post-compositing `filters` (`--boxblur lr=10`, `--deband coupling=1`) on the first frame.
    let out_boxblur = temp.join("cli-overlay-boxblur.mkv");
    for (plan_args, exec_args) in [
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                "--boxblur",
                "lr=10",
            ],
            vec![
                "media",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                out_boxblur.to_str().unwrap(),
                "--boxblur",
                "lr=10",
            ],
        ),
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                "--deband",
                "coupling=1",
            ],
            vec![
                "media",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                out_boxblur.to_str().unwrap(),
                "--deband",
                "coupling=1",
            ],
        ),
        (
            vec![
                "media",
                "plan",
                "decode",
                source_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
                "--boxblur",
                "lr=10",
            ],
            vec![
                "media",
                "decode",
                source_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
                "--boxblur",
                "lr=10",
            ],
        ),
        (
            vec![
                "media",
                "plan",
                "decode",
                source_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
                "--deband",
                "coupling=1",
            ],
            vec![
                "media",
                "decode",
                source_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
                "--deband",
                "coupling=1",
            ],
        ),
    ] {
        let exec_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&exec_args)
            .output()
            .unwrap();
        assert!(
            !exec_out.status.success(),
            "expected execution failure for {exec_args:?}"
        );
        let plan_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&plan_args)
            .output()
            .unwrap();
        assert!(
            !plan_out.status.success(),
            "expected plan failure for {plan_args:?}, got stdout: {}",
            String::from_utf8_lossy(&plan_out.stdout)
        );
    }

    // 3. When `--rotate` is used alone with `fvid media plan overlay` (or `fvid media plan transcode-lossless --overlay`),
    //    the `geometry` plan step must include `rotate` in its detail string just like `fvid media plan decode --overlay`.
    let rotate_plan_out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "plan",
            "overlay",
            source_y4m.to_str().unwrap(),
            overlay_y4m.to_str().unwrap(),
            "--rotate",
            "180",
        ])
        .output()
        .unwrap();
    assert!(
        rotate_plan_out.status.success(),
        "plan overlay --rotate 180 failed: {}",
        String::from_utf8_lossy(&rotate_plan_out.stderr)
    );
    let rotate_plan: serde_json::Value = serde_json::from_slice(&rotate_plan_out.stdout).unwrap();
    let geom_steps: Vec<_> = rotate_plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["action"] == "geometry")
        .collect();
    assert_eq!(geom_steps.len(), 1);
    assert!(
        geom_steps[0]["detail"]
            .as_str()
            .unwrap()
            .contains("rotate"),
        "expected geometry step detail in plan overlay --rotate 180 to mention rotate, got: {}",
        geom_steps[0]["detail"]
    );

    // 4. `fvid::native_plan::transcode_lossless` with `overlay` on MP4 must also validate
    //    post-compositing `filters` (such as oversized `boxblur` radius `lr=100` on 32x16 MP4).
    let bad_mp4_overlay = LosslessTransform {
        overlay: Some(fvid::media_info::OverlaySpec {
            path: source_mp4.clone(),
            x: 0,
            y: 0,
        }),
        boxblur: Some("lr=100".into()),
        ..Default::default()
    };
    let mp4_plan_err = fvid::native_plan::transcode_lossless(&source_mp4, &bad_mp4_overlay)
        .expect_err("native_plan::transcode_lossless with overlay on MP4 must reject oversized boxblur radius");
    assert!(
        mp4_plan_err.contains("boxblur"),
        "unexpected MP4 overlay filter validation error: {mp4_plan_err}"
    );

    let _ = std::fs::remove_dir_all(&temp);
}



#[test]
fn shuffleplanes_promotion_decode_pixel_format_export_y4m_budget_and_crop_lossless_rotate_rejection() {
    let source_y4m = fixture(); // 8x4, 4:2:0 (64 + 16 + 16 = 96 bytes/frame; 4:4:4 is 64 * 3 = 192 bytes/frame)
    let source_mp4 = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/shared-mp4-av-multiple.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-shuffleplanes-crop-rotate-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. Cross-plane `shuffleplanes` ("1:2:0") on 4:2:0 Y4M and 4:2:0 MP4 promotes the output frame
    //    subsampling to 4:4:4 (`Some([1, 1])`). Both `fvid::native_media::decode_video_request`
    //    and `fvid media decode` must report `pixel_format: "yuv444p"` (matching `transcode_lossless`),
    //    not `"yuv420p"`.
    for (label, src) in [("y4m", &source_y4m), ("mp4", &source_mp4)] {
        let decode_req = fvid::media_info::DecodeTransform {
            shuffleplanes: Some("1:2:0".into()),
            ..Default::default()
        };
        let decode_stats = fvid::native_media::decode_video_request(src, &decode_req)
            .unwrap_or_else(|e| panic!("{label}: decode_video_request failed: {e}"));
        assert_eq!(
            decode_stats.pixel_format, "yuv444p",
            "{label}: decode_video_request with cross-plane shuffleplanes must report yuv444p"
        );

        let cli_decode = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args([
                "media",
                "decode",
                src.to_str().unwrap(),
                "--shuffleplanes",
                "1:2:0",
            ])
            .output()
            .unwrap();
        assert!(
            cli_decode.status.success(),
            "{label}: cli decode --shuffleplanes 1:2:0 failed: {}",
            String::from_utf8_lossy(&cli_decode.stderr)
        );
        let cli_json: serde_json::Value = serde_json::from_slice(&cli_decode.stdout).unwrap();
        assert_eq!(
            cli_json["pixel_format"], "yuv444p",
            "{label}: cli decode with cross-plane shuffleplanes must report yuv444p: {cli_json}"
        );
    }

    // 2. `export_y4m_pipeline_with_options` (and `fvid media export-y4m`) with `--shuffleplanes "1:2:0"`
    //    on 4:2:0 Y4M (raw input frame = 48 bytes, promoted 4:4:4 Y4M output frame = 96 bytes)
    //    and `--max-packet-bytes 60` must fail in `validate_video_packet_budget_with_geometry_filters_interval_and_limit`
    //    before emitting any progress packets (`packets == 0`).
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut opts = fvid::media::CopyOptions::default();
    opts.max_packet_bytes = 60;
    opts.progress = Some(fvid::media_control::ProgressHook::new(move |ev| {
        events_clone.lock().unwrap().push(ev);
    }));
    let filters = fvid::native_pixels::PixelFilters::from_request(&fvid::media_info::DecodeTransform {
        shuffleplanes: Some("1:2:0".into()),
        ..Default::default()
    })
    .unwrap();
    let out_y4m = temp.join("promoted-budget.y4m");
    let err = fvid::native_export::export_y4m_pipeline_with_options(
        &source_y4m,
        &out_y4m,
        None,
        &Default::default(),
        &filters,
        &opts,
    )
    .expect_err("export_y4m_pipeline_with_options must reject 96-byte promoted 4:4:4 Y4M frame under 60-byte limit");
    assert!(
        err.to_string().contains("Y4M frame exceeds byte limit"),
        "unexpected error: {err}"
    );
    assert!(!out_y4m.exists());
    let recorded = events.lock().unwrap();
    assert!(
        recorded.iter().all(|ev| ev.packets == 0 && !ev.done),
        "expected no progress packets emitted before preflight rejection, got {recorded:?}"
    );

    // 3. `fvid media crop-lossless` with `--crop` AND `--rotate` alongside `--max-packets 10`
    //    (or an unrecognized flag so `owned_lossless_command` falls through to line 683)
    //    must reject `--rotate` with `"crop-lossless does not take --rotate; use transcode-lossless"`.
    let out_crop_rot = temp.join("crop-rotate.mkv");
    for extra in [
        vec!["--crop", "0:0:4:2", "--rotate", "90"],
        vec!["--crop", "0:0:4:2", "--rotate", "90", "--max-packets", "10"],
        vec!["--crop", "0:0:4:2", "--rotate", "90", "--epx", "2"],
    ] {
        let mut cmd = vec![
            "media",
            "crop-lossless",
            source_y4m.to_str().unwrap(),
            out_crop_rot.to_str().unwrap(),
        ];
        cmd.extend(extra.iter().copied());
        let res = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&cmd)
            .output()
            .unwrap();
        assert!(
            !res.status.success(),
            "crop-lossless with {extra:?} must fail"
        );
        let stderr = String::from_utf8_lossy(&res.stderr);
        assert!(
            stderr.contains("crop-lossless does not take --rotate; use transcode-lossless"),
            "unexpected crop-lossless --rotate error for {extra:?}: {stderr}"
        );
        assert!(!out_crop_rot.exists());
    }

    let _ = std::fs::remove_dir_all(&temp);
}



#[test]
fn y4m_plan_transcode_lossless_with_metadata_or_stream_options_validates_first_frame_and_default_fps() {
    let source_y4m = fixture();
    let default_fps_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/default-fps-8x4.y4m");

    // 1. Y4M source without an explicit `F` token (`default-fps-8x4.y4m`, which takes the
    //    `fvid_media::plan_transcode_lossless` route) must still validate first-frame spatial
    //    geometry (`crop`, `pad`, `scale`) and pixel filter dimensions (`boxblur`) even when
    //    called with default `CopyOptions`!
    let default_opts = fvid::media::CopyOptions::default();
    let bad_crop_default = LosslessTransform {
        crop: Some(fvid::media_info::CropRect {
            x: 0,
            y: 0,
            width: 99,
            height: 99,
        }),
        ..Default::default()
    };
    let default_fps_crop_err =
        fvid::media::plan_transcode_lossless(&default_fps_y4m, &bad_crop_default, &default_opts, None)
            .expect_err("plan_transcode_lossless on default-fps Y4M must reject out-of-bounds crop");
    assert!(
        default_fps_crop_err.contains("crop"),
        "unexpected error for out-of-bounds crop on default-fps Y4M: {default_fps_crop_err}"
    );

    // 2. When `fvid::media::plan_transcode_lossless` (or `fvid_media::plan_transcode_lossless`)
    //    is called with metadata options (`--metadata title=test`) or `--streams 0` and default
    //    packet limits (`max_packets: None`, default `max_packet_bytes`), it must still validate
    //    first-frame spatial geometry (`crop`, `pad`, `scale`) and pixel filter dimensions (`boxblur`)
    //    instead of only reading two lines from the Y4M file!
    let mut meta_opts = fvid::media::CopyOptions::default();
    meta_opts.metadata_set.push(("title".into(), "test".into()));

    let bad_crop = LosslessTransform {
        crop: Some(fvid::media_info::CropRect {
            x: 0,
            y: 0,
            width: 99,
            height: 99,
        }),
        ..Default::default()
    };
    let crop_err = fvid::media::plan_transcode_lossless(&source_y4m, &bad_crop, &meta_opts, None)
        .expect_err("plan_transcode_lossless with metadata options must reject out-of-bounds Y4M crop");
    assert!(
        crop_err.contains("crop"),
        "unexpected error for out-of-bounds Y4M crop with metadata options: {crop_err}"
    );

    let bad_blur = LosslessTransform {
        boxblur: Some("lr=100".into()),
        ..Default::default()
    };
    let blur_err = fvid::media::plan_transcode_lossless(&source_y4m, &bad_blur, &meta_opts, None)
        .expect_err("plan_transcode_lossless with metadata options must reject oversized boxblur radius on Y4M");
    assert!(
        blur_err.contains("boxblur"),
        "unexpected error for oversized Y4M boxblur with metadata options: {blur_err}"
    );

    // 3. `fvid_media::owned_y4m_decode::decode_video_transformed` with `unsharp` (when no other
    //    filter is set) must succeed instead of failing with `"unsharp requires persistent streaming history"`.
    let unsharp_decode = fvid::media_info::DecodeTransform {
        unsharp: Some("lx=3:ly=3:la=0.5".into()),
        ..Default::default()
    };
    let unsharp_stats =
        fvid_media::owned_y4m_decode::decode_video_transformed(&source_y4m, unsharp_decode)
            .expect("owned_y4m_decode::decode_video_transformed with unsharp must succeed");
    assert_eq!(unsharp_stats.video_frames, 1);

    let unsharp_lossless = LosslessTransform {
        unsharp: Some("lx=3:ly=3:la=0.5".into()),
        ..Default::default()
    };
    let unsharp_plan =
        fvid::media::plan_transcode_lossless(&source_y4m, &unsharp_lossless, &meta_opts, None)
            .expect("plan_transcode_lossless with unsharp and metadata options must succeed");
    assert_eq!(unsharp_plan.command, "transcode-lossless");
}

#[test]
fn cli_overlay_rejects_duplicate_and_unknown_options_and_plan_transcode_rejects_encoder_flags() {
    let source_y4m = fixture();
    let overlay_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/overlay-4x2.y4m");
    let temp = std::env::temp_dir().join(format!("fvid-overlay-cli-flags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let out_mkv = temp.join("never.mkv");

    // 1. `fvid media plan transcode` and `fvid media plan` must reject `--encoder` and `--encoder-option`
    //    ("plan does not take metadata mutation or encoder selection") in both default builds and `--features media`.
    for args in [
        vec![
            "media",
            "plan",
            "transcode",
            source_y4m.to_str().unwrap(),
            "--encoder",
            "ffv1",
        ],
        vec![
            "media",
            "plan",
            source_y4m.to_str().unwrap(),
            "--encoder",
            "ffv1",
        ],
        vec![
            "media",
            "plan",
            source_y4m.to_str().unwrap(),
            "--encoder-option",
            "crf=18",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for {args:?}, got stdout: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("plan does not take metadata mutation or encoder selection"),
            "unexpected error for {args:?}: {stderr}"
        );
    }

    // 2. `fvid media overlay` and `fvid media plan overlay` must reject duplicate `--overlay`,
    //    `--overlay-x`, `--overlay-y`, and unknown `--bogus` options instead of silently
    //    overwriting or falling through to `this media operation requires cargo build --release --features media`.
    for (args, expected_substr) in [
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
                "--overlay",
                overlay_y4m.to_str().unwrap(),
            ],
            "duplicate",
        ),
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                "--overlay-x",
                "0",
                "--overlay-x",
                "2",
            ],
            "duplicate",
        ),
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                "--overlay-y",
                "0",
                "--overlay-y",
                "2",
            ],
            "duplicate",
        ),
        (
            vec![
                "media",
                "plan",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                "--bogus-flag",
            ],
            "unsupported",
        ),
        (
            vec![
                "media",
                "overlay",
                source_y4m.to_str().unwrap(),
                overlay_y4m.to_str().unwrap(),
                out_mkv.to_str().unwrap(),
                "--bogus-flag",
            ],
            "unsupported",
        ),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "expected failure for {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(expected_substr)
                && !stderr.contains("requires cargo build --release --features media"),
            "unexpected error for {args:?}: {stderr}"
        );
    }

    // 3. `fvid::media::plan_overlay` and `fvid::media::overlay_video` must reject out-of-range
    //    `overlay_x` / `overlay_y` coordinates (`i64::from(i32::MAX) + 1` is not needed since `x: i32`,
    //    but out-of-canvas coordinates or negative coordinates on Y4M must fail consistently across
    //    default options and metadata options).
    let mut meta_opts = fvid::media::CopyOptions::default();
    meta_opts.metadata_set.push(("title".into(), "OverlayTitle".into()));
    assert!(
        fvid::media::plan_overlay(&source_y4m, &overlay_y4m, 100, 0, &Default::default()).is_err()
    );
    assert!(fvid::media::plan_overlay(&source_y4m, &overlay_y4m, 100, 0, &meta_opts).is_err());

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn cli_plan_transcode_lossless_require_two_paths_and_plan_commands_ignore_progress() {
    let source_y4m = fixture(); // 8x4 Y4M, 3 frames
    let zero_y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/zero-frames-8x4.y4m");
    let source_wav = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudnorm-feedback.wav");
    let source_srt = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn/caption.srt");

    let temp = std::env::temp_dir().join(format!(
        "fvid-cli-plan-parity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let out_mkv = temp.join("out.mkv");

    // 0. Companion 0-frame fixture refusal check.
    assert!(
        fvid::media::plan_transcode_lossless(
            &zero_y4m,
            &LosslessTransform::default(),
            &Default::default(),
            None,
        )
        .is_err()
    );

    // 1. `fvid media transcode-lossless INPUT --quiet` (only 1 path, no destination) must fail
    //    instead of parsing `--quiet` as destination path `args[2]` and creating a file named `--quiet`!
    //    Likewise, passing an extra positional path (`INPUT OUTPUT EXTRA`) to `transcode-lossless`,
    //    `burn-subtitles`, or `xfade` must be rejected with a clear error instead of silently ignoring
    //    `EXTRA` or falling through to `requires cargo build --release --features media`.
    for args in [
        vec![
            "media",
            "transcode-lossless",
            source_y4m.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "crop-lossless",
            source_y4m.to_str().unwrap(),
            "--crop",
            "0:0:4:4",
            "--quiet",
        ],
        vec![
            "media",
            "transcode-lossless",
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "transcode-lossless",
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "burn-subtitles",
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            "--subs",
            source_srt.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "burn-subtitles",
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            "--subs",
            source_srt.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "xfade",
            source_y4m.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            "--xfade-duration",
            "0.04",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "xfade",
            source_y4m.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            "--xfade-duration",
            "0.04",
            "--quiet",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "expected failure for {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate error for {args:?}: {stderr}"
        );
    }

    // 2. `fvid media plan remux INPUT --progress`, `fvid media plan normalize-loudness INPUT --progress`,
    //    `fvid media plan trim INPUT --from 0 --to 0.2 --progress`, and `fvid media plan trim-pcm INPUT --from 0 --to 0.2 --progress`
    //    must accept and ignore `--progress` during planning (matching `run_native`, `plan loudness`, `plan loudnorm`,
    //    `plan decode-audio`, `plan transcode-lossless`, `plan overlay`, `plan xfade`, `plan burn-subtitles`, and `plan concat`).
    for args in [
        vec![
            "media",
            "plan",
            "remux",
            source_wav.to_str().unwrap(),
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "normalize-loudness",
            source_wav.to_str().unwrap(),
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "trim",
            source_wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.2",
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "trim-pcm",
            source_wav.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.2",
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "mix-audio",
            source_wav.to_str().unwrap(),
            source_wav.to_str().unwrap(),
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            source_wav.to_str().unwrap(),
            source_wav.to_str().unwrap(),
            "--progress",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "concat",
            source_wav.to_str().unwrap(),
            source_wav.to_str().unwrap(),
            "--progress",
            "--quiet",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected {args:?} to succeed, stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn export_y4m_and_owned_video_cli_validate_extensions_and_flag_ordering() {
    let source_y4m = fixture();
    let source_srt = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn/caption.srt");
    let temp = std::env::temp_dir().join(format!(
        "fvid-export-y4m-owned-video-cli-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let out_y4m = temp.join("out.y4m");
    let out_mkv = temp.join("out.mkv");

    // 1. `fvid::native_export::export_y4m_pipeline_with_options` and `fvid media export-y4m`
    //    must reject non-`.y4m` destination paths (such as `out.mkv`).
    let bad_ext_err = fvid::native_export::export_y4m_pipeline_with_options(
        &source_y4m,
        &out_mkv,
        None,
        &Default::default(),
        &Default::default(),
        &fvid::media::CopyOptions::default(),
    )
    .expect_err("export_y4m_pipeline_with_options must reject non-.y4m destination");
    assert!(
        bad_ext_err.to_string().contains(".y4m"),
        "unexpected error for non-.y4m export destination: {bad_ext_err}"
    );
    assert!(
        !out_mkv.exists(),
        "rejected non-.y4m export must not create destination file"
    );

    // 2. `fvid media export-y4m`, `transcode-lossless`, `crop-lossless`, `xfade`, `burn-subtitles`,
    //    and their `plan` subcommands must accept leading/interleaved flags (such as `--quiet`).
    for args in [
        vec![
            "media",
            "export-y4m",
            "--quiet",
            "--boxblur",
            "1:1",
            source_y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "transcode-lossless",
            "--quiet",
            "--hflip",
            source_y4m.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "crop-lossless",
            "--quiet",
            "--crop",
            "0:0:4:4",
            source_y4m.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "xfade",
            "--quiet",
            "--xfade-duration",
            "0.04",
            source_y4m.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "burn-subtitles",
            "--quiet",
            "--subs",
            source_srt.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected {args:?} to succeed with leading flags, stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_file(&out_y4m);

    // 3. Missing positional paths, extra positional paths, non-.y4m export destinations, and
    //    unknown flags on `export-y4m`, `transcode-lossless`, `crop-lossless`, `xfade`, and
    //    `burn-subtitles` (plus `plan` variants) must fail with direct diagnostics instead of
    //    falling through to `requires cargo build --release --features media`.
    for args in [
        vec![
            "media",
            "export-y4m",
            source_y4m.to_str().unwrap(),
            out_mkv.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "export-y4m",
            source_y4m.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "export-y4m",
            source_y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            "--quiet",
        ],
        vec!["media", "transcode-lossless"],
        vec!["media", "transcode-lossless", source_y4m.to_str().unwrap()],
        vec!["media", "crop-lossless"],
        vec!["media", "plan", "transcode-lossless"],
        vec!["media", "plan", "crop-lossless"],
        vec![
            "media",
            "plan",
            "crop-lossless",
            source_y4m.to_str().unwrap(),
            "--crop",
            "0:0:4:4",
            "--bogus-flag",
        ],
        vec!["media", "xfade"],
        vec!["media", "xfade", source_y4m.to_str().unwrap()],
        vec!["media", "plan", "xfade"],
        vec!["media", "plan", "xfade", source_y4m.to_str().unwrap()],
        vec![
            "media",
            "plan",
            "xfade",
            source_y4m.to_str().unwrap(),
            source_y4m.to_str().unwrap(),
            "--xfade-duration",
            "0.04",
            "--bogus-flag",
        ],
        vec!["media", "burn-subtitles"],
        vec!["media", "burn-subtitles", source_y4m.to_str().unwrap()],
        vec!["media", "plan", "burn-subtitles"],
        vec![
            "media",
            "plan",
            "burn-subtitles",
            source_y4m.to_str().unwrap(),
            "--subs",
            source_srt.to_str().unwrap(),
            "--bogus-flag",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "expected failure for {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate error for {args:?}: {stderr}"
        );
    }

    let _ = std::fs::remove_dir_all(&temp);
}





