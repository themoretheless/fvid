use std::{path::PathBuf, process::Command};
#[test]
fn cli_xfade_runs_without_libav_and_retains_secondary_tail() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let output = std::env::temp_dir().join(format!(
        "fvid-xfade-cli-{}-{}.mkv",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(output.clone());
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg("media")
        .arg("xfade")
        .arg(fixtures.join("xfade-primary.y4m"))
        .arg(fixtures.join("xfade-secondary.y4m"))
        .arg(&output)
        .args([
            "--xfade-duration",
            "0.5",
            "--xfade-offset",
            "0.5",
            "--progress",
            "--max-packets",
            "5",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "owned Y4M cross-fade FFV1 export");
    assert_eq!(stats["video_frames"], 4);
    assert_eq!(stats["decoded_frames"], 5);
    assert!(String::from_utf8_lossy(&result.stderr).contains("\"done\":true"));
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(output).unwrap(),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 4);
}
