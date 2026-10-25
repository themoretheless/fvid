//! Regression tests for Batch 268 (`FVID-0275`):
//! Native Y4M video export (`export_y4m_pipeline_with_options`), trim (`trim_y4m_with_options`
//! and `plan_trim_y4m`), and concat (`concat_y4m_with_options` and `plan_concat_y4m`)
//! scope compressed MP4 and Matroska packet-byte budget checks (`options.max_packet_bytes`)
//! to the active `options.max_packets` prefix and reached concat segments instead of
//! rejecting oversized tail packets or unreached trailing segments that `max_packets`
//! stops before decoding.

use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-prefix-budget")
}

#[test]
fn y4m_export_trim_and_plan_trim_scope_packet_budget_to_max_packets_prefix() {
    let fixtures = fixture_dir();
    let mkv = fixtures.join("ffv1-two-frames-tail-oversized.mkv");
    let mp4 = fixtures.join("avc-two-frames-tail-oversized.mp4");

    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-prefix-budget-single-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. Matroska V_FFV1: packet 0 is ~29 bytes (decoded 4x3 frame = 18 bytes), packet 1 is ~229 bytes.
    // With max_packet_bytes = 64 and max_packets = Some(1), only packet 0 is decoded/exported,
    // so export_y4m_pipeline_with_options, trim_y4m_with_options, and plan_trim_y4m must succeed!
    let mut mkv_prefix1 = fvid::media::CopyOptions::default();
    mkv_prefix1.max_packet_bytes = 64;
    mkv_prefix1.max_packets = Some(1);

    let out_mkv_export = dir.join("mkv-export-prefix1.y4m");
    let stats = fvid::native_export::export_y4m_pipeline_with_options(
        &mkv,
        &out_mkv_export,
        None,
        &Default::default(),
        &Default::default(),
        &mkv_prefix1,
    )
    .expect("export_y4m_pipeline_with_options should accept Matroska when packet 0 fits max_packet_bytes=64 under max_packets=Some(1)");
    assert_eq!(stats.packets, 1);
    assert!(out_mkv_export.exists());

    let plan = fvid::native_plan::trim_y4m_with_options(&mkv, 0, 100_000, &mkv_prefix1)
        .expect("plan_trim_y4m should accept Matroska when packet 0 fits max_packet_bytes=64 under max_packets=Some(1)");
    assert_eq!(plan.command, "trim");

    let out_mkv_trim = dir.join("mkv-trim-prefix1.y4m");
    let trim_stats = fvid::native_export::trim_y4m_with_options(&mkv, &out_mkv_trim, 0, 100_000, &mkv_prefix1)
        .expect("trim_y4m_with_options should accept Matroska when packet 0 fits max_packet_bytes=64 under max_packets=Some(1)");
    assert_eq!(trim_stats.packets, 1);
    assert!(out_mkv_trim.exists());

    // Without max_packets=Some(1) (e.g. max_packets=Some(2) or None), packet 1 (~229 bytes > 64)
    // is reached and must still be rejected with "WebM packet exceeds budget"!
    let mut mkv_prefix2 = mkv_prefix1.clone();
    mkv_prefix2.max_packets = Some(2);
    let err_plan = fvid::native_plan::trim_y4m_with_options(&mkv, 0, 100_000, &mkv_prefix2)
        .expect_err("plan_trim_y4m with max_packets=Some(2) must reject oversized packet 1");
    assert!(
        err_plan.contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_plan}"
    );
    let out_mkv_bad = dir.join("mkv-export-prefix2.y4m");
    let err_exec = fvid::native_export::export_y4m_pipeline_with_options(
        &mkv,
        &out_mkv_bad,
        None,
        &Default::default(),
        &Default::default(),
        &mkv_prefix2,
    )
    .expect_err("export_y4m_pipeline_with_options with max_packets=Some(2) must reject oversized packet 1");
    assert!(
        err_exec.to_string().contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_exec}"
    );
    assert!(!out_mkv_bad.exists());

    // 2. MP4 avc1: sample 0 is 778 bytes (decoded 16x32 frame = 768 bytes), sample 1 is 1283 bytes.
    // With max_packet_bytes = 900 and max_packets = Some(1), only sample 0 is decoded/exported,
    // so export_y4m_pipeline_with_options, trim_y4m_with_options, and plan_trim_y4m must succeed!
    let mut mp4_prefix1 = fvid::media::CopyOptions::default();
    mp4_prefix1.max_packet_bytes = 900;
    mp4_prefix1.max_packets = Some(1);

    let out_mp4_export = dir.join("mp4-export-prefix1.y4m");
    let stats_mp4 = fvid::native_export::export_y4m_pipeline_with_options(
        &mp4,
        &out_mp4_export,
        None,
        &Default::default(),
        &Default::default(),
        &mp4_prefix1,
    )
    .expect("export_y4m_pipeline_with_options should accept MP4 when sample 0 fits max_packet_bytes=900 under max_packets=Some(1)");
    assert_eq!(stats_mp4.packets, 1);
    assert!(out_mp4_export.exists());

    let plan_mp4 = fvid::native_plan::trim_y4m_with_options(&mp4, 0, 100_000, &mp4_prefix1)
        .expect("plan_trim_y4m should accept MP4 when sample 0 fits max_packet_bytes=900 under max_packets=Some(1)");
    assert_eq!(plan_mp4.command, "trim");

    let out_mp4_trim = dir.join("mp4-trim-prefix1.y4m");
    let trim_mp4 = fvid::native_export::trim_y4m_with_options(&mp4, &out_mp4_trim, 0, 100_000, &mp4_prefix1)
        .expect("trim_y4m_with_options should accept MP4 when sample 0 fits max_packet_bytes=900 under max_packets=Some(1)");
    assert_eq!(trim_mp4.packets, 1);
    assert!(out_mp4_trim.exists());

    // With max_packets = Some(2), sample 1 (1283 bytes > 900) is reached and must be rejected!
    let mut mp4_prefix2 = mp4_prefix1.clone();
    mp4_prefix2.max_packets = Some(2);
    let err_mp4_plan = fvid::native_plan::trim_y4m_with_options(&mp4, 0, 100_000, &mp4_prefix2)
        .expect_err("plan_trim_y4m with max_packets=Some(2) must reject oversized MP4 sample 1");
    assert!(
        err_mp4_plan.contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget, got: {err_mp4_plan}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn y4m_concat_and_plan_concat_scope_packet_budget_across_segments() {
    let fixtures = fixture_dir();
    let one_frame = fixtures.join("ffv1-one-frame.mkv");
    let two_frames_tail_oversized = fixtures.join("ffv1-two-frames-tail-oversized.mkv");

    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-prefix-budget-concat-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Case A: sources = [two_frames_tail_oversized, two_frames_tail_oversized], max_packets = Some(1), max_packet_bytes = 64.
    // Segment 1 emits 1 frame (packet 0 <= 64 bytes) and exhausts max_packets = Some(1) before packet 1 of segment 1 or segment 2!
    let mut opts_1 = fvid::media::CopyOptions::default();
    opts_1.max_packet_bytes = 64;
    opts_1.max_packets = Some(1);

    let sources_a = vec![
        two_frames_tail_oversized.clone(),
        two_frames_tail_oversized.clone(),
    ];
    let plan_a = fvid::native_plan::concat_y4m_with_options(&sources_a, &opts_1)
        .expect("plan_concat_y4m with max_packets=Some(1) should succeed when frame 0 of segment 0 fits max_packet_bytes");
    assert_eq!(plan_a.command, "concat");

    let out_a = dir.join("concat-1pkt.y4m");
    let stats_a = fvid::native_export::concat_y4m_with_options(&sources_a, &out_a, &opts_1)
        .expect("concat_y4m_with_options with max_packets=Some(1) should succeed when frame 0 of segment 0 fits max_packet_bytes");
    assert_eq!(stats_a.packets, 1);
    assert!(out_a.exists());

    // Case B: sources = [one_frame, two_frames_tail_oversized], max_packets = Some(2), max_packet_bytes = 64.
    // Segment 1 contributes 1 frame (remaining_packets becomes 1 for segment 2).
    // Segment 2 only needs 1 frame (packet 0 <= 64 bytes), so packet 1 of segment 2 (~229 bytes) is never reached!
    let mut opts_2 = fvid::media::CopyOptions::default();
    opts_2.max_packet_bytes = 64;
    opts_2.max_packets = Some(2);

    let sources_b = vec![one_frame.clone(), two_frames_tail_oversized.clone()];
    let plan_b = fvid::native_plan::concat_y4m_with_options(&sources_b, &opts_2)
        .expect("plan_concat_y4m with max_packets=Some(2) should scope remaining_packets=Some(1) to segment 2");
    assert_eq!(plan_b.command, "concat");

    let out_b = dir.join("concat-2pkt.y4m");
    let stats_b = fvid::native_export::concat_y4m_with_options(&sources_b, &out_b, &opts_2)
        .expect("concat_y4m_with_options with max_packets=Some(2) should scope remaining_packets=Some(1) to segment 2");
    assert_eq!(stats_b.packets, 2);
    assert!(out_b.exists());

    // Case C: sources = [one_frame, two_frames_tail_oversized], max_packets = Some(3), max_packet_bytes = 64.
    // Segment 1 contributes 1 frame (remaining_packets becomes 2 for segment 2), so packet 1 of segment 2 IS reached and must fail!
    let mut opts_3 = fvid::media::CopyOptions::default();
    opts_3.max_packet_bytes = 64;
    opts_3.max_packets = Some(3);

    let err_plan_c = fvid::native_plan::concat_y4m_with_options(&sources_b, &opts_3)
        .expect_err("plan_concat_y4m with max_packets=Some(3) must reject oversized packet 1 in segment 2");
    assert!(
        err_plan_c.contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_plan_c}"
    );
    let out_c = dir.join("concat-3pkt.y4m");
    let err_exec_c = fvid::native_export::concat_y4m_with_options(&sources_b, &out_c, &opts_3)
        .expect_err("concat_y4m_with_options with max_packets=Some(3) must reject oversized packet 1 in segment 2");
    assert!(
        err_exec_c.to_string().contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_exec_c}"
    );
    assert!(!out_c.exists());

    let _ = std::fs::remove_dir_all(&dir);
}
