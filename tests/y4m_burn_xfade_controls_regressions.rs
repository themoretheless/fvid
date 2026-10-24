use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

#[test]
fn y4m_concat_trim_export_and_burn_xfade_honor_budgets_and_metadata_controls() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-burn-xfade-controls-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let primary = fixture("y4m-burn-xfade-primary.y4m");
    let secondary = fixture("y4m-burn-xfade-secondary.y4m");
    let subs = fixture("y4m-burn-xfade.srt");

    // 1. fvid::media::concat and plan_concat_y4m enforce RSS, packet-byte, and packet-count budgets
    let mut tight_rss = fvid::media::CopyOptions::default();
    tight_rss.max_rss_bytes = Some(1);

    let err = fvid::media::plan_concat_y4m(&[primary.clone(), secondary.clone()], &tight_rss)
        .unwrap_err();
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");

    let concat_rss_out = dir.join("concat-rss.y4m");
    let err = fvid::media::concat(
        &[primary.clone(), secondary.clone()],
        &concat_rss_out,
        &tight_rss,
    )
    .unwrap_err();
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");
    assert!(!concat_rss_out.exists());

    let mut tight_pkt = fvid::media::CopyOptions::default();
    tight_pkt.max_packet_bytes = 16;
    let concat_pkt_out = dir.join("concat-pkt.y4m");
    let err = fvid::media::concat(
        &[primary.clone(), secondary.clone()],
        &concat_pkt_out,
        &tight_pkt,
    )
    .unwrap_err();
    assert!(
        err.contains("exceeds byte limit") || err.contains("exceeds playback buffer budget"),
        "unexpected error: {err}"
    );
    assert!(!concat_pkt_out.exists());

    let mut two_pkts = fvid::media::CopyOptions::default();
    two_pkts.max_packets = Some(2);
    two_pkts.max_rss_bytes = Some(4096 * 1024 * 1024);
    let plan = fvid::media::plan_concat_y4m(&[primary.clone(), secondary.clone()], &two_pkts)
        .expect("plan_concat_y4m with budgets should succeed");
    assert!(plan.notes.iter().any(|n| n.contains("RSS limit:")));

    let concat_ok_out = dir.join("concat-ok.y4m");
    let stats = fvid::media::concat(
        &[primary.clone(), secondary.clone()],
        &concat_ok_out,
        &two_pkts,
    )
    .expect("concat to y4m with max_packets=2 should succeed");
    assert_eq!(stats.packets, 2);
    assert!(concat_ok_out.exists());

    // 2. fvid::media::trim and plan_trim_y4m enforce RSS, packet-byte, and packet-count budgets
    let err = fvid::media::plan_trim_y4m(&primary, 0, 120_000, &tight_rss).unwrap_err();
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");

    let trim_rss_out = dir.join("trim-rss.y4m");
    let err = fvid::media::trim(&primary, &trim_rss_out, 0, 120_000, &tight_rss).unwrap_err();
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");
    assert!(!trim_rss_out.exists());

    let mut one_pkt = fvid::media::CopyOptions::default();
    one_pkt.max_packets = Some(1);
    one_pkt.max_rss_bytes = Some(4096 * 1024 * 1024);
    let trim_plan = fvid::media::plan_trim_y4m(&primary, 0, 120_000, &one_pkt)
        .expect("plan_trim_y4m with budgets should succeed");
    assert!(trim_plan.notes.iter().any(|n| n.contains("RSS limit:")));

    let trim_ok_out = dir.join("trim-ok.y4m");
    let trim_stats = fvid::media::trim(&primary, &trim_ok_out, 0, 120_000, &one_pkt)
        .expect("trim to y4m with max_packets=1 should succeed");
    assert_eq!(trim_stats.packets, 1);
    assert!(trim_ok_out.exists());

    // 3. CLI concat (.y4m), trim (.y4m), and export-y4m honor budget flags
    let cli_concat_out = dir.join("cli-concat.y4m");
    let status = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "concat",
            cli_concat_out.to_str().unwrap(),
            primary.to_str().unwrap(),
            secondary.to_str().unwrap(),
            "--max-packets",
            "2",
            "--max-rss-mib",
            "4096",
            "--quiet",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli concat y4m failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(cli_concat_out.exists());

    let cli_trim_out = dir.join("cli-trim.y4m");
    let status = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "trim",
            primary.to_str().unwrap(),
            cli_trim_out.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.12",
            "--max-packets",
            "1",
            "--max-rss-mib",
            "4096",
            "--quiet",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli trim y4m failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(cli_trim_out.exists());

    let cli_export_out = dir.join("cli-export.y4m");
    let status = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "export-y4m",
            primary.to_str().unwrap(),
            cli_export_out.to_str().unwrap(),
            "--max-packets",
            "2",
            "--max-rss-mib",
            "4096",
            "--quiet",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli export-y4m failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(cli_export_out.exists());

    // 4. burn_subtitles and plan_burn_subtitles support container & stream metadata mutations and enforce max_packet_bytes > 0
    let mut zero_pkt = fvid::media::CopyOptions::default();
    zero_pkt.max_packet_bytes = 0;
    assert!(fvid::media::plan_burn_subtitles(&primary, &subs, &zero_pkt).is_err());

    let mut burn_meta = fvid::media::CopyOptions::default();
    burn_meta.streams = vec![0];
    burn_meta.max_rss_bytes = Some(4096 * 1024 * 1024);
    burn_meta
        .metadata_set
        .push(("title".into(), "Burned Title".into()));
    burn_meta
        .stream_metadata_set
        .push((0, "title".into(), "Subtitled Track".into()));
    burn_meta
        .stream_metadata_set
        .push((0, "language".into(), "rus".into()));

    let burn_plan = fvid::media::plan_burn_subtitles(&primary, &subs, &burn_meta)
        .expect("plan_burn_subtitles with metadata should succeed");
    assert_eq!(burn_plan.command, "burn-subtitles");

    let burn_out = dir.join("burn-meta.mkv");
    let burn_stats = fvid::media::burn_subtitles(&primary, &burn_out, &subs, &burn_meta)
        .expect("burn_subtitles with metadata should succeed");
    assert_eq!(burn_stats.video_frames, 3);
    let burn_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&burn_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(burn_reader.tags.title, "Burned Title");
    assert_eq!(burn_reader.tracks[0].name, "Subtitled Track");
    assert_eq!(burn_reader.tracks[0].language, "rus");

    let cli_burn_out = dir.join("cli-burn-meta.mkv");
    let status = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "burn-subtitles",
            primary.to_str().unwrap(),
            cli_burn_out.to_str().unwrap(),
            "--subs",
            subs.to_str().unwrap(),
            "--metadata",
            "title=CLI Burn Title",
            "--stream-metadata",
            "0:title=CLI Track",
            "--streams",
            "0",
            "--max-rss-mib",
            "4096",
            "--quiet",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli burn-subtitles with metadata failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let cli_burn_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&cli_burn_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(cli_burn_reader.tags.title, "CLI Burn Title");
    assert_eq!(cli_burn_reader.tracks[0].name, "CLI Track");

    // 5. plan_xfade and xfade_video check RSS budget during planning and accept stream 0 selection
    let err = fvid::media::plan_xfade(
        &primary,
        &secondary,
        "fade",
        40_000,
        40_000,
        &tight_rss,
    )
    .unwrap_err();
    assert!(err.contains("rss budget exceeded"), "unexpected error: {err}");

    let mut xfade_opts = fvid::media::CopyOptions::default();
    xfade_opts.streams = vec![0];
    xfade_opts.max_rss_bytes = Some(4096 * 1024 * 1024);
    let xfade_plan = fvid::media::plan_xfade(
        &primary,
        &secondary,
        "fade",
        40_000,
        40_000,
        &xfade_opts,
    )
    .expect("plan_xfade with streams=[0] should succeed");
    assert_eq!(xfade_plan.command, "xfade");

    let cli_xfade_out = dir.join("cli-xfade.mkv");
    let status = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "xfade",
            primary.to_str().unwrap(),
            secondary.to_str().unwrap(),
            cli_xfade_out.to_str().unwrap(),
            "--xfade-duration",
            "0.04",
            "--xfade-offset",
            "0.04",
            "--streams",
            "0",
            "--max-rss-mib",
            "4096",
            "--quiet",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "cli xfade with --streams 0 failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(cli_xfade_out.exists());
}
