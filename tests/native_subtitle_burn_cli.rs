use std::{path::PathBuf, process::Command};
#[test]
fn native_burn_and_plan_run_without_external_tools() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let output = std::env::temp_dir().join(format!("fvid-burn-cli-{}.mkv", std::process::id()));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
    let _cleanup = Cleanup(output.clone());
    for planning in [true, false] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fvid"));
        command.arg("media");
        if planning { command.arg("plan"); }
        command.arg("burn-subtitles").arg(root.join("subtitle-burn.y4m"));
        if !planning { command.arg(&output); }
        let result = command.arg("--subs").arg(root.join("subtitle-burn.srt"))
            .env("PATH", "/nonexistent").output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        if planning {
            assert!(value["graph"].is_null());
            assert_eq!(value["command"], "burn-subtitles");
            assert!(!output.exists());
        } else {
            assert_eq!(value["video_frames"], 3);
            assert_eq!(value["backend"], "owned SRT burn-in FFV1 export");
        }
    }
}
