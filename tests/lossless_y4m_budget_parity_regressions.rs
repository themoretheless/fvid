use fvid::media::{self, CopyOptions, LosslessTransform};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-y4m-budget-parity")
}

#[test]
fn lossless_export_and_planning_validate_zero_packet_bytes_and_policy_across_y4m_mp4_and_matroska() {
    let dir = fixture_dir();
    let y4m = dir.join("lossless-textured.y4m");
    let mkv = dir.join("single-video-ffv1.mkv");
    let mp4 = dir.join("single-video-avc.mp4");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch252-policy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. max_packet_bytes == 0 must return "packet byte limit must be positive" on all three containers
    // across both plan_transcode_lossless and transcode_lossless (instead of "request has no owned lossless export plan"
    // or "owned Y4M lossless export does not yet implement requested policy").
    let mut zero_pkt = CopyOptions::default();
    zero_pkt.max_packet_bytes = 0;

    for (label, src) in [("y4m", &y4m), ("mkv", &mkv), ("mp4", &mp4)] {
        let plan_err = media::plan_transcode_lossless(
            src,
            &LosslessTransform::default(),
            &zero_pkt,
            None,
        )
        .unwrap_err();
        assert_eq!(
            plan_err, "packet byte limit must be positive",
            "plan_transcode_lossless({label}) with max_packet_bytes=0 returned: {plan_err}"
        );

        let out = temp_dir.join(format!("{label}-zero-pkt.mkv"));
        let exec_err = media::transcode_lossless(
            src,
            &out,
            LosslessTransform::default(),
            &zero_pkt,
        )
        .unwrap_err();
        assert_eq!(exec_err, plan_err, "exec vs plan mismatch on {label} with max_packet_bytes=0");
        assert!(!out.exists());

        let fvid_plan_err = fvid_media::plan_transcode_lossless(
            src,
            &LosslessTransform::default(),
            &zero_pkt,
            None,
        )
        .unwrap_err();
        let fvid_exec_err = fvid_media::transcode_lossless(
            src,
            &out,
            LosslessTransform::default(),
            &zero_pkt,
        )
        .unwrap_err();
        assert_eq!(fvid_plan_err, "packet byte limit must be positive");
        assert_eq!(fvid_exec_err, fvid_plan_err);
    }

    // 2. Unsupported policy options (streams = [1] or max_controlled_bytes = Some(1024)) on Y4M, single-track MP4,
    // and single-track Matroska must report identical policy refusals in plan_transcode_lossless and transcode_lossless.
    let mut stream1 = CopyOptions::default();
    stream1.streams = vec![1];
    let mut mem_budget = CopyOptions::default();
    mem_budget.max_controlled_bytes = Some(1024);

    for (opt_label, opts) in [("streams=[1]", &stream1), ("max_controlled_bytes=1024", &mem_budget)] {
        for (src_label, src) in [("y4m", &y4m), ("mkv", &mkv), ("mp4", &mp4)] {
            let plan_err = media::plan_transcode_lossless(
                src,
                &LosslessTransform::default(),
                opts,
                None,
            )
            .unwrap_err();
            let out = temp_dir.join(format!("{src_label}-policy.mkv"));
            let exec_err = media::transcode_lossless(
                src,
                &out,
                LosslessTransform::default(),
                opts,
            )
            .unwrap_err();
            assert_eq!(
                exec_err, plan_err,
                "exec vs plan mismatch on {src_label} with {opt_label}: exec={exec_err:?}, plan={plan_err:?}"
            );
            assert!(
                plan_err.contains("owned Y4M lossless export does not yet implement requested policy"),
                "unexpected error on {src_label} with {opt_label}: {plan_err}"
            );
            assert!(!out.exists());
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn lossless_planning_enforces_encoded_ffv1_packet_bytes_and_zero_packet_work_limit() {
    let dir = fixture_dir();
    let y4m = dir.join("lossless-textured.y4m");
    let mkv = dir.join("single-video-ffv1.mkv");
    let mp4 = dir.join("single-video-avc.mp4");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch252-encbudget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. lossless-textured.y4m has 384-byte input Y4M frames (16x16), and when scaled to 128x128
    // its encoded FFV1 packet exceeds 390 bytes while the source Y4M frame (384 bytes) <= 390 bytes!
    // Both plan_transcode_lossless and transcode_lossless must reject it with "encoded FFV1 packet exceeds byte limit".
    let scale128_transform = LosslessTransform {
        scale: Some(fvid::media::ScaleSize { width: 128, height: 128 }),
        ..Default::default()
    };
    let mut pkt390 = CopyOptions::default();
    pkt390.max_packet_bytes = 390;

    let y4m_plan_err = media::plan_transcode_lossless(
        &y4m,
        &scale128_transform,
        &pkt390,
        None,
    )
    .unwrap_err();
    let out_y4m = temp_dir.join("y4m-390.mkv");
    let y4m_exec_err = media::transcode_lossless(
        &y4m,
        &out_y4m,
        scale128_transform,
        &pkt390,
    )
    .unwrap_err();
    assert_eq!(y4m_plan_err, "encoded FFV1 packet exceeds byte limit");
    assert_eq!(y4m_exec_err, y4m_plan_err);
    assert!(!out_y4m.exists());

    // 2. single-video-ffv1.mkv has 29-byte input packets; when scaled to 128x128 and encoded,
    // with max_packet_bytes = 32 (larger than 29-byte input packet, smaller than re-encoded 128x128 FFV1 packet),
    // both plan_transcode_lossless and transcode_lossless must reject with "encoded FFV1 packet exceeds byte limit".
    let scale_transform = LosslessTransform {
        scale: Some(fvid::media::ScaleSize { width: 128, height: 128 }),
        ..Default::default()
    };
    let mut pkt32 = CopyOptions::default();
    pkt32.max_packet_bytes = 32;

    let mkv_plan_err = media::plan_transcode_lossless(
        &mkv,
        &scale_transform,
        &pkt32,
        None,
    )
    .unwrap_err();
    let out_mkv = temp_dir.join("mkv-32.mkv");
    let mkv_exec_err = media::transcode_lossless(
        &mkv,
        &out_mkv,
        scale_transform.clone(),
        &pkt32,
    )
    .unwrap_err();
    assert_eq!(mkv_plan_err, "encoded FFV1 packet exceeds byte limit");
    assert_eq!(mkv_exec_err, mkv_plan_err);
    assert!(!out_mkv.exists());

    // 3. max_packets = Some(0) on Y4M, Matroska, and MP4 must fail identically in plan_transcode_lossless and transcode_lossless
    let mut zero_packets = CopyOptions::default();
    zero_packets.max_packets = Some(0);
    for (src_label, src) in [("y4m", &y4m), ("mkv", &mkv), ("mp4", &mp4)] {
        let p_err = media::plan_transcode_lossless(
            src,
            &LosslessTransform::default(),
            &zero_packets,
            None,
        )
        .unwrap_err();
        let out = temp_dir.join(format!("{src_label}-0pkts.mkv"));
        let e_err = media::transcode_lossless(
            src,
            &out,
            LosslessTransform::default(),
            &zero_packets,
        )
        .unwrap_err();
        assert_eq!(e_err, p_err, "max_packets=0 mismatch on {src_label}");
        assert!(!out.exists());
    }

    // 4. Positive acceptance with default options on all three single-track inputs
    for (src_label, src) in [("y4m", &y4m), ("mkv", &mkv), ("mp4", &mp4)] {
        let plan = media::plan_transcode_lossless(
            src,
            &LosslessTransform::default(),
            &CopyOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(plan.command, "transcode-lossless");
        let out = temp_dir.join(format!("{src_label}-ok.mkv"));
        let stats = media::transcode_lossless(
            src,
            &out,
            LosslessTransform::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(stats.video_frames, 2);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn y4m_trim_and_concat_enforce_packet_budget_before_packet_work_limit() {
    let dir = fixture_dir();
    let y4m = dir.join("lossless-textured.y4m");
    let mkv = dir.join("single-video-ffv1.mkv");
    let mp4 = dir.join("single-video-avc.mp4");

    let temp_dir = std::env::temp_dir().join(format!("fvid-batch252-y4m-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // When both max_packet_bytes is violated AND max_packets == Some(0),
    // plan_trim_y4m / trim and plan_concat_y4m / concat must report the packet byte budget error identically!
    let mut bad_both = CopyOptions::default();
    bad_both.max_packet_bytes = 8;
    bad_both.max_packets = Some(0);

    for (label, src, expected_substr) in [
        ("y4m", &y4m, "Y4M frame exceeds byte limit"),
        ("mkv", &mkv, "WebM packet exceeds budget"),
        ("mp4", &mp4, "MP4 packet exceeds budget"),
    ] {
        let trim_plan_err = media::plan_trim_y4m(src, 0, 500_000, &bad_both).unwrap_err();
        let out_trim = temp_dir.join(format!("trim-{label}.y4m"));
        let trim_exec_err = media::trim(src, &out_trim, 0, 500_000, &bad_both).unwrap_err();
        assert!(
            trim_plan_err.contains(expected_substr),
            "plan_trim_y4m({label}) returned: {trim_plan_err}"
        );
        assert_eq!(
            trim_exec_err, trim_plan_err,
            "trim vs plan_trim_y4m mismatch on {label}"
        );
        assert!(!out_trim.exists());

        let concat_sources = vec![src.clone(), src.clone()];
        let concat_plan_err = media::plan_concat_y4m(&concat_sources, &bad_both).unwrap_err();
        let out_concat = temp_dir.join(format!("concat-{label}.y4m"));
        let concat_exec_err = media::concat(&concat_sources, &out_concat, &bad_both).unwrap_err();
        assert!(
            concat_plan_err.contains(expected_substr),
            "plan_concat_y4m({label}) returned: {concat_plan_err}"
        );
        assert_eq!(
            concat_exec_err, concat_plan_err,
            "concat vs plan_concat_y4m mismatch on {label}"
        );
        assert!(!out_concat.exists());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}
