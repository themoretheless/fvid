#![cfg(not(feature = "media"))]
use std::process::Command;
#[test]
fn loudnorm_cli_exports_long_wave_and_reports_prefix_progress_without_legacy() {
    let dir = std::env::temp_dir().join(format!("fvid-native-loudnorm-cli-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudnorm-dynamic.wav");
    let output = dir.join("long.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&output)
        .args(["--max-memory-mib", "24"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid dynamic loudnorm");
    assert_eq!(stats["sample_frames"], 576000);
    let prefix = dir.join("prefix.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&prefix)
        .args([
            "--quiet",
            "--progress",
            "--max-packets",
            "1",
            "--streams",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let events: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.last().unwrap()["done"], true);
    assert_eq!(
        events.iter().filter(|event| event["done"] == true).count(),
        1
    );
    assert_eq!(events.last().unwrap()["packets"], 1);
    let mut file = std::fs::File::open(&prefix).unwrap();
    assert_eq!(
        fvid_media::owned_wave_inspect::inspect(&mut file, None)
            .unwrap()
            .sample_frames,
        4096
    );
    let invalid = dir.join("invalid.wav");
    let result = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "loudnorm"])
        .arg(&fixture)
        .arg(&invalid)
        .args(["--loudnorm-args", "I=-16", "--loudnorm-args", "I=-20"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!invalid.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate loudnorm option"));
    std::fs::remove_dir_all(dir).unwrap();
}
