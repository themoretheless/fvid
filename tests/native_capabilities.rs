use std::process::Command;

#[test]
fn cli_and_public_api_report_owned_components() {
    let inventory = fvid::native_capabilities::inventory();
    assert_eq!(
        inventory.library_version,
        format!("FVid {}", env!("CARGO_PKG_VERSION"))
    );
    for names in [
        &inventory.demuxers,
        &inventory.muxers,
        &inventory.decoders,
        &inventory.encoders,
        &inventory.filters,
    ] {
        assert!(names.windows(2).all(|p| p[0] < p[1]));
    }
    assert!(inventory.decoders.contains(&"h264".into()));
    assert!(inventory.decoders.contains(&"hevc".into()));
    assert!(inventory.decoders.contains(&"aac".into()));
    assert!(inventory.decoders.contains(&"opus".into()));
    assert!(!inventory.encoders.contains(&"libx264".into()));
    let expected = serde_json::to_value(&inventory).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "capabilities"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
        expected
    );
    #[cfg(feature = "media")]
    assert_eq!(
        serde_json::to_value(fvid::media::capabilities()).unwrap(),
        expected
    );
    let invalid = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "capabilities", "--unknown"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
}
