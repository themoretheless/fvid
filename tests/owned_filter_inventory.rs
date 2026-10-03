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
