use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn owned_plans_match_cli_and_public_api_without_changing_inputs() {
    for name in [
        "audio/aac-mono-44k.aac",
        "avc/hlg-vui-only.mp4",
        "hevc/main10-ipb.mp4",
        "audio/aac-native-edit.m4a",
        "video.mp4",
    ] {
        let path = fixture(name);
        let before = std::fs::read(&path).unwrap();
        let plan = fvid::native_plan::remux(&path).unwrap().unwrap();
        assert_eq!(plan.command, "remux");
        assert!(plan.streams.iter().all(|s| s.disposition == "copy"));
        assert!(plan.notes.iter().any(|s| s.contains("backend: fvid")));
        let expected = serde_json::to_value(plan).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "remux"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            expected
        );
        #[cfg(feature = "media")]
        {
            assert_eq!(
                serde_json::to_value(fvid::media::plan_remux(&path, &Default::default()).unwrap())
                    .unwrap(),
                expected
            );
            let options = fvid::media::CopyOptions {
                streams: vec![0],
                ..Default::default()
            };
            assert!(fvid::media::plan_remux(&path, &options).is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "remux"])
            .arg(&path)
            .args(["--streams", "0"])
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}
#[test]
fn corrupt_known_input_is_an_error_and_unknown_is_distinct() {
    let path = std::env::temp_dir().join(format!("fvid-remux-plan-{}", std::process::id()));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    std::fs::write(&path, b"unknown").unwrap();
    assert!(fvid::native_plan::remux(&path).unwrap().is_none());
    std::fs::write(&path, b"\0\0\0\xffftyp").unwrap();
    assert!(fvid::native_plan::remux(&path).is_err());
    let mut aac = std::fs::read(fixture("audio/aac-mono-44k.aac")).unwrap();
    aac.pop();
    std::fs::write(&path, aac).unwrap();
    assert!(fvid::native_plan::remux(&path).is_err());
}
