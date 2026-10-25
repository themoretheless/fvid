use fvid::media::{self, CopyOptions, CropRect, LosslessTransform};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-overlay-multitrack-budget-parity")
}

fn fvid_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn y4m_lossless_crop_and_overlay_enforce_input_packet_bytes_across_plan_and_execution() {
    let dir = fixture_dir();
    let primary = dir.join("lossless-primary.y4m");
    let foreground = dir.join("lossless-foreground.y4m");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch245-y4m-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Primary Y4M frame is 96x64 YUV420 (9216 bytes), while compressed FFV1 packet is < 100 bytes.
    // With max_packet_bytes = 4096, plan_transcode_lossless, transcode_lossless, crop_lossless,
    // plan_overlay, and overlay_video must all reject the oversized Y4M input frame.
    let mut pkt4096 = CopyOptions::default();
    pkt4096.max_packet_bytes = 4096;

    let plan_err = media::plan_transcode_lossless(
        &primary,
        &LosslessTransform::default(),
        &pkt4096,
        None,
    )
    .unwrap_err();
    assert!(
        plan_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "plan_transcode_lossless pkt4096 returned: {plan_err}"
    );

    let out_transcode = temp_dir.join("transcode-pkt4096.mkv");
    let exec_err = media::transcode_lossless(
        &primary,
        &out_transcode,
        LosslessTransform::default(),
        &pkt4096,
    )
    .unwrap_err();
    assert_eq!(exec_err, plan_err);
    assert!(!out_transcode.exists());

    // crop_lossless with a 16x16 crop (cropped output is 384 bytes < 4096, but source Y4M frame is 9216 bytes > 4096)
    let crop = CropRect {
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    };
    let crop_transform = LosslessTransform {
        crop: Some(crop),
        ..Default::default()
    };
    let crop_plan_err =
        media::plan_transcode_lossless(&primary, &crop_transform, &pkt4096, None).unwrap_err();
    assert!(
        crop_plan_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "plan_transcode_lossless (crop) pkt4096 returned: {crop_plan_err}"
    );
    let out_crop = temp_dir.join("crop-pkt4096.mkv");
    let crop_exec_err =
        media::crop_lossless(&primary, &out_crop, crop, &pkt4096).unwrap_err();
    assert_eq!(crop_exec_err, crop_plan_err);
    assert!(!out_crop.exists());

    // plan_overlay and overlay_video with max_packet_bytes = 4096
    let ov_plan_err = media::plan_overlay(&primary, &foreground, 0, 0, &pkt4096).unwrap_err();
    assert!(
        ov_plan_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "plan_overlay pkt4096 returned: {ov_plan_err}"
    );
    let out_ov = temp_dir.join("overlay-pkt4096.mkv");
    let ov_exec_err =
        media::overlay_video(&primary, &foreground, &out_ov, 0, 0, &pkt4096).unwrap_err();
    assert_eq!(ov_exec_err, ov_plan_err);
    assert!(!out_ov.exists());

    // 2. Foreground Y4M frame is 32x32 YUV420 (1536 bytes).
    // When foreground (1536 bytes) is overlaid onto foreground (1536 bytes) with max_packet_bytes = 1024,
    // or when primary is cropped to 32x32 (1536 bytes) and overlaid with foreground (1536 bytes) with
    // max_packet_bytes = 10000 on primary and foreground > limit: test foreground frame_len > max_packet_bytes
    // by overlaying primary (9216 bytes) onto foreground (1536 bytes) with max_packet_bytes = 4096!
    // Here main (foreground, 1536 bytes) <= 4096, while overlay (primary, 9216 bytes) > 4096!
    let rev_ov_plan_err = media::plan_overlay(&foreground, &primary, 0, 0, &pkt4096).unwrap_err();
    assert!(
        rev_ov_plan_err.contains("Y4M frame byte length exceeds packet byte limit"),
        "plan_overlay (oversized foreground) returned: {rev_ov_plan_err}"
    );
    let out_rev_ov = temp_dir.join("rev-overlay-pkt4096.mkv");
    let rev_ov_exec_err =
        media::overlay_video(&foreground, &primary, &out_rev_ov, 0, 0, &pkt4096).unwrap_err();
    assert_eq!(rev_ov_exec_err, rev_ov_plan_err);
    assert!(!out_rev_ov.exists());

    // With max_packet_bytes = 16384, overlay succeeds
    let mut pkt16k = CopyOptions::default();
    pkt16k.max_packet_bytes = 16384;
    let ok_plan = media::plan_overlay(&primary, &foreground, 0, 0, &pkt16k).unwrap();
    assert_eq!(ok_plan.command, "overlay");
    let stats = media::overlay_video(&primary, &foreground, &out_ov, 0, 0, &pkt16k).unwrap();
    assert_eq!(stats.video_frames, 2);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_lossless_planning_enforces_companion_packet_bytes() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch245-multitrack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. multitrack-ffv1-aac.mkv has ~20-byte FFV1 video packets and 512-byte AAC audio packets in Cluster 0.
    // With max_packet_bytes = 256, both plan_transcode_lossless and transcode_lossless must reject the
    // 512-byte companion audio packet during container indexing.
    let mut pkt256 = CopyOptions::default();
    pkt256.max_packet_bytes = 256;
    let mkv_plan_err = media::plan_transcode_lossless(
        &mkv,
        &LosslessTransform::default(),
        &pkt256,
        None,
    )
    .unwrap_err();
    assert!(
        mkv_plan_err.contains("WebM packet exceeds budget"),
        "plan_transcode_lossless on multitrack MKV pkt256 returned: {mkv_plan_err}"
    );
    let out_mkv = temp_dir.join("mkv-pkt256.mkv");
    let mkv_exec_err = media::transcode_lossless(
        &mkv,
        &out_mkv,
        LosslessTransform::default(),
        &pkt256,
    )
    .unwrap_err();
    assert_eq!(mkv_exec_err, mkv_plan_err);
    assert!(!out_mkv.exists());

    // With default options, multitrack-ffv1-aac.mkv plans and exports cleanly
    let ok_mkv_plan = media::plan_transcode_lossless(
        &mkv,
        &LosslessTransform::default(),
        &CopyOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(ok_mkv_plan.streams.len(), 2);
    let mkv_stats = media::transcode_lossless(
        &mkv,
        &out_mkv,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(mkv_stats.video_frames, 2);
    assert_eq!(mkv_stats.copied_packets, 2);
    let _ = std::fs::remove_file(&out_mkv);

    // 2. multitrack-avc-aac.mp4 has ~778-byte AVC video packets and 2048-byte AAC audio packets.
    // With max_packet_bytes = 1024, video packets fit within 1024 bytes, but companion AAC packets (2048 bytes)
    // exceed max_packet_bytes! Both plan_transcode_lossless and transcode_lossless must reject it.
    let mut pkt1024 = CopyOptions::default();
    pkt1024.max_packet_bytes = 1024;
    let mp4_plan_err = media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &pkt1024,
        None,
    )
    .unwrap_err();
    assert!(
        mp4_plan_err.contains("MP4 packet exceeds budget"),
        "plan_transcode_lossless on multitrack MP4 pkt1024 returned: {mp4_plan_err}"
    );
    let out_mp4 = temp_dir.join("mp4-pkt1024.mkv");
    let mp4_exec_err = media::transcode_lossless(
        &mp4,
        &out_mp4,
        LosslessTransform::default(),
        &pkt1024,
    )
    .unwrap_err();
    assert_eq!(mp4_exec_err, mp4_plan_err);
    assert!(!out_mp4.exists());

    // When only video stream 0 is selected (`--streams 0`), the 2048-byte AAC track 1 is not selected,
    // so both plan_transcode_lossless and transcode_lossless succeed with max_packet_bytes = 1024!
    let mut pkt1024_video_only = pkt1024.clone();
    pkt1024_video_only.streams = vec![0];
    let ok_mp4_plan = media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &pkt1024_video_only,
        None,
    )
    .unwrap();
    assert_eq!(ok_mp4_plan.streams.len(), 1);
    let mp4_stats = media::transcode_lossless(
        &mp4,
        &out_mp4,
        LosslessTransform::default(),
        &pkt1024_video_only,
    )
    .unwrap();
    assert_eq!(mp4_stats.video_frames, 2);
    assert_eq!(mp4_stats.copied_packets, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn cli_lossless_crop_and_overlay_preserve_policy_and_memory_budget_refusals() {
    let dir = fixture_dir();
    let primary = dir.join("lossless-primary.y4m");
    let foreground = dir.join("lossless-foreground.y4m");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch245-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. CLI `transcode-lossless`, `crop-lossless`, and `overlay` with `--streams 1` or `--max-memory-mib 16`
    // must report the owned lossless policy refusal instead of falling through to
    // "this media operation requires cargo build --release --features media".
    for (flag, val) in [("--streams", "1"), ("--max-memory-mib", "16")] {
        for (plan_args, exec_args) in [
            (
                vec![
                    "media",
                    "plan",
                    "transcode-lossless",
                    primary.to_str().unwrap(),
                    flag,
                    val,
                ],
                vec![
                    "media",
                    "transcode-lossless",
                    primary.to_str().unwrap(),
                    temp_dir.join("tl.mkv").to_str().unwrap(),
                    flag,
                    val,
                ],
            ),
            (
                vec![
                    "media",
                    "plan",
                    "crop-lossless",
                    primary.to_str().unwrap(),
                    "--crop",
                    "0:0:16:16",
                    flag,
                    val,
                ],
                vec![
                    "media",
                    "crop-lossless",
                    primary.to_str().unwrap(),
                    temp_dir.join("cl.mkv").to_str().unwrap(),
                    "--crop",
                    "0:0:16:16",
                    flag,
                    val,
                ],
            ),
            (
                vec![
                    "media",
                    "plan",
                    "overlay",
                    primary.to_str().unwrap(),
                    foreground.to_str().unwrap(),
                    flag,
                    val,
                ],
                vec![
                    "media",
                    "overlay",
                    primary.to_str().unwrap(),
                    foreground.to_str().unwrap(),
                    temp_dir.join("ov.mkv").to_str().unwrap(),
                    flag,
                    val,
                ],
            ),
            (
                vec![
                    "media",
                    "plan",
                    "overlay",
                    primary.to_str().unwrap(),
                    foreground.to_str().unwrap(),
                    "--hflip",
                    flag,
                    val,
                ],
                vec![
                    "media",
                    "overlay",
                    primary.to_str().unwrap(),
                    foreground.to_str().unwrap(),
                    temp_dir.join("ov-hflip.mkv").to_str().unwrap(),
                    "--hflip",
                    flag,
                    val,
                ],
            ),
        ] {
            let p_out = Command::new(fvid_bin()).args(&plan_args).output().unwrap();
            assert!(!p_out.status.success(), "expected failure for {plan_args:?}");
            let p_stderr = String::from_utf8_lossy(&p_out.stderr);
            assert!(
                !p_stderr.contains("requires cargo build --release --features media"),
                "CLI plan fell through for {plan_args:?}: {p_stderr}"
            );
            assert!(
                p_stderr.contains("owned Y4M lossless export does not yet implement requested policy")
                    || p_stderr.contains("request has no owned lossless export plan"),
                "unexpected CLI plan stderr for {plan_args:?}: {p_stderr}"
            );

            let e_out = Command::new(fvid_bin()).args(&exec_args).output().unwrap();
            assert!(!e_out.status.success(), "expected failure for {exec_args:?}");
            let e_stderr = String::from_utf8_lossy(&e_out.stderr);
            assert!(
                !e_stderr.contains("requires cargo build --release --features media"),
                "CLI exec fell through for {exec_args:?}: {e_stderr}"
            );
            assert!(
                e_stderr.contains("owned Y4M lossless export does not yet implement requested policy"),
                "unexpected CLI exec stderr for {exec_args:?}: {e_stderr}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
