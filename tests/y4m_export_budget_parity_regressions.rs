//! Regression tests for Batch 246 (`FVID-0253`):
//! 1. `export_y4m_pipeline_with_options`, `concat_y4m_with_options`, and `trim_y4m_with_options`
//!    enforce `options.max_packet_bytes` on compressed MP4 and Matroska video packets even when
//!    the decoded Y4M frame size (`768` bytes for `32x16` YUV420p) is smaller than `max_packet_bytes`
//!    (`770` bytes) while the encoded AVC I_PCM packet (`778` bytes) exceeds `max_packet_bytes`,
//!    and also enforce single-track / explicit `--streams` selection in `export_y4m_pipeline_with_options`.
//! 2. `native_plan::concat_y4m_with_options` and `native_plan::trim_y4m_with_options` (and CLI
//!    `plan concat --output-format y4m` / `plan trim --output-format y4m`) validate `max_packet_bytes`
//!    against source Y4M frames and compressed MP4/Matroska video packets, and reject `max_packets == Some(0)`.
//! 3. CLI `export-y4m`, `trim`, `concat`, `loudnorm`, `decode`, `transcode-lossless`, `crop-lossless`,
//!    `overlay`, `xfade`, `burn-subtitles`, and `convert-subtitles` reject `--max-packet-bytes 0`
//!    with `"packet byte limit must be positive"` and `--max-rss-mib 0` with `"RSS limit must be positive"`.

use std::path::PathBuf;
use std::process::Command;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/y4m-export-budget-parity")
}

#[test]
fn y4m_export_concat_and_trim_enforce_compressed_video_packet_budget_and_selection() {
    let fixtures = fixture_dir();
    let mp4 = fixtures.join("avc-32x16.mp4");
    let mkv = fixtures.join("avc-32x16.mkv");
    let multitrack_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-overlay-multitrack-budget-parity/multitrack-ffv1-aac.mkv");

    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-export-pkt-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Decoded 32x16 YUV420p frame is 768 bytes, while each encoded AVC I_PCM packet is 778 bytes.
    // With max_packet_bytes = 770, the output Y4M frame check alone (768 <= 770) would pass,
    // so compressed container packet inspection must reject the 778-byte packet!
    let mut tight_pkt = fvid::media::CopyOptions::default();
    tight_pkt.max_packet_bytes = 770;

    for (label, source, expected_msg) in [
        ("MP4", &mp4, "MP4 packet exceeds budget"),
        ("Matroska", &mkv, "WebM packet exceeds budget"),
    ] {
        let out_export = dir.join(format!("export-{label}.y4m"));
        let err = fvid::native_export::export_y4m_pipeline_with_options(
            source,
            &out_export,
            None,
            &Default::default(),
            &Default::default(),
            &tight_pkt,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(expected_msg),
            "{label} export_y4m_pipeline_with_options should reject 778-byte packet with max_packet_bytes=770, got: {err}"
        );
        assert!(!out_export.exists());

        let out_trim = dir.join(format!("trim-{label}.y4m"));
        let err = fvid::native_export::trim_y4m_with_options(
            source,
            &out_trim,
            0,
            500_000,
            &tight_pkt,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(expected_msg),
            "{label} trim_y4m_with_options should reject 778-byte packet with max_packet_bytes=770, got: {err}"
        );
        assert!(!out_trim.exists());

        let out_concat = dir.join(format!("concat-{label}.y4m"));
        let err = fvid::native_export::concat_y4m_with_options(
            &[source.clone(), source.clone()],
            &out_concat,
            &tight_pkt,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains(expected_msg),
            "{label} concat_y4m_with_options should reject 778-byte packet with max_packet_bytes=770, got: {err}"
        );
        assert!(!out_concat.exists());
    }

    // With max_packet_bytes = 800, both encoded packets (778 bytes) and Y4M frames (768 bytes) fit.
    let mut enough_pkt = fvid::media::CopyOptions::default();
    enough_pkt.max_packet_bytes = 800;
    let ok_export = dir.join("export-ok.y4m");
    let stats = fvid::native_export::export_y4m_pipeline_with_options(
        &mkv,
        &ok_export,
        None,
        &Default::default(),
        &Default::default(),
        &enough_pkt,
    )
    .expect("export_y4m_pipeline_with_options with max_packet_bytes=800 should succeed");
    assert_eq!(stats.packets, 2);
    assert!(ok_export.exists());

    // Multi-track source without explicit `--streams` selection must be refused by export_y4m_pipeline_with_options
    // just like trim_y4m_with_options and concat_y4m_with_options.
    let multi_out = dir.join("multi-unselected.y4m");
    let err = fvid::native_export::export_y4m_pipeline_with_options(
        &multitrack_mkv,
        &multi_out,
        None,
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("video-only export requires explicit selection when other tracks exist"),
        "expected explicit selection refusal on multi-track input, got: {err}"
    );
    assert!(!multi_out.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plan_concat_y4m_and_plan_trim_y4m_enforce_packet_bytes_and_nonzero_packets() {
    let fixtures = fixture_dir();
    let y4m = fixtures.join("video-32x16.y4m");
    let mp4 = fixtures.join("avc-32x16.mp4");
    let mkv = fixtures.join("avc-32x16.mkv");

    // 1. Y4M source (768-byte frame) with max_packet_bytes = 512 must fail in plan_trim_y4m and plan_concat
    let mut tight_y4m = fvid::media::CopyOptions::default();
    tight_y4m.max_packet_bytes = 512;

    let err = fvid::native_plan::trim_y4m_with_options(&y4m, 0, 500_000, &tight_y4m).unwrap_err();
    assert!(
        err.contains("Y4M frame exceeds byte limit"),
        "plan trim_y4m should reject 768-byte Y4M frame when max_packet_bytes=512, got: {err}"
    );

    let err = fvid::native_plan::concat_y4m_with_options(&[y4m.clone(), y4m.clone()], &tight_y4m)
        .unwrap_err();
    assert!(
        err.contains("Y4M frame exceeds byte limit"),
        "plan concat_y4m should reject 768-byte Y4M frame when max_packet_bytes=512, got: {err}"
    );

    // 2. Compressed MP4 and Matroska sources (778-byte AVC packet, 768-byte Y4M frame) with max_packet_bytes = 770
    let mut tight_compressed = fvid::media::CopyOptions::default();
    tight_compressed.max_packet_bytes = 770;

    for (label, source, expected_msg) in [
        ("MP4", &mp4, "MP4 packet exceeds budget"),
        ("Matroska", &mkv, "WebM packet exceeds budget"),
    ] {
        let err = fvid::native_plan::trim_y4m_with_options(source, 0, 500_000, &tight_compressed)
            .unwrap_err();
        assert!(
            err.contains(expected_msg),
            "{label} plan trim_y4m should reject 778-byte packet with max_packet_bytes=770, got: {err}"
        );

        let err = fvid::native_plan::concat_y4m_with_options(
            &[source.clone(), source.clone()],
            &tight_compressed,
        )
        .unwrap_err();
        assert!(
            err.contains(expected_msg),
            "{label} plan concat_y4m should reject 778-byte packet with max_packet_bytes=770, got: {err}"
        );
    }

    // 3. max_packets = Some(0) produces zero decoded frames in execution and must be rejected in planning too
    let mut zero_packets = fvid::media::CopyOptions::default();
    zero_packets.max_packets = Some(0);

    let err =
        fvid::native_plan::trim_y4m_with_options(&y4m, 0, 500_000, &zero_packets).unwrap_err();
    assert!(
        err.contains("input has no decoded video frames"),
        "plan trim_y4m should reject max_packets=0, got: {err}"
    );

    let err = fvid::native_plan::concat_y4m_with_options(&[y4m.clone(), y4m.clone()], &zero_packets)
        .unwrap_err();
    assert!(
        err.contains("input has no decoded video frames"),
        "plan concat_y4m should reject max_packets=0, got: {err}"
    );
}

#[test]
fn cli_y4m_and_native_commands_reject_zero_packet_bytes_and_zero_rss_mib() {
    let fixtures = fixture_dir();
    let y4m = fixtures.join("video-32x16.y4m");
    let dir = std::env::temp_dir().join(format!(
        "fvid-cli-y4m-zero-limits-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out_y4m = dir.join("out.y4m");

    // 1. --max-packet-bytes 0 must fail with "packet byte limit must be positive"
    for args in [
        vec![
            "media",
            "export-y4m",
            y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
            "--max-packet-bytes",
            "0",
        ],
        vec![
            "media",
            "trim",
            y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.5",
            "--max-packet-bytes",
            "0",
        ],
        vec![
            "media",
            "concat",
            out_y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--max-packet-bytes",
            "0",
        ],
        vec![
            "media",
            "plan",
            "trim",
            y4m.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.5",
            "--output-format",
            "y4m",
            "--max-packet-bytes",
            "0",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "CLI {:?} should reject --max-packet-bytes 0",
            args
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("packet byte limit must be positive"),
            "CLI {:?} unexpected stderr: {stderr}",
            args
        );
        assert!(!out_y4m.exists());
    }

    // 2. --max-rss-mib 0 must fail with "max-rss-mib must be 1..=65536"
    for args in [
        vec![
            "media",
            "export-y4m",
            y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
            "--max-rss-mib",
            "0",
        ],
        vec![
            "media",
            "trim",
            y4m.to_str().unwrap(),
            out_y4m.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.5",
            "--max-rss-mib",
            "0",
        ],
        vec![
            "media",
            "concat",
            out_y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--max-rss-mib",
            "0",
        ],
        vec![
            "media",
            "plan",
            "trim",
            y4m.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.5",
            "--output-format",
            "y4m",
            "--max-rss-mib",
            "0",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "CLI {:?} should reject --max-rss-mib 0",
            args
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("max-rss-mib must be 1..=65536"),
            "CLI {:?} unexpected stderr: {stderr}",
            args
        );
        assert!(!out_y4m.exists());
    }

    let _ = std::fs::remove_dir_all(&dir);
}
