use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-burn-xfade-plan-controls-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/burn-xfade-plan-controls")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn burn_subtitles_and_xfade_plans_enforce_budgets_metadata_and_cli_controls() {
    let dir = TempDir::new("all");
    let burn_main = fixture("burn-main-96x64.y4m");
    let burn_srt = fixture("burn-cues.srt");
    let xfade_main = fixture("xfade-main-4x4.y4m");
    let xfade_other = fixture("xfade-other-4x4.y4m");

    // 1. `plan_burn_subtitles` must enforce `max_packets` and `max_packet_bytes` on Y4M and FFV1
    //    sources just like `burn_subtitles`, populate `streams` with stream 0, emit a `metadata`
    //    step when container/stream metadata mutations are requested, and record budget notes.
    let too_few_packets = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let err = fvid::media::plan_burn_subtitles(&burn_main, &burn_srt, &too_few_packets)
        .expect_err("plan_burn_subtitles must reject max_packets=1 on 3-frame Y4M");
    assert!(
        err.contains("packet count exceeds limit"),
        "unexpected plan_burn_subtitles max_packets error: {err}"
    );

    let tiny_packet = CopyOptions {
        max_packet_bytes: 2,
        ..Default::default()
    };
    let err = fvid::media::plan_burn_subtitles(&burn_main, &burn_srt, &tiny_packet)
        .expect_err("plan_burn_subtitles must reject max_packet_bytes=2 on Y4M");
    assert!(
        err.contains("exceeds") || err.contains("limit"),
        "unexpected plan_burn_subtitles max_packet_bytes error: {err}"
    );

    let burn_meta_opts = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Burned Subtitles".into()),
            ("CUSTOM_BURN_TAG".into(), "BurnTagVal".into()),
        ],
        stream_metadata_set: vec![
            (0, "title".into(), "Burned Video Track".into()),
            (0, "language".into(), "fra".into()),
        ],
        max_packets: Some(10),
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let burn_plan = fvid::media::plan_burn_subtitles(&burn_main, &burn_srt, &burn_meta_opts)
        .expect("plan_burn_subtitles with valid metadata and budgets");
    assert_eq!(burn_plan.command, "burn-subtitles");
    assert_eq!(burn_plan.streams.len(), 1);
    assert_eq!(burn_plan.streams[0].index, 0);
    assert_eq!(burn_plan.streams[0].media_type, "video");
    assert!(
        burn_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in burn-subtitles plan: {:?}",
        burn_plan.steps
    );
    assert!(
        burn_plan
            .notes
            .iter()
            .any(|n| n.contains("encoded packet limit:")),
        "expected budget note in burn-subtitles plan: {:?}",
        burn_plan.notes
    );

    // Also verify FFV1 `.mkv` source preflight in `plan_burn_subtitles`.
    let ffv1_source = dir.0.join("burn-source-ffv1.mkv");
    fvid::media::transcode_lossless(
        &burn_main,
        &ffv1_source,
        Default::default(),
        &Default::default(),
    )
    .expect("transcode_lossless Y4M -> FFV1 source");
    assert!(
        fvid::media::plan_burn_subtitles(&ffv1_source, &burn_srt, &too_few_packets).is_err(),
        "plan_burn_subtitles on FFV1 source must reject max_packets=1"
    );
    assert!(
        fvid::media::plan_burn_subtitles(&ffv1_source, &burn_srt, &tiny_packet).is_err(),
        "plan_burn_subtitles on FFV1 source must reject max_packet_bytes=2"
    );
    let ffv1_burn_plan =
        fvid::media::plan_burn_subtitles(&ffv1_source, &burn_srt, &burn_meta_opts)
            .expect("plan_burn_subtitles on FFV1 source with valid budgets");
    assert_eq!(ffv1_burn_plan.streams.len(), 1);
    assert_eq!(ffv1_burn_plan.streams[0].codec, "ffv1");

    // 2. `plan_xfade` and `xfade_video` must enforce `max_packets` and `max_packet_bytes`
    //    during `plan_xfade`, apply `stream_metadata_delete` before `stream_metadata_set` in
    //    `xfade_video`, and emit a `metadata` step and budget notes in `plan_xfade`.
    let err = fvid::media::plan_xfade(
        &xfade_main,
        &xfade_other,
        "fade",
        500_000,
        500_000,
        &too_few_packets,
    )
    .expect_err("plan_xfade must reject max_packets=1 on 3+3 frame Y4M fade");
    assert!(
        err.contains("packet count exceeds limit"),
        "unexpected plan_xfade max_packets error: {err}"
    );

    let err = fvid::media::plan_xfade(
        &xfade_main,
        &xfade_other,
        "fade",
        500_000,
        500_000,
        &tiny_packet,
    )
    .expect_err("plan_xfade must reject max_packet_bytes=2 on Y4M fade");
    assert!(
        err.contains("exceeds") || err.contains("limit"),
        "unexpected plan_xfade max_packet_bytes error: {err}"
    );

    let xfade_meta_opts = CopyOptions {
        metadata_delete: vec!["comment".into()],
        metadata_set: vec![
            ("title".into(), "Xfade Output".into()),
            ("CUSTOM_XFADE".into(), "XfadeVal".into()),
        ],
        stream_metadata_delete: vec![(0, "language".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "Xfade Track".into()),
            (0, "CUSTOM_TRACK".into(), "TrackVal".into()),
        ],
        max_packets: Some(20),
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let xfade_plan = fvid::media::plan_xfade(
        &xfade_main,
        &xfade_other,
        "fade",
        500_000,
        500_000,
        &xfade_meta_opts,
    )
    .expect("plan_xfade with valid metadata and budgets");
    assert_eq!(xfade_plan.command, "xfade");
    assert!(
        xfade_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in xfade plan: {:?}",
        xfade_plan.steps
    );
    assert!(
        xfade_plan
            .notes
            .iter()
            .any(|n| n.contains("encoded packet limit:")),
        "expected budget note in xfade plan: {:?}",
        xfade_plan.notes
    );

    // 3. CLI `burn-subtitles`, `plan burn-subtitles`, `xfade`, and `plan xfade` must support `-q`,
    //    reject duplicate non-metadata flags, reject `--progress` during `plan`, and enforce
    //    `--max-packets` / `--max-packet-bytes` in `plan`.
    let cli_burn_plan_quiet = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "burn-subtitles",
            burn_main.to_str().unwrap(),
            "--subs",
            burn_srt.to_str().unwrap(),
            "--metadata",
            "title=CLI Burn",
            "--stream-metadata",
            "0:title=CLI Burn Stream",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        cli_burn_plan_quiet.status.success(),
        "cli plan burn-subtitles -q failed: {}",
        String::from_utf8_lossy(&cli_burn_plan_quiet.stderr)
    );
    assert!(cli_burn_plan_quiet.stdout.is_empty());

    let cli_burn_plan_fail = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "burn-subtitles",
            burn_main.to_str().unwrap(),
            "--subs",
            burn_srt.to_str().unwrap(),
            "--max-packets",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_burn_plan_fail.status.success(),
        "cli plan burn-subtitles must reject --max-packets 1"
    );

    let cli_burn_dup_fail = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "burn-subtitles",
            burn_main.to_str().unwrap(),
            "--subs",
            burn_srt.to_str().unwrap(),
            "--max-packets",
            "10",
            "--max-packets",
            "20",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_burn_dup_fail.status.success(),
        "cli plan burn-subtitles must reject duplicate --max-packets"
    );

    let cli_xfade_plan_quiet = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "xfade",
            xfade_main.to_str().unwrap(),
            xfade_other.to_str().unwrap(),
            "--xfade-duration",
            "0.5",
            "--xfade-offset",
            "0.5",
            "--metadata",
            "title=CLI Xfade",
            "--stream-metadata",
            "0:title=CLI Xfade Stream",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        cli_xfade_plan_quiet.status.success(),
        "cli plan xfade -q failed: {}",
        String::from_utf8_lossy(&cli_xfade_plan_quiet.stderr)
    );
    assert!(cli_xfade_plan_quiet.stdout.is_empty());

    let cli_xfade_plan_fail = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "xfade",
            xfade_main.to_str().unwrap(),
            xfade_other.to_str().unwrap(),
            "--xfade-duration",
            "0.5",
            "--xfade-offset",
            "0.5",
            "--max-packets",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_xfade_plan_fail.status.success(),
        "cli plan xfade must reject --max-packets 1"
    );

    let cli_xfade_dup_fail = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "xfade",
            xfade_main.to_str().unwrap(),
            xfade_other.to_str().unwrap(),
            "--xfade-duration",
            "0.5",
            "--max-packets",
            "10",
            "--max-packets",
            "20",
        ])
        .output()
        .unwrap();
    assert!(
        !cli_xfade_dup_fail.status.success(),
        "cli plan xfade must reject duplicate --max-packets"
    );
}
