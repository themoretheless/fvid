use fvid::media::{self, CopyOptions, LosslessTransform, ScaleSize};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/multitrack-lossless-budget-parity")
}

#[test]
fn multitrack_lossless_planning_enforces_encoded_ffv1_packet_byte_budget_on_matroska_and_mp4() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-enc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. multitrack-ffv1-aac.mkv has ~29-byte FFV1 video packets on track 0 and 30-byte AAC packets on track 1.
    // When scaled to 128x128 with max_packet_bytes = 32 (> 29-byte video packet and > 30-byte AAC packet),
    // the re-encoded 128x128 FFV1 packet exceeds 32 bytes!
    // Both plan_transcode_lossless and transcode_lossless must reject with "encoded FFV1 packet exceeds byte limit".
    let scale128 = LosslessTransform {
        scale: Some(ScaleSize {
            width: 128,
            height: 128,
        }),
        ..Default::default()
    };
    let mut pkt32 = CopyOptions::default();
    pkt32.max_packet_bytes = 32;

    let out_mkv = temp_dir.join("mkv-scale128.mkv");
    let mkv_exec_err = media::transcode_lossless(&mkv, &out_mkv, scale128.clone(), &pkt32)
        .expect_err("transcode_lossless on multitrack MKV must reject encoded FFV1 packet > 32 bytes");
    assert_eq!(mkv_exec_err, "encoded FFV1 packet exceeds byte limit");
    assert!(!out_mkv.exists());

    let mkv_plan_err = media::plan_transcode_lossless(&mkv, &scale128, &pkt32, None)
        .expect_err("plan_transcode_lossless on multitrack MKV must reject encoded FFV1 packet > 32 bytes");
    assert_eq!(mkv_plan_err, mkv_exec_err);

    // 2. multitrack-avc-aac.mp4 has ~778-byte AVC video packets on track 0 and 2048-byte AAC packets on track 1.
    // When scaled to 256x256 with drawgrid="width=3:height=3:thickness=1:color=white" and max_packet_bytes = 2100 (> 778-byte AVC packet and > 2048-byte AAC packet),
    // the encoded FFV1 packet exceeds 2100 bytes!
    // Both plan_transcode_lossless and transcode_lossless must reject with "encoded FFV1 packet exceeds byte limit".
    let noisy_scale = LosslessTransform {
        scale: Some(ScaleSize {
            width: 256,
            height: 256,
        }),
        drawgrid: Some("width=3:height=3:thickness=1:color=white".into()),
        bitplanenoise: Some("1".into()),
        ..Default::default()
    };
    let mut pkt2100 = CopyOptions::default();
    pkt2100.max_packet_bytes = 2100;

    let out_mp4 = temp_dir.join("mp4-scale256.mkv");
    let mp4_exec_err = media::transcode_lossless(&mp4, &out_mp4, noisy_scale.clone(), &pkt2100)
        .expect_err("transcode_lossless on multitrack MP4 must reject encoded FFV1 packet > 2100 bytes");
    assert_eq!(mp4_exec_err, "encoded FFV1 packet exceeds byte limit");
    assert!(!out_mp4.exists());

    let mp4_plan_err = media::plan_transcode_lossless(&mp4, &noisy_scale, &pkt2100, None)
        .expect_err("plan_transcode_lossless on multitrack MP4 must reject encoded FFV1 packet > 2100 bytes");
    assert_eq!(mp4_plan_err, mp4_exec_err);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_enforce_selected_track_packet_byte_budgets() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-sel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. multitrack-ffv1-aac.mkv has ~29-byte FFV1 video packets on track 0 and 30-byte AAC packets on track 1.
    // When max_packet_bytes = 29 (< 30-byte AAC packets, >= 29-byte video packets):
    // - When streams = [] (both tracks selected), both plan_transcode_lossless and transcode_lossless reject with "WebM packet exceeds budget".
    // - When streams = [0] (only video stream 0 selected), both plan_transcode_lossless and transcode_lossless succeed!
    let mut mkv_both_29 = CopyOptions::default();
    mkv_both_29.max_packet_bytes = 29;
    let mkv_both_plan_err =
        media::plan_transcode_lossless(&mkv, &LosslessTransform::default(), &mkv_both_29, None)
            .unwrap_err();
    let out_mkv_both = temp_dir.join("mkv-both-29.mkv");
    let mkv_both_exec_err =
        media::transcode_lossless(&mkv, &out_mkv_both, LosslessTransform::default(), &mkv_both_29)
            .unwrap_err();
    assert!(
        mkv_both_plan_err.contains("WebM packet exceeds budget"),
        "mkv_both_plan_err: {mkv_both_plan_err}"
    );
    assert_eq!(mkv_both_exec_err, mkv_both_plan_err);
    assert!(!out_mkv_both.exists());

    let mut mkv_video_only_29 = CopyOptions::default();
    mkv_video_only_29.streams = vec![0];
    mkv_video_only_29.max_packet_bytes = 29;
    let mkv_plan = media::plan_transcode_lossless(
        &mkv,
        &LosslessTransform::default(),
        &mkv_video_only_29,
        None,
    )
    .expect("plan_transcode_lossless on multitrack MKV with streams=[0] and max_packet_bytes=29 must succeed");
    assert_eq!(mkv_plan.streams.len(), 1);
    assert_eq!(mkv_plan.streams[0].index, 0);

    let out_mkv_video = temp_dir.join("mkv-video-only-29.mkv");
    let mkv_stats = media::transcode_lossless(
        &mkv,
        &out_mkv_video,
        LosslessTransform::default(),
        &mkv_video_only_29,
    )
    .expect("transcode_lossless on multitrack MKV with streams=[0] and max_packet_bytes=29 must succeed");
    assert_eq!(mkv_stats.video_frames, 2);
    assert_eq!(mkv_stats.copied_packets, 0);

    // 2. multitrack-avc-aac.mp4 has ~778-byte AVC video packets on stream 0 and 2048-byte AAC packets on stream 1.
    // When streams = [0] (only video stream 0 is selected) and max_packet_bytes = 1024 (> 778-byte video packets, < 2048-byte unselected AAC packets),
    // both plan_transcode_lossless and transcode_lossless succeed; and when streams = [] (both selected), both reject with "MP4 packet exceeds budget"!
    let mut both_1024 = CopyOptions::default();
    both_1024.max_packet_bytes = 1024;
    let both_plan_err =
        media::plan_transcode_lossless(&mp4, &LosslessTransform::default(), &both_1024, None)
            .unwrap_err();
    let out_both = temp_dir.join("mp4-both-1024.mkv");
    let both_exec_err =
        media::transcode_lossless(&mp4, &out_both, LosslessTransform::default(), &both_1024)
            .unwrap_err();
    assert_eq!(both_plan_err, "MP4 packet exceeds budget");
    assert_eq!(both_exec_err, both_plan_err);
    assert!(!out_both.exists());

    let mut video_only_1024 = CopyOptions::default();
    video_only_1024.streams = vec![0];
    video_only_1024.max_packet_bytes = 1024;

    let plan = media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &video_only_1024,
        None,
    )
    .expect("plan_transcode_lossless with streams=[0] and max_packet_bytes=1024 must succeed");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].index, 0);

    let out_mp4 = temp_dir.join("mp4-video-only-1024.mkv");
    let stats = media::transcode_lossless(
        &mp4,
        &out_mp4,
        LosslessTransform::default(),
        &video_only_1024,
    )
    .expect("transcode_lossless with streams=[0] and max_packet_bytes=1024 must succeed");
    assert_eq!(stats.video_frames, 2);
    assert_eq!(stats.copied_packets, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_preserve_policy_and_metadata_refusal_parity() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-pol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. max_controlled_bytes = Some(1024) on multitrack MKV and MP4 must return
    // "owned Y4M lossless export does not yet implement requested policy" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut mem_opts = CopyOptions::default();
    mem_opts.max_controlled_bytes = Some(1024);

    // 2. Invalid container metadata (e.g. NUL byte in value) on multitrack MKV and MP4 must return
    // "NUL in container metadata" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut bad_meta = CopyOptions::default();
    bad_meta.metadata_set = vec![("TITLE".into(), "bad\0title".into())];

    // 3. Stream metadata targeting a missing input track (e.g. track 5 on a 2-track file) on multitrack MKV and MP4 must return
    // "metadata refers to missing input track" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut missing_track_meta = CopyOptions::default();
    missing_track_meta.stream_metadata_set = vec![(5, "CUSTOM".into(), "ok".into())];

    // 4. Invalid stream metadata on companion track 1 (e.g. NUL byte in value) on multitrack MKV and MP4 must return
    // "owned Y4M lossless export does not yet implement requested policy" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut bad_stream_meta = CopyOptions::default();
    bad_stream_meta.stream_metadata_set = vec![(1, "CUSTOM".into(), "bad\0val".into())];

    // 5. Selecting only companion audio stream 1 while excluding video stream 0 (streams = [1]) AND setting a tight
    // max_packet_bytes (e.g. 20 bytes) must still report "lossless export requires at least one selected video stream"
    // rather than failing early on unselected video packet bytes in open()!
    let mut audio_only_20 = CopyOptions::default();
    audio_only_20.streams = vec![1];
    audio_only_20.max_packet_bytes = 20;

    for (opt_label, opts, expected_substr) in [
        (
            "max_controlled_bytes=1024",
            &mem_opts,
            "owned Y4M lossless export does not yet implement requested policy",
        ),
        (
            "NUL in container metadata",
            &bad_meta,
            "NUL in container metadata",
        ),
        (
            "missing input track in stream metadata",
            &missing_track_meta,
            "metadata refers to missing input track",
        ),
        (
            "NUL in companion stream metadata",
            &bad_stream_meta,
            "owned Y4M lossless export does not yet implement requested policy",
        ),
        (
            "audio_only streams=[1] with max_packet_bytes=20",
            &audio_only_20,
            "lossless export requires at least one selected video stream",
        ),
    ] {
        for (src_label, src) in [("multitrack-mkv", &mkv), ("multitrack-mp4", &mp4)] {
            let plan_err = media::plan_transcode_lossless(
                src,
                &LosslessTransform::default(),
                opts,
                None,
            )
            .unwrap_err();
            let out = temp_dir.join(format!("{src_label}-bad.mkv"));
            let exec_err = media::transcode_lossless(
                src,
                &out,
                LosslessTransform::default(),
                opts,
            )
            .unwrap_err();
            assert!(
                plan_err.contains(expected_substr),
                "plan_transcode_lossless({src_label}, {opt_label}) returned: {plan_err}"
            );
            assert_eq!(
                exec_err, plan_err,
                "exec vs plan mismatch on ({src_label}, {opt_label})"
            );
            assert!(!out.exists());
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
