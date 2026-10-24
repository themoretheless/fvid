use fvid::media::{CopyOptions, LosslessTransform};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

#[test]
fn lossless_plan_enforces_multitrack_and_single_stream_controls_and_cli_routes_filters_with_controls()
{
    let y4m_main = fixture("lossless-plan-routing-main.y4m");
    let y4m_overlay = fixture("lossless-plan-routing-overlay.y4m");
    let mp4_av = fixture("shared-mp4-av-multiple.mp4");
    let webm_av = fixture("shared-vp9-companions.mkv");
    let ffv1_single = fixture("ffv1-gray-8.mkv");

    // 1. Multitrack MP4 and WebM plan_transcode_lossless must enforce max_packets, max_packet_bytes,
    //    stream selection, and stream metadata bounds just like transcode_lossless.
    for (label, source) in [("mp4", &mp4_av), ("webm", &webm_av)] {
        let too_few_packets = CopyOptions {
            max_packets: Some(1),
            ..Default::default()
        };
        let err =
            fvid::media::plan_transcode_lossless(source, &Default::default(), &too_few_packets, None)
                .expect_err(&format!("{label}: plan_transcode_lossless must reject max_packets=1"));
        assert!(
            err.contains("packet count exceeds limit"),
            "{label}: unexpected max_packets error: {err}"
        );

        let tiny_packet = CopyOptions {
            max_packet_bytes: 2,
            ..Default::default()
        };
        let err =
            fvid::media::plan_transcode_lossless(source, &Default::default(), &tiny_packet, None)
                .expect_err(&format!(
                    "{label}: plan_transcode_lossless must reject max_packet_bytes=2"
                ));
        assert!(
            err.contains("exceeds") || err.contains("limit"),
            "{label}: unexpected max_packet_bytes error: {err}"
        );

        let audio_only = CopyOptions {
            streams: vec![0, 2],
            ..Default::default()
        };
        let err =
            fvid::media::plan_transcode_lossless(source, &Default::default(), &audio_only, None)
                .expect_err(&format!(
                    "{label}: plan_transcode_lossless must reject audio-only stream selection"
                ));
        assert!(
            err.contains("selected video stream"),
            "{label}: unexpected audio-only selection error: {err}"
        );

        let dup_streams = CopyOptions {
            streams: vec![1, 1],
            ..Default::default()
        };
        let err =
            fvid::media::plan_transcode_lossless(source, &Default::default(), &dup_streams, None)
                .expect_err(&format!(
                    "{label}: plan_transcode_lossless must reject duplicate stream selection"
                ));
        assert!(
            err.contains("invalid or duplicate"),
            "{label}: unexpected duplicate selection error: {err}"
        );

        let bad_stream_meta = CopyOptions {
            stream_metadata_set: vec![(99, "title".into(), "missing".into())],
            ..Default::default()
        };
        assert!(
            fvid::media::plan_transcode_lossless(
                source,
                &Default::default(),
                &bad_stream_meta,
                None
            )
            .is_err(),
            "{label}: plan_transcode_lossless must reject out-of-range stream metadata"
        );
    }

    // 2. Valid multitrack plan_transcode_lossless reflects selected stream order, marks video as
    //    primary_video and companion audio as copy, and emits a metadata step when tags are edited.
    let reordered_meta = CopyOptions {
        streams: vec![2, 1],
        metadata_set: vec![("title".into(), "Lossless Multitrack Plan".into())],
        stream_metadata_set: vec![
            (1, "title".into(), "Video Track".into()),
            (2, "title".into(), "Audio Track".into()),
        ],
        max_packets: Some(64),
        ..Default::default()
    };
    for (label, source) in [("mp4", &mp4_av), ("webm", &webm_av)] {
        let plan = fvid::media::plan_transcode_lossless(
            source,
            &LosslessTransform {
                negate: Some(String::new()),
                ..Default::default()
            },
            &reordered_meta,
            None,
        )
        .unwrap_or_else(|e| panic!("{label} valid multitrack plan failed: {e}"));
        assert_eq!(plan.streams.len(), 2, "{label}: expected 2 selected streams");
        assert_eq!(plan.streams[0].index, 2);
        assert_eq!(plan.streams[0].media_type, "audio");
        assert_eq!(plan.streams[0].disposition, "copy");
        assert_eq!(plan.streams[1].index, 1);
        assert_eq!(plan.streams[1].media_type, "video");
        assert_eq!(plan.streams[1].disposition, "primary_video");
        assert!(
            plan.steps.iter().any(|s| s.action == "metadata"),
            "{label}: expected metadata step in plan: {:?}",
            plan.steps
        );
        assert!(
            plan.notes
                .iter()
                .any(|n| n.contains("packet work limit: Some(64)")),
            "{label}: expected budget note in plan: {:?}",
            plan.notes
        );
    }

    // 3. Single-stream Y4M and Matroska FFV1 plan_transcode_lossless must also enforce max_packets
    //    and max_packet_bytes.
    let y4m_pkt_err = fvid::media::plan_transcode_lossless(
        &y4m_main,
        &Default::default(),
        &CopyOptions {
            max_packets: Some(2),
            ..Default::default()
        },
        None,
    )
    .expect_err("Y4M with 3 frames must reject max_packets=2 in plan_transcode_lossless");
    assert!(
        y4m_pkt_err.contains("packet count exceeds limit"),
        "unexpected Y4M max_packets error: {y4m_pkt_err}"
    );

    let ffv1_pkt_err = fvid::media::plan_transcode_lossless(
        &ffv1_single,
        &Default::default(),
        &CopyOptions {
            max_packets: Some(1),
            ..Default::default()
        },
        None,
    )
    .expect_err("FFV1 with 2 frames must reject max_packets=1 in plan_transcode_lossless");
    assert!(
        ffv1_pkt_err.contains("packet count exceeds limit"),
        "unexpected FFV1 max_packets error: {ffv1_pkt_err}"
    );

    let ffv1_byte_err = fvid::media::plan_transcode_lossless(
        &ffv1_single,
        &Default::default(),
        &CopyOptions {
            max_packet_bytes: 2,
            ..Default::default()
        },
        None,
    )
    .expect_err("FFV1 must reject max_packet_bytes=2 in plan_transcode_lossless");
    assert!(
        ffv1_byte_err.contains("exceeds") || ffv1_byte_err.contains("limit"),
        "unexpected FFV1 max_packet_bytes error: {ffv1_byte_err}"
    );

    // 4. CLI: `plan crop-lossless` and `overlay` with both processing filters and custom controls
    //    must route through the owned pipeline without legacy fallback.
    let bin = env!("CARGO_BIN_EXE_fvid");
    let temp = std::env::temp_dir().join(format!(
        "fvid-lossless-plan-cli-routing-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let crop_plan_out = Command::new(bin)
        .args([
            "media",
            "plan",
            "crop-lossless",
            y4m_main.to_str().unwrap(),
            "--crop",
            "0:0:2:2",
            "--metadata",
            "title=CroppedPlan",
        ])
        .output()
        .unwrap();
    assert!(
        crop_plan_out.status.success(),
        "plan crop-lossless failed: {}",
        String::from_utf8_lossy(&crop_plan_out.stderr)
    );
    let crop_plan: serde_json::Value =
        serde_json::from_slice(&crop_plan_out.stdout).unwrap();
    assert_eq!(crop_plan["command"].as_str(), Some("crop-lossless"));
    assert!(
        crop_plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["action"].as_str() == Some("metadata"))
    );

    let overlay_plan_out = Command::new(bin)
        .args([
            "media",
            "plan",
            "overlay",
            y4m_main.to_str().unwrap(),
            y4m_overlay.to_str().unwrap(),
            "--hflip",
            "--negate",
            "",
            "--metadata",
            "title=OverlayFiltered",
        ])
        .output()
        .unwrap();
    assert!(
        overlay_plan_out.status.success(),
        "plan overlay with filter + metadata failed: {}",
        String::from_utf8_lossy(&overlay_plan_out.stderr)
    );
    let overlay_plan: serde_json::Value =
        serde_json::from_slice(&overlay_plan_out.stdout).unwrap();
    assert_eq!(overlay_plan["command"].as_str(), Some("overlay"));
    assert!(
        overlay_plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["action"].as_str() == Some("metadata"))
    );

    let overlay_mkv = temp.join("overlay-filtered.mkv");
    let overlay_exec_out = Command::new(bin)
        .args([
            "media",
            "overlay",
            y4m_main.to_str().unwrap(),
            y4m_overlay.to_str().unwrap(),
            overlay_mkv.to_str().unwrap(),
            "--hflip",
            "--negate",
            "",
            "--metadata",
            "title=OverlayFiltered",
        ])
        .output()
        .unwrap();
    assert!(
        overlay_exec_out.status.success(),
        "overlay with filter + metadata failed: {}",
        String::from_utf8_lossy(&overlay_exec_out.stderr)
    );
    let probed = fvid::media::probe(&overlay_mkv).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("OverlayFiltered")
    );

    let _ = std::fs::remove_dir_all(&temp);
}
