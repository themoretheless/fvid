#![cfg(all(
    feature = "media-cuda",
    any(target_os = "windows", target_os = "linux")
))]

#[test]
#[ignore = "requires physical NVIDIA Main10 NVDEC/NVENC and CUDA runtime"]
fn synthetic_main10_cli_exports_complete_bitstreams_without_host_copies() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/cuda-hevc-main10-edit-repeat.mp4");
    let output = std::env::temp_dir().join(format!("fvid-cuda-cli-{}.mkv", std::process::id()));
    let export = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "hw-filter"])
        .arg(&source)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(stats["video_frames"], 14);
    assert_eq!(stats["host_frame_copies"], 0);
    let decode = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(&output)
        .output()
        .unwrap();
    std::fs::remove_file(&output).unwrap();
    assert!(
        decode.status.success(),
        "{}",
        String::from_utf8_lossy(&decode.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&decode.stdout).unwrap();
    assert_eq!(stats["video_frames"], 14);
    assert_eq!(stats["pixel_format"], "yuv420p10le");
    assert_eq!(stats["decode_errors"], 0);
}
