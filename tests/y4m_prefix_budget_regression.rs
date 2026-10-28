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

#[test]
fn y4m_interval_export_trim_and_plan_trim_scope_packet_budget_to_interval_and_nonzero_from_max_packets() {
    let fixtures = fixture_dir();
    let mkv3 = fixtures.join("ffv1-three-frames-tail-oversized.mkv");
    let mp43 = fixtures.join("avc-three-frames-tail-oversized.mp4");

    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-interval-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 1. Matroska V_FFV1 3-frame fixture:
    //    frame 0 @ 0 ms (~29 bytes), frame 1 @ 21 ms (~29 bytes), frame 2 @ 42 ms (~229 bytes).
    //    (a) Interval [0, 30_000 us) with max_packets = None and max_packet_bytes = 64:
    //        Frames 0 and 1 are in [0, 30ms), while oversized frame 2 @ 42ms is >= to (30ms).
    let mut mkv_interval_only = fvid::media::CopyOptions::default();
    mkv_interval_only.max_packet_bytes = 64;

    let plan_mkv_a = fvid::native_plan::trim_y4m_with_options(&mkv3, 0, 30_000, &mkv_interval_only)
        .expect("plan_trim_y4m on [0, 30ms) with max_packets=None should exclude oversized frame 2 @ 42ms");
    assert_eq!(plan_mkv_a.command, "trim");
    let media_plan_mkv_a = fvid::media::plan_trim_y4m(&mkv3, 0, 30_000, &mkv_interval_only)
        .expect("fvid::media::plan_trim_y4m on [0, 30ms) should succeed");
    assert_eq!(media_plan_mkv_a.command, "trim");

    let out_mkv_a_trim = dir.join("mkv-interval-0-30ms.y4m");
    let stats_mkv_a = fvid::native_export::trim_y4m_with_options(
        &mkv3,
        &out_mkv_a_trim,
        0,
        30_000,
        &mkv_interval_only,
    )
    .expect("trim_y4m_with_options on [0, 30ms) with max_packets=None should exclude oversized frame 2 @ 42ms");
    assert_eq!(stats_mkv_a.packets, 2);
    assert!(out_mkv_a_trim.exists());

    let out_mkv_a_media = dir.join("mkv-media-trim-0-30ms.y4m");
    let stats_mkv_a_media = fvid::media::trim(&mkv3, &out_mkv_a_media, 0, 30_000, &mkv_interval_only)
        .expect("fvid::media::trim to .y4m on [0, 30ms) should succeed");
    assert_eq!(stats_mkv_a_media.packets, 2);

    //    (b) Non-zero `from`: interval [21_000 us, 100_000 us) with max_packets = Some(1) and max_packet_bytes = 64:
    //        Frame 0 @ 0ms is decoded as pre-roll (< 21ms), frame 1 @ 21ms is emitted as packet 1,
    //        exhausting max_packets = Some(1) before oversized frame 2 @ 42ms!
    let mut mkv_nonzero_from = fvid::media::CopyOptions::default();
    mkv_nonzero_from.max_packet_bytes = 64;
    mkv_nonzero_from.max_packets = Some(1);

    let plan_mkv_b = fvid::native_plan::trim_y4m_with_options(&mkv3, 21_000, 100_000, &mkv_nonzero_from)
        .expect("plan_trim_y4m on [21ms, 100ms) with max_packets=Some(1) should scope budget to pre-roll + 1 emitted frame");
    assert_eq!(plan_mkv_b.command, "trim");

    let out_mkv_b_trim = dir.join("mkv-interval-21-100ms-1pkt.y4m");
    let stats_mkv_b = fvid::native_export::trim_y4m_with_options(
        &mkv3,
        &out_mkv_b_trim,
        21_000,
        100_000,
        &mkv_nonzero_from,
    )
    .expect("trim_y4m_with_options on [21ms, 100ms) with max_packets=Some(1) should succeed");
    assert_eq!(stats_mkv_b.packets, 1);
    assert!(out_mkv_b_trim.exists());

    let out_mkv_b_export = dir.join("mkv-export-21-100ms-1pkt.y4m");
    let stats_mkv_b_export = fvid::native_export::export_y4m_pipeline_with_options(
        &mkv3,
        &out_mkv_b_export,
        Some((
            std::time::Duration::from_micros(21_000),
            std::time::Duration::from_micros(100_000),
        )),
        &Default::default(),
        &Default::default(),
        &mkv_nonzero_from,
    )
    .expect("export_y4m_pipeline_with_options on [21ms, 100ms) with max_packets=Some(1) should succeed");
    assert_eq!(stats_mkv_b_export.packets, 1);

    //    (c) Control check on Matroska: interval [21_000 us, 100_000 us) with max_packets = Some(2) (or None)
    //        reaches oversized frame 2 @ 42ms and must reject it with "WebM packet exceeds budget"!
    let mut mkv_nonzero_from_2 = mkv_nonzero_from.clone();
    mkv_nonzero_from_2.max_packets = Some(2);
    let err_mkv_plan = fvid::native_plan::trim_y4m_with_options(&mkv3, 21_000, 100_000, &mkv_nonzero_from_2)
        .expect_err("plan_trim_y4m on [21ms, 100ms) with max_packets=Some(2) must reject oversized frame 2");
    assert!(
        err_mkv_plan.contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_mkv_plan}"
    );
    let out_mkv_bad = dir.join("mkv-interval-21-100ms-2pkt.y4m");
    let err_mkv_exec = fvid::native_export::trim_y4m_with_options(
        &mkv3,
        &out_mkv_bad,
        21_000,
        100_000,
        &mkv_nonzero_from_2,
    )
    .expect_err("trim_y4m_with_options on [21ms, 100ms) with max_packets=Some(2) must reject oversized frame 2");
    assert!(
        err_mkv_exec.to_string().contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_mkv_exec}"
    );
    assert!(!out_mkv_bad.exists());

    // 2. MP4 avc1 3-frame fixture (rate=48000, delta=1024 => 21_333.333 us/frame:
    //    sample 0 @ 0 us = 778B, sample 1 @ 21_333.333 us = 778B, sample 2 @ 42_666.666 us = 1283B):
    //    (a) Interval [0, 30_000 us) with max_packets = None and max_packet_bytes = 900:
    //        Samples 0 and 1 are in [0, 30ms), while oversized sample 2 @ 42.667ms is >= to (30ms).
    let mut mp4_interval_only = fvid::media::CopyOptions::default();
    mp4_interval_only.max_packet_bytes = 900;

    let plan_mp4_a = fvid::native_plan::trim_y4m_with_options(&mp43, 0, 30_000, &mp4_interval_only)
        .expect("plan_trim_y4m on MP4 [0, 30ms) with max_packets=None should exclude oversized sample 2 @ 42.67ms");
    assert_eq!(plan_mp4_a.command, "trim");

    let out_mp4_a_trim = dir.join("mp4-interval-0-30ms.y4m");
    let stats_mp4_a = fvid::native_export::trim_y4m_with_options(
        &mp43,
        &out_mp4_a_trim,
        0,
        30_000,
        &mp4_interval_only,
    )
    .expect("trim_y4m_with_options on MP4 [0, 30ms) with max_packets=None should exclude oversized sample 2 @ 42.67ms");
    assert_eq!(stats_mp4_a.packets, 2);
    assert!(out_mp4_a_trim.exists());

    //    (b) Non-zero `from`: interval [20_000 us, 100_000 us) with max_packets = Some(1) and max_packet_bytes = 900:
    //        Sample 0 @ 0ms is decoded as pre-roll (< 20ms), sample 1 @ 21.33ms is emitted as packet 1,
    //        exhausting max_packets = Some(1) before oversized sample 2 @ 42.67ms!
    let mut mp4_nonzero_from = fvid::media::CopyOptions::default();
    mp4_nonzero_from.max_packet_bytes = 900;
    mp4_nonzero_from.max_packets = Some(1);

    let plan_mp4_b = fvid::native_plan::trim_y4m_with_options(&mp43, 20_000, 100_000, &mp4_nonzero_from)
        .expect("plan_trim_y4m on MP4 [20ms, 100ms) with max_packets=Some(1) should scope budget to pre-roll + 1 emitted sample");
    assert_eq!(plan_mp4_b.command, "trim");

    let out_mp4_b_trim = dir.join("mp4-interval-20-100ms-1pkt.y4m");
    let stats_mp4_b = fvid::native_export::trim_y4m_with_options(
        &mp43,
        &out_mp4_b_trim,
        20_000,
        100_000,
        &mp4_nonzero_from,
    )
    .expect("trim_y4m_with_options on MP4 [20ms, 100ms) with max_packets=Some(1) should succeed");
    assert_eq!(stats_mp4_b.packets, 1);
    assert!(out_mp4_b_trim.exists());

    //    (c) Control check on MP4: interval [20_000 us, 100_000 us) with max_packets = Some(2)
    //        reaches oversized sample 2 @ 42.67ms and must reject it with "MP4 packet exceeds budget"!
    let mut mp4_nonzero_from_2 = mp4_nonzero_from.clone();
    mp4_nonzero_from_2.max_packets = Some(2);
    let err_mp4_plan = fvid::native_plan::trim_y4m_with_options(&mp43, 20_000, 100_000, &mp4_nonzero_from_2)
        .expect_err("plan_trim_y4m on MP4 [20ms, 100ms) with max_packets=Some(2) must reject oversized sample 2");
    assert!(
        err_mp4_plan.contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget, got: {err_mp4_plan}"
    );
    let out_mp4_bad = dir.join("mp4-interval-20-100ms-2pkt.y4m");
    let err_mp4_exec = fvid::native_export::trim_y4m_with_options(
        &mp43,
        &out_mp4_bad,
        20_000,
        100_000,
        &mp4_nonzero_from_2,
    )
    .expect_err("trim_y4m_with_options on MP4 [20ms, 100ms) with max_packets=Some(2) must reject oversized sample 2");
    assert!(
        err_mp4_exec.to_string().contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget, got: {err_mp4_exec}"
    );
    assert!(!out_mp4_bad.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn matroska_ffv1_lossless_interval_export_and_plan_scope_packet_budget_to_active_interval() {
    let fixtures = fixture_dir();
    let mkv3 = fixtures.join("ffv1-three-frames-tail-oversized.mkv");

    let dir = std::env::temp_dir().join(format!(
        "fvid-lossless-ffv1-interval-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // `ffv1-three-frames-tail-oversized.mkv` has 3 FFV1 frames:
    //   packet 0 @ 0 ms (~29 bytes),
    //   packet 1 @ 21 ms (~29 bytes),
    //   packet 2 @ 42 ms (~229 bytes).
    //
    // 1. Interval [0, 30_000 us) with max_packet_bytes = 64 and max_packets = Some(2):
    //    `Presentation::next` stops before packet 2 @ 42 ms (`pre_read_stop = 30 ms`),
    //    so only packets 0 and 1 (each <= 64 bytes) are consumed.
    let mut transform_0_30 = fvid::media::LosslessTransform::default();
    transform_0_30.interval = Some((0, 30_000));

    let mut opts = fvid::media::CopyOptions::default();
    opts.max_packet_bytes = 64;
    opts.max_packets = Some(2);

    let plan_0_30 = fvid::media::plan_transcode_lossless(&mkv3, &transform_0_30, &opts, None)
        .expect("plan_transcode_lossless on [0, 30ms) should exclude trailing oversized FFV1 packet 2 @ 42ms");
    assert_eq!(plan_0_30.command, "transcode-lossless");

    let out_0_30 = dir.join("lossless-0-30ms.mkv");
    let stats_0_30 = fvid::media::transcode_lossless(&mkv3, &out_0_30, transform_0_30.clone(), &opts)
        .expect("transcode_lossless on [0, 30ms) should exclude trailing oversized FFV1 packet 2 @ 42ms");
    assert_eq!(stats_0_30.decoded_frames, 2);
    assert_eq!(stats_0_30.video_frames, 2);
    assert!(out_0_30.exists());

    // 2. Non-zero `from`: interval [21_000 us, 30_000 us) with max_packet_bytes = 64 and max_packets = Some(2):
    //    Packet 0 @ 0 ms is decoded as pre-roll, packet 1 @ 21 ms is emitted, and packet 2 @ 42 ms is never read.
    let mut transform_21_30 = fvid::media::LosslessTransform::default();
    transform_21_30.interval = Some((21_000, 30_000));

    let plan_21_30 = fvid::media::plan_transcode_lossless(&mkv3, &transform_21_30, &opts, None)
        .expect("plan_transcode_lossless on [21ms, 30ms) should exclude trailing oversized FFV1 packet 2 @ 42ms");
    assert_eq!(plan_21_30.command, "transcode-lossless");

    let out_21_30 = dir.join("lossless-21-30ms.mkv");
    let stats_21_30 = fvid::media::transcode_lossless(&mkv3, &out_21_30, transform_21_30.clone(), &opts)
        .expect("transcode_lossless on [21ms, 30ms) should exclude trailing oversized FFV1 packet 2 @ 42ms");
    assert_eq!(stats_21_30.decoded_frames, 2);
    assert_eq!(stats_21_30.video_frames, 1);
    assert!(out_21_30.exists());

    // 3. Control check: interval [0, 60_000 us) includes packet 2 @ 42 ms (~229 bytes > 64 bytes)
    //    and must be rejected by both `plan_transcode_lossless` and `transcode_lossless`!
    let mut transform_0_60 = fvid::media::LosslessTransform::default();
    transform_0_60.interval = Some((0, 60_000));
    let mut opts_3 = opts.clone();
    opts_3.max_packets = Some(3);

    let err_plan = fvid::media::plan_transcode_lossless(&mkv3, &transform_0_60, &opts_3, None)
        .expect_err("plan_transcode_lossless on [0, 60ms) must reject active oversized FFV1 packet 2");
    assert!(
        err_plan.to_string().contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_plan}"
    );
    let out_bad = dir.join("lossless-0-60ms.mkv");
    let err_exec = fvid::media::transcode_lossless(&mkv3, &out_bad, transform_0_60, &opts_3)
        .expect_err("transcode_lossless on [0, 60ms) must reject active oversized FFV1 packet 2");
    assert!(
        err_exec.to_string().contains("WebM packet exceeds budget"),
        "expected WebM packet exceeds budget, got: {err_exec}"
    );
    assert!(!out_bad.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_trim_y4m_and_plan_concat_y4m_reject_zero_frame_inputs_and_empty_presentation_intervals_under_default_options() {
    let fixtures = fixture_dir();
    let zero_y4m = fixtures.join("zero-frames-4x4.y4m");
    let valid_y4m = fixtures.join("two-frames-4x4-25fps.y4m");
    let mkv3 = fixtures.join("ffv1-three-frames-tail-oversized.mkv");
    let mp43 = fixtures.join("avc-three-frames-tail-oversized.mp4");

    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-plan-empty-interval-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let default_opts = fvid::media::CopyOptions::default();

    // 1. 0-frame Y4M source (`zero-frames-4x4.y4m`) under default CopyOptions must be rejected
    //    by both `plan_trim_y4m` and `trim_y4m_with_options` with "input has no decoded video frames".
    let out_zero_trim = dir.join("zero-trim.y4m");
    let exec_zero_trim = fvid::native_export::trim_y4m_with_options(
        &zero_y4m,
        &out_zero_trim,
        0,
        40_000,
        &default_opts,
    )
    .expect_err("trim_y4m_with_options must reject 0-frame Y4M input");
    assert!(
        exec_zero_trim
            .to_string()
            .contains("input has no decoded video frames"),
        "unexpected trim_y4m_with_options error: {exec_zero_trim}"
    );
    assert!(!out_zero_trim.exists());

    let plan_zero_trim = fvid::media::plan_trim_y4m(&zero_y4m, 0, 40_000, &default_opts)
        .expect_err("plan_trim_y4m must reject 0-frame Y4M input under default CopyOptions");
    assert!(
        plan_zero_trim.contains("input has no decoded video frames"),
        "unexpected plan_trim_y4m error: {plan_zero_trim}"
    );

    // 2. Empty Y4M interval [80_000 us, 120_000 us) on a 2-frame 25fps Y4M (frames @ 0 us and 40_000 us)
    //    under default CopyOptions must be rejected by both `plan_trim_y4m` and `trim_y4m_with_options`.
    let out_y4m_after = dir.join("y4m-after.y4m");
    let exec_y4m_after = fvid::native_export::trim_y4m_with_options(
        &valid_y4m,
        &out_y4m_after,
        80_000,
        120_000,
        &default_opts,
    )
    .expect_err("trim_y4m_with_options must reject empty Y4M interval beyond EOF");
    assert!(
        exec_y4m_after
            .to_string()
            .contains("input has no decoded video frames"),
        "unexpected trim_y4m_with_options error: {exec_y4m_after}"
    );
    assert!(!out_y4m_after.exists());

    let plan_y4m_after = fvid::media::plan_trim_y4m(&valid_y4m, 80_000, 120_000, &default_opts)
        .expect_err("plan_trim_y4m must reject empty Y4M interval beyond EOF under default CopyOptions");
    assert!(
        plan_y4m_after.contains("input has no decoded video frames"),
        "unexpected plan_trim_y4m error: {plan_y4m_after}"
    );

    // 3. Empty Matroska interval [100_000 us, 200_000 us) on `ffv1-three-frames-tail-oversized.mkv`
    //    (frames @ 0 ms, 21 ms, 42 ms) under default CopyOptions must be rejected by both
    //    `plan_trim_y4m` and `trim_y4m_with_options`.
    let out_mkv_after = dir.join("mkv-after.y4m");
    let exec_mkv_after = fvid::native_export::trim_y4m_with_options(
        &mkv3,
        &out_mkv_after,
        100_000,
        200_000,
        &default_opts,
    )
    .expect_err("trim_y4m_with_options must reject empty Matroska interval beyond EOF");
    assert!(
        exec_mkv_after
            .to_string()
            .contains("input has no decoded video frames"),
        "unexpected trim_y4m_with_options error: {exec_mkv_after}"
    );
    assert!(!out_mkv_after.exists());

    let plan_mkv_after = fvid::media::plan_trim_y4m(&mkv3, 100_000, 200_000, &default_opts)
        .expect_err("plan_trim_y4m must reject empty Matroska interval beyond EOF under default CopyOptions");
    assert!(
        plan_mkv_after.contains("input has no decoded video frames"),
        "unexpected plan_trim_y4m error: {plan_mkv_after}"
    );

    // 4. Empty MP4 interval [100_000 us, 200_000 us) on `avc-three-frames-tail-oversized.mp4`
    //    (samples @ 0 ms, 21.33 ms, 42.67 ms) under default CopyOptions must be rejected by both
    //    `plan_trim_y4m` and `trim_y4m_with_options`.
    let out_mp4_after = dir.join("mp4-after.y4m");
    let exec_mp4_after = fvid::native_export::trim_y4m_with_options(
        &mp43,
        &out_mp4_after,
        100_000,
        200_000,
        &default_opts,
    )
    .expect_err("trim_y4m_with_options must reject empty MP4 interval beyond EOF");
    assert!(
        exec_mp4_after
            .to_string()
            .contains("input has no decoded video frames"),
        "unexpected trim_y4m_with_options error: {exec_mp4_after}"
    );
    assert!(!out_mp4_after.exists());

    let plan_mp4_after = fvid::media::plan_trim_y4m(&mp43, 100_000, 200_000, &default_opts)
        .expect_err("plan_trim_y4m must reject empty MP4 interval beyond EOF under default CopyOptions");
    assert!(
        plan_mp4_after.contains("input has no decoded video frames"),
        "unexpected plan_trim_y4m error: {plan_mp4_after}"
    );

    // 5. `plan_concat_y4m` and `concat_y4m_with_options` on a 0-frame second segment (`[valid_y4m, zero_y4m]`)
    //    when `max_packets = Some(2)` (where segment 0 has 2 frames so `remaining_packets` reaches 0 after segment 0)
    //    must still succeed when segment 1 is unreached, whereas when segment 1 IS reached (`max_packets = None` or `Some(3)`),
    //    both `plan_concat_y4m` and `concat_y4m_with_options` reject the 0-frame segment with "input has no decoded video frames"!
    let mut opts_2pkt = default_opts.clone();
    opts_2pkt.max_packets = Some(2);
    let plan_unreached_ok = fvid::media::plan_concat_y4m(
        &[valid_y4m.clone(), zero_y4m.clone()],
        &opts_2pkt,
    )
    .expect("plan_concat_y4m with max_packets=Some(2) should succeed when 2-frame segment 0 satisfies the limit before 0-frame segment 1");
    assert_eq!(plan_unreached_ok.command, "concat");

    let out_unreached_ok = dir.join("concat-unreached-ok.y4m");
    let stats_unreached = fvid::native_export::concat_y4m_with_options(
        &[valid_y4m.clone(), zero_y4m.clone()],
        &out_unreached_ok,
        &opts_2pkt,
    )
    .expect("concat_y4m_with_options with max_packets=Some(2) should succeed when 2-frame segment 0 satisfies the limit before 0-frame segment 1");
    assert_eq!(stats_unreached.packets, 2);
    assert!(out_unreached_ok.exists());

    let plan_reached_err = fvid::media::plan_concat_y4m(
        &[valid_y4m.clone(), zero_y4m.clone()],
        &default_opts,
    )
    .expect_err("plan_concat_y4m must reject reached 0-frame segment 1");
    assert!(
        plan_reached_err.contains("input has no decoded video frames"),
        "unexpected plan_concat_y4m error: {plan_reached_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
