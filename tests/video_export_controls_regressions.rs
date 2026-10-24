use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-video-export-controls-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reproduction_lossless_export_and_cli_controls_enforce_rss_and_packet_budgets() {
    let main = fixture("video-export-controls-main.y4m");
    let overlay = fixture("video-export-controls-overlay.y4m");
    let subs = fixture("video-export-controls.srt");
    let dir = temp_dir("repro");

    // 1. Library API: generous max_rss_bytes is accepted and exported; tiny max_rss_bytes is refused.
    let generous_rss = fvid::media::CopyOptions {
        max_rss_bytes: Some(16 << 30),
        metadata_set: vec![("title".into(), "Controlled Export".into())],
        ..Default::default()
    };
    let plan = fvid::media::plan_transcode_lossless(
        &main,
        &fvid::media::LosslessTransform::default(),
        &generous_rss,
        None,
    )
    .expect("plan_transcode_lossless should accept generous max_rss_bytes");
    assert_eq!(plan.command, "transcode-lossless");

    let out_lossless = dir.join("lossless-generous.mkv");
    let stats = fvid::media::transcode_lossless(
        &main,
        &out_lossless,
        fvid::media::LosslessTransform::default(),
        &generous_rss,
    )
    .expect("transcode_lossless should succeed under generous max_rss_bytes");
    assert_eq!(stats.video_frames, 3);
    assert_eq!(
        fvid::media::probe(&out_lossless)
            .unwrap()
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Controlled Export")
    );

    let tiny_rss = fvid::media::CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    let err = fvid::media::plan_transcode_lossless(
        &main,
        &fvid::media::LosslessTransform::default(),
        &tiny_rss,
        None,
    )
    .expect_err("tiny max_rss_bytes must fail planning");
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");

    let out_tiny_rss = dir.join("lossless-tiny-rss.mkv");
    let err = fvid::media::transcode_lossless(
        &main,
        &out_tiny_rss,
        fvid::media::LosslessTransform::default(),
        &tiny_rss,
    )
    .expect_err("tiny max_rss_bytes must fail export before publishing output");
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");
    assert!(!out_tiny_rss.exists());

    // 2. CLI transcode-lossless and crop-lossless accept --max-rss-mib, --max-packet-bytes, and metadata flags without legacy feature.
    let bin = env!("CARGO_BIN_EXE_fvid");
    let cli_lossless = dir.join("cli-lossless.mkv");
    let status = Command::new(bin)
        .args([
            "media",
            "transcode-lossless",
            main.to_str().unwrap(),
            cli_lossless.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-packet-bytes",
            "65536",
            "--metadata",
            "title=CLI Lossless",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli transcode-lossless failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        fvid::media::probe(&cli_lossless)
            .unwrap()
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Lossless")
    );

    let cli_crop = dir.join("cli-crop.mkv");
    let status = Command::new(bin)
        .args([
            "media",
            "crop-lossless",
            main.to_str().unwrap(),
            cli_crop.to_str().unwrap(),
            "--crop",
            "0:0:2:2",
            "--max-rss-mib",
            "4096",
            "--max-packet-bytes",
            "65536",
            "--metadata",
            "title=CLI Crop",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli crop-lossless failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let crop_info = fvid::media::probe(&cli_crop).unwrap();
    assert_eq!(crop_info.streams[0].width, 2);
    assert_eq!(crop_info.streams[0].height, 2);
    assert_eq!(crop_info.metadata.get("title").map(String::as_str), Some("CLI Crop"));

    // 3. CLI overlay and plan overlay accept --max-packets, --max-packet-bytes, --max-rss-mib, and --metadata.
    let plan_overlay = Command::new(bin)
        .args([
            "media",
            "plan",
            "overlay",
            main.to_str().unwrap(),
            overlay.to_str().unwrap(),
            "--max-packets",
            "10",
            "--max-packet-bytes",
            "65536",
            "--max-rss-mib",
            "4096",
            "--metadata",
            "title=CLI Overlay",
        ])
        .output()
        .unwrap();
    assert!(
        plan_overlay.status.success(),
        "cli plan overlay failed: {}",
        String::from_utf8_lossy(&plan_overlay.stderr)
    );
    let cli_overlay = dir.join("cli-overlay.mkv");
    let run_overlay = Command::new(bin)
        .args([
            "media",
            "overlay",
            main.to_str().unwrap(),
            overlay.to_str().unwrap(),
            cli_overlay.to_str().unwrap(),
            "--max-packets",
            "10",
            "--max-packet-bytes",
            "65536",
            "--max-rss-mib",
            "4096",
            "--metadata",
            "title=CLI Overlay",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        run_overlay.status.success(),
        "cli overlay failed: {}",
        String::from_utf8_lossy(&run_overlay.stderr)
    );
    assert_eq!(
        fvid::media::probe(&cli_overlay)
            .unwrap()
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Overlay")
    );

    // 4. CLI xfade and burn-subtitles enforce --max-packet-bytes and --max-rss-mib.
    let cli_xfade = dir.join("cli-xfade-small-packet.mkv");
    let xfade_small = Command::new(bin)
        .args([
            "media",
            "xfade",
            main.to_str().unwrap(),
            main.to_str().unwrap(),
            cli_xfade.to_str().unwrap(),
            "--xfade-duration",
            "0.04",
            "--xfade-offset",
            "0.04",
            "--max-packet-bytes",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!xfade_small.status.success());
    let stderr = String::from_utf8_lossy(&xfade_small.stderr);
    assert!(
        stderr.contains("packet exceeds byte limit"),
        "unexpected xfade error: {stderr}"
    );
    assert!(!cli_xfade.exists());

    let cli_burn = dir.join("cli-burn.mkv");
    let burn_ok = Command::new(bin)
        .args([
            "media",
            "burn-subtitles",
            main.to_str().unwrap(),
            cli_burn.to_str().unwrap(),
            "--subs",
            subs.to_str().unwrap(),
            "--max-rss-mib",
            "4096",
            "--max-packet-bytes",
            "65536",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        burn_ok.status.success(),
        "cli burn-subtitles failed: {}",
        String::from_utf8_lossy(&burn_ok.stderr)
    );
    assert!(cli_burn.exists());

    let cli_burn_small = dir.join("cli-burn-small-packet.mkv");
    let burn_small = Command::new(bin)
        .args([
            "media",
            "burn-subtitles",
            main.to_str().unwrap(),
            cli_burn_small.to_str().unwrap(),
            "--subs",
            subs.to_str().unwrap(),
            "--max-packet-bytes",
            "1",
        ])
        .output()
        .unwrap();
    assert!(!burn_small.status.success());
    let stderr = String::from_utf8_lossy(&burn_small.stderr);
    assert!(
        stderr.contains("packet exceeds byte limit"),
        "unexpected burn-subtitles error: {stderr}"
    );
    assert!(!cli_burn_small.exists());

    std::fs::remove_dir_all(dir).unwrap();
}
