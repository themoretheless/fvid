use std::{path::Path, process::Command};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4")
}
#[test]
fn plain_decode_is_available_without_the_media_adapter() {
    let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(fixture())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    #[cfg(not(feature = "videotoolbox"))]
    assert!(text.contains("\"backend\":\"fvid\""), "{text}");
    assert!(text.contains("\"video_frames\":25"), "{text}");
    assert!(text.contains("\"width\":320"), "{text}");
    assert!(text.contains("\"height\":240"), "{text}");
}
#[test]
fn quiet_and_double_dash_preserve_command_semantics() {
    let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode", "--quiet", "--"])
        .arg(fixture())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
#[test]
fn unknown_options_are_not_ignored_by_native_dispatch() {
    let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode", "--not-a-real-option"])
        .arg(fixture())
        .output()
        .unwrap();
    assert!(!output.status.success());
}
