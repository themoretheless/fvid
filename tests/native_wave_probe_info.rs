#[test]
fn wave_probe_retains_info_tags_through_shared_library_parser() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let source = root.join("wave-probe-info.wav");
    let video = fvid::native_probe::probe(&root.join("wave-probe-info.y4m")).unwrap();
    assert_eq!(video.duration_us, Some(100000));
    let expected = fvid_media::owned_probe::probe(&source).unwrap();
    assert_eq!(
        expected.metadata.get("title").unwrap(),
        "Synthetic probe regression"
    );
    assert_eq!(expected.duration_us, Some(100000));
    assert_eq!(expected.streams[0].duration, Some(4800));
    let actual = fvid::native_probe::probe(&source).unwrap();
    assert_eq!(actual.metadata.get("title"), expected.metadata.get("title"));
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "probe"])
        .arg(&source)
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
