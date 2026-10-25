use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-xfade-subburn-budget-parity")
        .join(name)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-xfade-subburn-budget-parity-{}-{label}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn y4m_export_concat_and_trim_scope_packet_budget_to_selected_track_and_decoded_frame() {
    let mkv = fixture("multitrack-ffv1-aac.mkv");
    let mp4 = fixture("multitrack-avc-aac.mp4");
    let dir = temp_dir("selected-track-budget");

    // 1. Multitrack Matroska (track 0 = 4x4 FFV1 video ~25B packets, 24B decoded Y4M frame;
    //    track 1 = AAC audio 512B packets).
    // With `--streams 0` and `max_packet_bytes: 100`, the unselected 512B audio track must
    // NOT cause a false "WebM packet exceeds budget" failure in plan or execution!
    let mut mkv_ok = fvid::media::CopyOptions::default();
    mkv_ok.streams = vec![0];
    mkv_ok.max_packet_bytes = 100;

    let plan_trim_ok = fvid::media::plan_trim_y4m(&mkv, 0, 80_000, &mkv_ok)
        .expect("plan_trim_y4m must ignore unselected audio track packet size when --streams 0 is selected");
    assert_eq!(plan_trim_ok.command, "trim");

    let out_trim_mkv = dir.join("trim-selected.y4m");
    let trim_stats = fvid::media::trim(&mkv, &out_trim_mkv, 0, 80_000, &mkv_ok)
        .expect("trim to Y4M must succeed when selected video track and output Y4M frame fit within max_packet_bytes=100");
    assert_eq!(trim_stats.packets, 2);

    let plan_concat_ok = fvid::media::plan_concat_y4m(&[mkv.clone(), mkv.clone()], &mkv_ok)
        .expect("plan_concat_y4m must ignore unselected audio track packet size when --streams 0 is selected");
    assert_eq!(plan_concat_ok.command, "concat");

    let out_concat_mkv = dir.join("concat-selected.y4m");
    let concat_stats = fvid::media::concat(&[mkv.clone(), mkv.clone()], &out_concat_mkv, &mkv_ok)
        .expect("concat to Y4M must succeed when selected video track and output Y4M frame fit within max_packet_bytes=100");
    assert_eq!(concat_stats.packets, 6);

    // CLI `export-y4m` must also support `--streams 0` and `--rotate 90` with `--max-packet-bytes 100`.
    let bin = env!("CARGO_BIN_EXE_fvid");
    let out_cli_export = dir.join("cli-export-selected.y4m");
    let cli_out = Command::new(bin)
        .args([
            "media",
            "export-y4m",
            mkv.to_str().unwrap(),
            out_cli_export.to_str().unwrap(),
            "--streams",
            "0",
            "--rotate",
            "90",
            "--max-packet-bytes",
            "100",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        cli_out.status.success(),
        "CLI export-y4m with --streams 0 --rotate 90 --max-packet-bytes 100 must succeed: {}",
        String::from_utf8_lossy(&cli_out.stderr)
    );
    assert!(out_cli_export.exists());

    // 2. When `max_packet_bytes: 20` is smaller than the 24-byte output Y4M frame (or 25-byte FFV1 packet),
    //    `plan_trim_y4m` and `plan_concat_y4m` must reject it during planning.
    //    Even more specifically: if `max_packet_bytes` is 22 on a compressed source or if geometry scales
    //    or decodes to a larger Y4M frame (e.g. 24 bytes when compressed packet is <= max_packet_bytes),
    //    let's check `max_packet_bytes: 20` on `mkv` and `max_packet_bytes: 770` on `mp4`!
    //    Wait: in `multitrack-ffv1-aac.mkv`, the FFV1 packet is ~21 bytes and decoded Y4M frame is 24 bytes!
    //    Or with `max_packet_bytes = 23` (where FFV1 packet <= 23 bytes, or on `mkv` where decoded Y4M is 24 bytes):
    let mut mkv_small_frame = fvid::media::CopyOptions::default();
    mkv_small_frame.streams = vec![0];
    mkv_small_frame.max_packet_bytes = 23;
    let trim_err = fvid::media::plan_trim_y4m(&mkv, 0, 80_000, &mkv_small_frame)
        .expect_err("plan_trim_y4m must reject max_packet_bytes smaller than output Y4M frame or video packet");
    assert!(
        trim_err.contains("exceeds") && (trim_err.contains("limit") || trim_err.contains("budget")),
        "unexpected plan_trim_y4m error: {trim_err}"
    );

    // 3. Multitrack MP4 (track 0 = 32x16 AVC ~778B packets, 768B Y4M frame; track 1 = 2048B AAC packets).
    // With `--streams 0` and `max_packet_bytes: 1000`, unselected 2048B audio packets must not reject!
    let mut mp4_ok = fvid::media::CopyOptions::default();
    mp4_ok.streams = vec![0];
    mp4_ok.max_packet_bytes = 1000;
    let plan_mp4 = fvid::media::plan_trim_y4m(&mp4, 0, 40_000, &mp4_ok)
        .expect("plan_trim_y4m on MP4 with --streams 0 and max_packet_bytes=1000 must succeed");
    assert_eq!(plan_mp4.command, "trim");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_concat_y4m_and_plan_trim_y4m_enforce_cancellation_and_segment_compatibility() {
    let seg_25 = fixture("seg-4x4-25fps-par1x1.y4m");
    let seg_50 = fixture("seg-4x4-50fps-par1x1.y4m");
    let seg_par2x1 = fixture("seg-4x4-25fps-par2x1.y4m");
    let seg_2x4 = fixture("seg-2x4-25fps-par1x1.y4m");

    // 1. Pre-cancelled CancelFlag must fail `plan_concat_y4m` and `plan_trim_y4m`.
    let cancel = fvid::media_control::CancelFlag::new();
    cancel.cancel();
    let mut cancelled_opts = fvid::media::CopyOptions::default();
    cancelled_opts.cancel = Some(cancel);

    let err_trim = fvid::media::plan_trim_y4m(&seg_25, 0, 40_000, &cancelled_opts)
        .expect_err("plan_trim_y4m must honor pre-cancelled CancelFlag");
    assert!(err_trim.contains("cancelled"), "unexpected error: {err_trim}");

    let err_concat = fvid::media::plan_concat_y4m(&[seg_25.clone(), seg_25.clone()], &cancelled_opts)
        .expect_err("plan_concat_y4m must honor pre-cancelled CancelFlag");
    assert!(err_concat.contains("cancelled"), "unexpected error: {err_concat}");

    // 2. `plan_concat_y4m` must reject mismatched frame rates across segments.
    let err_rate = fvid::media::plan_concat_y4m(
        &[seg_25.clone(), seg_50],
        &fvid::media::CopyOptions::default(),
    )
    .expect_err("plan_concat_y4m must reject mismatched frame rates");
    assert!(
        err_rate.contains("Y4M concat requires identical frame rates"),
        "unexpected error: {err_rate}"
    );

    // 3. `plan_concat_y4m` must reject mismatched dimensions or pixel aspect across segments.
    let err_dim = fvid::media::plan_concat_y4m(
        &[seg_25.clone(), seg_2x4],
        &fvid::media::CopyOptions::default(),
    )
    .expect_err("plan_concat_y4m must reject mismatched frame dimensions");
    assert!(
        err_dim.contains("Y4M export requires fixed frame geometry and depth"),
        "unexpected error: {err_dim}"
    );

    let err_par = fvid::media::plan_concat_y4m(
        &[seg_25, seg_par2x1],
        &fvid::media::CopyOptions::default(),
    )
    .expect_err("plan_concat_y4m must reject mismatched pixel aspect ratio");
    assert!(
        err_par.contains("Y4M export requires fixed frame geometry and depth"),
        "unexpected error: {err_par}"
    );
}

#[test]
fn xfade_validates_pixel_aspect_compatibility_between_primary_and_secondary_inputs() {
    let seg_1x1 = fixture("seg-4x4-25fps-par1x1.y4m");
    let seg_2x2 = fixture("seg-4x4-25fps-par2x2.y4m");
    let seg_2x1 = fixture("seg-4x4-25fps-par2x1.y4m");
    let dir = temp_dir("xfade-aspect");

    // 1. Mismatched pixel aspect (A1:1 vs A2:1) must be rejected by both plan_xfade and xfade_video.
    let opts = fvid::media::CopyOptions::default();
    let plan_err = fvid_media::plan_xfade(&seg_1x1, &seg_2x1, "fade", 40_000, 40_000, &opts)
        .expect_err("plan_xfade must reject mismatched pixel aspect ratio");
    assert!(
        plan_err.contains("cross-fade inputs require matching geometry, format and range"),
        "unexpected plan_xfade error: {plan_err}"
    );

    let out_bad = dir.join("xfade-bad.mkv");
    let exec_err = fvid_media::xfade_video(&seg_1x1, &seg_2x1, &out_bad, "fade", 40_000, 40_000, &opts)
        .expect_err("xfade_video must reject mismatched pixel aspect ratio");
    assert_eq!(plan_err, exec_err);
    assert!(!out_bad.exists());

    // 2. Equivalent reduced pixel aspect (A1:1 vs A2:2) must succeed in both plan_xfade and xfade_video.
    let plan_ok = fvid_media::plan_xfade(&seg_1x1, &seg_2x2, "fade", 40_000, 40_000, &opts)
        .expect("plan_xfade must accept equivalent reduced pixel aspect A1:1 and A2:2");
    assert_eq!(plan_ok.command, "xfade");

    let out_ok = dir.join("xfade-ok.mkv");
    let stats = fvid_media::xfade_video(&seg_1x1, &seg_2x2, &out_ok, "fade", 40_000, 40_000, &opts)
        .expect("xfade_video must accept equivalent reduced pixel aspect A1:1 and A2:2");
    assert_eq!(stats.video_frames, 4);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn subtitle_burn_checks_cancellation_first_and_prioritizes_single_track_refusal_over_unselected_packets() {
    let mkv = fixture("multitrack-ffv1-aac.mkv");
    let y4m = fixture("seg-4x4-25fps-par1x1.y4m");
    let subs = fixture("cues.srt");
    let dir = temp_dir("subburn");

    // 1. Pre-cancelled CancelFlag must return "media operation cancelled" in both plan_burn_subtitles and burn_subtitles,
    //    even when other options or source tracks would otherwise fail.
    let cancel = fvid::media_control::CancelFlag::new();
    cancel.cancel();
    let mut cancelled_opts = fvid::media::CopyOptions::default();
    cancelled_opts.cancel = Some(cancel);
    cancelled_opts.max_packet_bytes = 10;

    let plan_cancel_err = fvid_media::plan_burn_subtitles(&y4m, &subs, &cancelled_opts)
        .expect_err("plan_burn_subtitles must check cancellation first");
    assert_eq!(plan_cancel_err, "media operation cancelled");

    let out_cancel = dir.join("burn-cancel.mkv");
    let burn_cancel_err = fvid_media::burn_subtitles(&y4m, &out_cancel, &subs, &cancelled_opts)
        .expect_err("burn_subtitles must check cancellation first");
    assert_eq!(burn_cancel_err, "media operation cancelled");
    assert!(!out_cancel.exists());

    // 2. Multitrack Matroska source with `max_packet_bytes: 100` (smaller than the 512-byte audio packet on track 1)
    //    must report the single-track subtitle-burn refusal instead of a misleading "WebM packet exceeds budget"
    //    from the unsupported second track.
    let mut small_pkt_opts = fvid::media::CopyOptions::default();
    small_pkt_opts.max_packet_bytes = 100;

    let plan_multi_err = fvid_media::plan_burn_subtitles(&mkv, &subs, &small_pkt_opts)
        .expect_err("plan_burn_subtitles must reject multitrack Matroska source");
    assert!(
        plan_multi_err.contains("owned subtitle burn requires Y4M or supported single-track Matroska/MP4 video"),
        "unexpected plan_burn_subtitles error on multitrack Matroska: {plan_multi_err}"
    );

    let out_multi = dir.join("burn-multi.mkv");
    let burn_multi_err = fvid_media::burn_subtitles(&mkv, &out_multi, &subs, &small_pkt_opts)
        .expect_err("burn_subtitles must reject multitrack Matroska source");
    assert_eq!(plan_multi_err, burn_multi_err);
    assert!(!out_multi.exists());

    let _ = std::fs::remove_dir_all(&dir);
}
