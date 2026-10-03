#[test]
fn migrated_filters_are_reported_by_owned_api_and_cli() {
    let inventory = fvid::native_capabilities::inventory();
    for name in ["eq", "hue", "unsharp"] {
        assert!(inventory.filters.iter().any(|filter| filter == name));
    }
    assert!(inventory.filters.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(inventory.library_version.starts_with("FVid "));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "capabilities"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual, serde_json::to_value(inventory).unwrap());
}

#[test]
fn owned_lossless_plan_lists_migrated_filters_in_execution_order() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::LosslessTransform {
        eq: Some("brightness=0.06".into()),
        unsharp: Some("3:3:0.5".into()),
        hue: Some("h=90".into()),
        avgblur: Some("1:1".into()),
        negate: Some("".into()),
        ..Default::default()
    };
    let plan = fvid::native_plan::transcode_lossless(&source, &request).unwrap();
    let filters: Vec<_> = plan
        .steps
        .iter()
        .filter(|step| step.action == "filter")
        .map(|step| step.detail.split(';').next().unwrap())
        .collect();
    assert_eq!(
        filters,
        [
            "FVid eq=brightness=0.06",
            "FVid unsharp=3:3:0.5",
            "FVid hue=h=90",
            "FVid avgblur=1:1",
            "FVid negate="
        ]
    );
    assert!(plan.notes.iter().any(|note| note.contains("backend: fvid")));
    assert!(plan
        .steps
        .iter()
        .any(|step| step.action == "encode" && step.detail.contains("FFV1")));
}

#[test]
fn migrated_filter_cli_plan_matches_owned_api_without_legacy() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::LosslessTransform {
        eq: Some("brightness=0.06".into()),
        unsharp: Some("3:3:0.5".into()),
        hue: Some("h=90".into()),
        ..Default::default()
    };
    let expected = fvid::native_plan::transcode_lossless(&source, &request).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "transcode-lossless"])
        .arg(&source)
        .args([
            "--eq",
            "brightness=0.06",
            "--unsharp",
            "3:3:0.5",
            "--hue",
            "h=90",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn plain_lossless_cli_plan_is_also_owned() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let expected = fvid::native_plan::transcode_lossless(&source, &Default::default()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "transcode-lossless"])
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn existing_owned_filter_cli_plan_does_not_require_legacy() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = fvid::media_info::LosslessTransform {
        avgblur: Some("1:1".into()),
        negate: Some("".into()),
        chromashift: Some("cbh=1".into()),
        ..Default::default()
    };
    let expected = fvid::native_plan::transcode_lossless(&source, &request).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "transcode-lossless"])
        .arg(source)
        .args(["--chromashift", "cbh=1", "--negate", "", "--avgblur", "1:1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn geometry_and_filter_cli_plan_is_owned() {
    use fvid::media_info::{CropRect, LosslessTransform, PadRect, ScaleSize, TransposeMode};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let request = LosslessTransform {
        crop: Some(CropRect {
            x: 0,
            y: 0,
            width: 16,
            height: 16,
        }),
        horizontal_flip: true,
        vertical_flip: true,
        transpose: Some(TransposeMode::Clock),
        pad: Some(PadRect {
            width: 32,
            height: 32,
            x: 2,
            y: 2,
        }),
        scale: Some(ScaleSize {
            width: 16,
            height: 16,
        }),
        hue: Some("h=90".into()),
        ..Default::default()
    };
    let expected = fvid::native_plan::transcode_lossless(&source, &request).unwrap();
    let geometry = expected
        .steps
        .iter()
        .position(|step| step.action == "geometry")
        .unwrap();
    let filter = expected
        .steps
        .iter()
        .position(|step| step.action == "filter")
        .unwrap();
    assert!(geometry < filter);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "transcode-lossless"])
        .arg(source)
        .args([
            "--hue",
            "h=90",
            "--crop",
            "0:0:16:16",
            "--hflip",
            "--vflip",
            "--transpose",
            "clock",
            "--pad",
            "32:32:2:2",
            "--scale",
            "16:16",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn geometry_plan_rejects_invalid_fields_duplicates_and_overflow() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    for (options, error) in [
        (vec!["--crop", "0:16:16"], "crop requires X:Y:WIDTH:HEIGHT"),
        (vec!["--hflip", "--hflip"], "duplicate option: --hflip"),
        (vec!["--transpose", "invalid"], "transpose must be"),
        (vec!["--scale", "4294967296:16"], "out of range"),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "transcode-lossless"])
            .arg(&source)
            .args(options)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn owned_encoder_settings_admit_only_implemented_level() {
    for (name, options, accepted) in [
        ("ffv1", vec![], true),
        ("ffv1", vec![("level", "1")], true),
        ("ffv1", vec![("level", "3")], false),
        ("ffv1", vec![("level", "1"), ("coder", "1")], false),
        ("h264", vec![], false),
    ] {
        let settings = fvid::media_info::EncoderSettings {
            name: name.into(),
            options: options
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        };
        assert_eq!(
            fvid_media::owned_lossless::supports_encoder(&settings),
            accepted
        );
    }
}

#[test]
fn temporal_lossless_cli_plans_use_owned_export_backend() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/framestep-six-frames.y4m");
    for (option, value) in [
        ("--framestep", "2"),
        ("--reverse", ""),
        ("--shuffleframes", "2 1 0"),
    ] {
        let mut request = fvid::media_info::LosslessTransform::default();
        match option {
            "--framestep" => request.framestep = Some(value.into()),
            "--reverse" => request.reverse = Some(value.into()),
            _ => request.shuffleframes = Some(value.into()),
        }
        let expected = fvid_media::owned_lossless::plan_transcode_lossless(
            &source,
            &request,
            &Default::default(),
            None,
        )
        .unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "transcode-lossless"])
            .arg(&source)
            .args([option, value])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}
