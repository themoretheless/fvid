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
