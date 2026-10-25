use fvid::media::CopyOptions;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/remux-concat-controls")
        .join(name)
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-remux-concat-controls-{tag}-{}",
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

#[test]
fn companion_video_fixture_decodes_cleanly() {
    let stats = fvid::media::decode_video(&fixture("video.mkv")).unwrap();
    assert_eq!(stats.video_frames, 1);
}

#[test]
fn wave_concat_and_remux_support_budgets_and_metadata_in_public_api_and_cli() {
    let dir = TempDir::new("wave");
    let wav_a = fixture("pcm-48k-a.wav");
    let wav_b = fixture("pcm-48k-b.wav");
    let sources = vec![wav_a.clone(), wav_b.clone()];

    // 1. Public API `plan_concat` & `concat` on WAVE with packet cap & metadata edits.
    let options = CopyOptions {
        max_packets: Some(2),
        metadata_delete: vec!["title".into()],
        metadata_set: vec![("artist".into(), "FVid Concat Artist".into())],
        ..Default::default()
    };
    let plan = fvid::media::plan_concat(&sources, &options).unwrap();
    assert_eq!(plan.command, "concat");
    assert_eq!(plan.streams[0].codec, "pcm_s16le");
    assert!(plan
        .steps
        .iter()
        .any(|s| s.detail.contains("in 2 frame-aligned blocks")));

    let out_concat = dir.0.join("concat-capped.wav");
    let stats = fvid::media::concat(&sources, &out_concat, &options).unwrap();
    assert_eq!(stats.packets, 2);
    assert_eq!(stats.segments, 2);
    let info = fvid::media::probe(&out_concat).unwrap();
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("FVid Concat Artist")
    );
    assert!(!info.metadata.contains_key("title"));

    // 2. Tight memory and RSS budgets reject WAVE concat and plan_concat cleanly.
    let tight_mem = CopyOptions {
        max_controlled_bytes: Some(16),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&sources, &tight_mem).is_err());
    assert!(fvid::media::concat(&sources, &dir.0.join("never-mem.wav"), &tight_mem).is_err());

    let tight_rss = CopyOptions {
        max_rss_bytes: Some(1024 * 1024),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&sources, &tight_rss).is_err());
    assert!(fvid::media::concat(&sources, &dir.0.join("never-rss.wav"), &tight_rss).is_err());

    // 3. CLI `plan remux` and `remux` on WAVE with metadata and packet cap without `--features media`.
    let cli_plan_remux = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "remux"])
        .arg(&wav_a)
        .args([
            "--max-packets",
            "1",
            "--metadata-delete",
            "title",
            "--metadata",
            "artist=Edited WAVE Artist",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_remux.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_plan_remux.stderr)
    );
    let remux_plan_json: serde_json::Value =
        serde_json::from_slice(&cli_plan_remux.stdout).unwrap();
    assert_eq!(remux_plan_json["command"], "remux");

    let cli_remux_out = dir.0.join("cli-remux.wav");
    let cli_remux = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&wav_a)
        .arg(&cli_remux_out)
        .args([
            "--max-packets",
            "1",
            "--metadata-delete",
            "title",
            "--metadata",
            "artist=Edited WAVE Artist",
        ])
        .output()
        .unwrap();
    assert!(
        cli_remux.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_remux.stderr)
    );
    let cli_remux_info = fvid::media::probe(&cli_remux_out).unwrap();
    assert!(!cli_remux_info.metadata.contains_key("title"));
    assert_eq!(
        cli_remux_info.metadata.get("artist").map(String::as_str),
        Some("Edited WAVE Artist")
    );

    // 4. CLI `plan concat` and `concat` on WAVE with packet cap and metadata edits.
    let cli_plan_concat = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "concat"])
        .arg(&wav_a)
        .arg(&wav_b)
        .args([
            "--max-packets",
            "2",
            "--metadata-delete",
            "title",
            "--metadata",
            "artist=CLI Concat Artist",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_concat.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_plan_concat.stderr)
    );

    let cli_concat_out = dir.0.join("cli-concat.wav");
    let cli_concat = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&cli_concat_out)
        .arg(&wav_a)
        .arg(&wav_b)
        .args([
            "--max-packets",
            "2",
            "--metadata-delete",
            "title",
            "--metadata",
            "artist=CLI Concat Artist",
        ])
        .output()
        .unwrap();
    assert!(
        cli_concat.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_concat.stderr)
    );
    let cli_concat_info = fvid::media::probe(&cli_concat_out).unwrap();
    assert_eq!(
        cli_concat_info.metadata.get("artist").map(String::as_str),
        Some("CLI Concat Artist")
    );
}

#[test]
fn adts_remux_and_concat_enforce_packet_caps_and_byte_limits_in_api_and_cli() {
    let dir = TempDir::new("adts");
    let adts = fixture("adts-mono-48k.aac");
    let sources = vec![adts.clone(), adts.clone()];

    // 1. Public API `remux` on ADTS to `.mka` and `.m4a` with `max_packets: Some(2)`.
    let cap_opts = CopyOptions {
        max_packets: Some(2),
        ..Default::default()
    };
    let plan_remux = fvid::media::plan_remux(&adts, &cap_opts).unwrap();
    assert_eq!(plan_remux.command, "remux");

    let out_mka = dir.0.join("remux-cap.mka");
    let stats_mka = fvid::media::remux(&adts, &out_mka, &cap_opts).unwrap();
    assert_eq!(stats_mka.packets, 2);

    let out_m4a = dir.0.join("remux-cap.m4a");
    let stats_m4a = fvid::media::remux(&adts, &out_m4a, &cap_opts).unwrap();
    assert_eq!(stats_m4a.packets, 2);

    // 2. Public API `plan_concat` & `concat` on ADTS with `max_packets: Some(4)`.
    let concat_cap_opts = CopyOptions {
        max_packets: Some(4),
        ..Default::default()
    };
    let plan_concat = fvid::media::plan_concat(&sources, &concat_cap_opts).unwrap();
    assert_eq!(plan_concat.command, "concat");

    let concat_mka = dir.0.join("concat-cap.mka");
    let concat_mka_stats = fvid::media::concat(&sources, &concat_mka, &concat_cap_opts).unwrap();
    assert_eq!(concat_mka_stats.packets, 4);

    let concat_m4a = dir.0.join("concat-cap.m4a");
    let concat_m4a_stats = fvid::media::concat(&sources, &concat_m4a, &concat_cap_opts).unwrap();
    assert_eq!(concat_m4a_stats.packets, 4);

    // 3. Tight `max_packet_bytes: 1` rejects ADTS remux and concat in both plan and execution.
    let tight_pkt = CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    assert!(fvid::media::plan_remux(&adts, &tight_pkt).is_err());
    assert!(fvid::media::remux(&adts, &dir.0.join("never.mka"), &tight_pkt).is_err());
    assert!(fvid::media::remux(&adts, &dir.0.join("never.m4a"), &tight_pkt).is_err());
    assert!(fvid::media::plan_concat(&sources, &tight_pkt).is_err());
    assert!(fvid::media::concat(&sources, &dir.0.join("never-c.mka"), &tight_pkt).is_err());
    assert!(fvid::media::concat(&sources, &dir.0.join("never-c.m4a"), &tight_pkt).is_err());

    // 4. CLI `remux`, `plan remux`, `concat`, `plan concat` with `--max-packets` and `--max-packet-bytes`.
    let cli_remux_mka = dir.0.join("cli-adts.mka");
    let run_remux = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&adts)
        .arg(&cli_remux_mka)
        .args(["--max-packets", "2"])
        .output()
        .unwrap();
    assert!(
        run_remux.status.success(),
        "{}",
        String::from_utf8_lossy(&run_remux.stderr)
    );

    let cli_concat_m4a = dir.0.join("cli-concat.m4a");
    let run_concat = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&cli_concat_m4a)
        .arg(&adts)
        .arg(&adts)
        .args(["--max-packets", "4"])
        .output()
        .unwrap();
    assert!(
        run_concat.status.success(),
        "{}",
        String::from_utf8_lossy(&run_concat.stderr)
    );

    let fail_concat = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(dir.0.join("cli-fail.m4a"))
        .arg(&adts)
        .arg(&adts)
        .args(["--max-packet-bytes", "1"])
        .output()
        .unwrap();
    assert!(!fail_concat.status.success());
}

#[test]
fn sample_domain_and_mp4_matroska_concat_enforce_budgets_in_api_and_cli() {
    let dir = TempDir::new("sample-concat");
    let g711 = fixture("g711-alaw-48k.wav");
    let ogg = fixture("ogg-opus-48k.ogg");
    let mkv_flac = fixture("matroska-flac-48k.mka");
    let mp4_pcm = fixture("mp4-pcm-48k.m4a");
    let mixed = vec![g711.clone(), ogg.clone(), mkv_flac.clone(), mp4_pcm.clone()];

    // 1. Sample-domain concat enforces `max_packets`, `max_packet_bytes`, `max_controlled_bytes`, `max_rss_bytes`.
    let tight_pkt = CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&mixed, &tight_pkt).is_err());
    assert!(fvid::media::concat(&mixed, &dir.0.join("never-pkt.mka"), &tight_pkt).is_err());

    let tight_work = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&mixed, &tight_work).is_err());
    assert!(fvid::media::concat(&mixed, &dir.0.join("never-work.mka"), &tight_work).is_err());

    let tight_rss = CopyOptions {
        max_rss_bytes: Some(1024 * 1024),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&mixed, &tight_rss).is_err());
    assert!(fvid::media::concat(&mixed, &dir.0.join("never-rss.mka"), &tight_rss).is_err());

    let tight_mem = CopyOptions {
        max_controlled_bytes: Some(16 * 1024 * 1024),
        ..Default::default()
    };
    assert!(fvid::media::plan_concat(&mixed, &tight_mem).is_err());
    assert!(fvid::media::concat(&mixed, &dir.0.join("never-mem.mka"), &tight_mem).is_err());

    let generous = CopyOptions {
        max_packet_bytes: 65536,
        max_packets: Some(64),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };
    let plan = fvid::media::plan_concat(&mixed, &generous).unwrap();
    assert_eq!(plan.command, "concat");
    let out_mixed = dir.0.join("mixed-generous.mka");
    let stats = fvid::media::concat(&mixed, &out_mixed, &generous).unwrap();
    assert_eq!(stats.segments, 4);

    // 2. CLI sample-domain concat and plan concat enforce budget flags.
    let cli_fail = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(dir.0.join("cli-never.mka"))
        .args(&mixed)
        .args(["--max-packet-bytes", "1"])
        .output()
        .unwrap();
    assert!(!cli_fail.status.success());

    let cli_ok_out = dir.0.join("cli-mixed.mka");
    let cli_ok = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&cli_ok_out)
        .args(&mixed)
        .args(["--max-packet-bytes", "65536", "--max-packets", "64"])
        .output()
        .unwrap();
    assert!(
        cli_ok.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_ok.stderr)
    );
}

#[test]
fn remux_plan_adts_packet_scan_matroska_budget_step_and_plan_transcode_cli_parity() {
    let corrupt_adts = fixture("adts-corrupt-second-packet.aac");
    let valid_adts = fixture("adts-mono-48k.aac");
    let matroska = fixture("video.mkv");
    let mp4_pcm = fixture("mp4-pcm-48k.m4a");
    let y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/source-8x4.y4m");

    // 1. Corrupt ADTS second packet must be rejected by both `fvid::native_plan::remux`,
    //    `fvid::media::plan_remux`, and `fvid media plan remux` even when called with default options.
    let api_err = fvid::media::plan_remux(&corrupt_adts, &CopyOptions::default())
        .expect_err("fvid::media::plan_remux must reject truncated second ADTS packet");
    let native_err = fvid::native_plan::remux(&corrupt_adts)
        .expect_err("fvid::native_plan::remux must scan ADTS packets and reject truncated second packet");
    assert_eq!(native_err, api_err);

    let cli_corrupt = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "remux"])
        .arg(&corrupt_adts)
        .output()
        .unwrap();
    assert!(
        !cli_corrupt.status.success(),
        "fvid media plan remux must reject truncated second ADTS packet"
    );

    // 2. Matroska `fvid media plan remux` and `fvid::native_plan::remux` must match
    //    `fvid::media::plan_remux` (including the `budget` step).
    let expected_mkv_plan =
        serde_json::to_value(fvid::media::plan_remux(&matroska, &CopyOptions::default()).unwrap())
            .unwrap();
    assert_eq!(
        serde_json::to_value(fvid::native_plan::remux(&matroska).unwrap().unwrap()).unwrap(),
        expected_mkv_plan
    );
    let cli_mkv = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "remux"])
        .arg(&matroska)
        .output()
        .unwrap();
    assert!(
        cli_mkv.status.success(),
        "{}",
        String::from_utf8_lossy(&cli_mkv.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&cli_mkv.stdout).unwrap(),
        expected_mkv_plan
    );

    // 3. `fvid::media::plan_remux` must reject out-of-range stream selection on MP4, Matroska, and ADTS.
    let mut bad_stream_opts = CopyOptions::default();
    bad_stream_opts.streams = vec![99];
    assert!(fvid::media::plan_remux(&mp4_pcm, &bad_stream_opts).is_err());
    assert!(fvid::media::plan_remux(&matroska, &bad_stream_opts).is_err());
    assert!(fvid::media::plan_remux(&valid_adts, &bad_stream_opts).is_err());

    // 4. Shorthand `fvid media plan INPUT [--crop ...]` must work in default builds without `--features media`.
    let cli_shorthand_remux = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", matroska.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        cli_shorthand_remux.status.success(),
        "shorthand plan INPUT (remux) failed: {}",
        String::from_utf8_lossy(&cli_shorthand_remux.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&cli_shorthand_remux.stdout).unwrap(),
        expected_mkv_plan
    );

    let cli_shorthand_lossless = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "plan",
            y4m.to_str().unwrap(),
            "--crop",
            "0:0:4:2",
        ])
        .output()
        .unwrap();
    assert!(
        cli_shorthand_lossless.status.success(),
        "shorthand plan INPUT --crop failed: {}",
        String::from_utf8_lossy(&cli_shorthand_lossless.stderr)
    );
    let shorthand_lossless_json: serde_json::Value =
        serde_json::from_slice(&cli_shorthand_lossless.stdout).unwrap();
    assert_eq!(shorthand_lossless_json["command"], "transcode-lossless");

    // 5. CLI `plan remux`, `remux`, `plan loudness`, and `loudness`:
    //    - accept flags before positional arguments (`--quiet`, `--max-packets`, etc.)
    //    - `plan remux` accepts an optional destination path `INPUT [OUTPUT.wav|OUTPUT.mp4|OUTPUT.m4a|OUTPUT.mka|OUTPUT.mkv]`
    //      and validates destination-specific constraints (e.g. `.wav` rejects `--stream-metadata` and non-INFO keys;
    //      `.m4a` on ADTS rejects metadata mutations; `.mp3` is rejected)
    //    - `remux`, `plan remux`, `loudness`, and `plan loudness` reject missing/extra positional arguments or
    //      unsupported extensions directly instead of falling through to the feature-gate error.
    let wav_a = fixture("pcm-48k-a.wav");
    let dir = TempDir::new("cli-remux-loudness");

    for ok_args in [
        vec![
            "media",
            "plan",
            "remux",
            "--quiet",
            wav_a.to_str().unwrap(),
            dir.0.join("planned.wav").to_str().unwrap(),
            "--metadata",
            "title=Valid Wav Riff Info",
        ],
        vec![
            "media",
            "plan",
            "remux",
            "--max-packets",
            "1",
            valid_adts.to_str().unwrap(),
            dir.0.join("planned.mka").to_str().unwrap(),
            "--metadata",
            "CUSTOM_TAG=ValidMkaTag",
            "--stream-metadata",
            "0:title=TrackZero",
        ],
        vec![
            "media",
            "plan",
            "loudness",
            "--quiet",
            wav_a.to_str().unwrap(),
        ],
        vec![
            "media",
            "loudness",
            "--quiet",
            wav_a.to_str().unwrap(),
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&ok_args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected success for {:?}: {}",
            ok_args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for bad_args in [
        vec![
            "media",
            "plan",
            "remux",
            wav_a.to_str().unwrap(),
            dir.0.join("planned.wav").to_str().unwrap(),
            "--stream-metadata",
            "0:title=NotAllowedOnWav",
        ],
        vec![
            "media",
            "plan",
            "remux",
            wav_a.to_str().unwrap(),
            dir.0.join("planned.wav").to_str().unwrap(),
            "--metadata",
            "CUSTOM_NON_RIFF=NotAllowedOnWav",
        ],
        vec![
            "media",
            "plan",
            "remux",
            valid_adts.to_str().unwrap(),
            dir.0.join("planned.m4a").to_str().unwrap(),
            "--metadata",
            "title=NotAllowedOnAdtsMp4",
        ],
        vec![
            "media",
            "plan",
            "remux",
            wav_a.to_str().unwrap(),
            dir.0.join("planned.mp3").to_str().unwrap(),
        ],
        vec![
            "media",
            "remux",
            wav_a.to_str().unwrap(),
            dir.0.join("bad.mp3").to_str().unwrap(),
        ],
        vec![
            "media",
            "remux",
            wav_a.to_str().unwrap(),
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "loudness",
            "--quiet",
        ],
        vec![
            "media",
            "loudness",
            "--quiet",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&bad_args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for args {:?}",
            bad_args
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate fallback for {:?}: {stderr}",
            bad_args
        );
    }

    // 6. CLI `concat` and `plan concat`:
    //    - accept flags before positional arguments (`--quiet`, `--max-packets`, etc.)
    //    - validate destination-specific metadata constraints on `concat` (e.g. `.wav` rejects `--stream-metadata`
    //      and non-RIFF-INFO container keys; `.m4a` on ADTS rejects metadata mutations; `.mp3` is rejected)
    //    - reject missing/too-few positional arguments or unknown flags directly instead of falling through
    //      to the feature-gate error.
    let wav_b = fixture("pcm-48k-b.wav");
    let mp4_pcm = fixture("mp4-pcm-48k.m4a");
    for ok_concat_args in [
        vec![
            "media",
            "plan",
            "concat",
            "--quiet",
            "--max-packets",
            "2",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
        ],
        vec![
            "media",
            "concat",
            "--quiet",
            "--max-packets",
            "2",
            dir.0.join("flag-first-concat.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "title=FlagFirstWavConcat",
        ],
        vec![
            "media",
            "concat",
            "--quiet",
            dir.0.join("flag-first-concat.mka").to_str().unwrap(),
            mp4_pcm.to_str().unwrap(),
            mp4_pcm.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "title=PlannedWavConcatTitle",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.mka").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "CUSTOM_CONCAT_TAG=ValidMkaConcat",
            "--stream-metadata",
            "0:title=ConcatTrackZero",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.y4m").to_str().unwrap(),
            y4m.to_str().unwrap(),
            y4m.to_str().unwrap(),
            "--quiet",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&ok_concat_args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected success for {:?}: {}",
            ok_concat_args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for bad_concat_args in [
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--stream-metadata",
            "0:title=NotAllowedOnWavConcatPlan",
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "CUSTOM_NON_RIFF=NotAllowedOnWavConcatPlan",
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.m4a").to_str().unwrap(),
            valid_adts.to_str().unwrap(),
            valid_adts.to_str().unwrap(),
            "--metadata",
            "title=NotAllowedOnMp4ConcatPlan",
        ],
        vec![
            "media",
            "plan",
            "concat",
            "--output-format",
            "m4a",
            valid_adts.to_str().unwrap(),
            valid_adts.to_str().unwrap(),
            "--metadata",
            "title=NotAllowedOnMp4ConcatPlan",
        ],
        vec![
            "media",
            "plan",
            "concat",
            dir.0.join("planned-concat.mp3").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "concat",
            "--output-format",
            "mp3",
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
        ],
        vec![
            "media",
            "concat",
            dir.0.join("bad-concat.mp3").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
        ],
        vec![
            "media",
            "concat",
            dir.0.join("bad-concat-stream-meta.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--stream-metadata",
            "0:title=NotAllowedOnWav",
        ],
        vec![
            "media",
            "concat",
            dir.0.join("bad-concat-adts-meta.m4a").to_str().unwrap(),
            valid_adts.to_str().unwrap(),
            valid_adts.to_str().unwrap(),
            "--metadata",
            "title=NotAllowedOnAdtsMp4",
        ],
        vec![
            "media",
            "concat",
            dir.0.join("too-few.wav").to_str().unwrap(),
            wav_a.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "concat",
            wav_a.to_str().unwrap(),
        ],
        vec![
            "media",
            "concat",
            dir.0.join("unknown-flag.mka").to_str().unwrap(),
            mp4_pcm.to_str().unwrap(),
            mp4_pcm.to_str().unwrap(),
            "--unknown-concat-flag",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&bad_concat_args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for args {:?}",
            bad_concat_args
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate fallback for {:?}: {stderr}",
            bad_concat_args
        );
    }

    // 7. CLI `trim` and `plan trim`:
    //    - Y4M `trim` and `plan trim --output-format y4m` accept flags before positional paths
    //    - Matroska/MP4 audio `trim` with `--max-packet-bytes 1` or `--max-packets 0` fails with
    //      the budget diagnostic instead of falling through to the feature-gate error
    //    - `trim`, `trim-pcm`, `plan trim`, and `plan trim-pcm` with missing positional paths or
    //      unsupported destination extensions (`bad.mp3`) fail with direct CLI diagnostics.
    let mkv_flac = fixture("matroska-flac-48k.mka");
    for ok_trim_args in [
        vec![
            "media",
            "plan",
            "trim",
            "--quiet",
            "--output-format",
            "y4m",
            "--from",
            "0",
            "--to",
            "0.08",
            y4m.to_str().unwrap(),
        ],
        vec![
            "media",
            "trim",
            "--quiet",
            "--from",
            "0",
            "--to",
            "0.08",
            y4m.to_str().unwrap(),
            dir.0.join("trimmed.y4m").to_str().unwrap(),
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&ok_trim_args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected success for {:?}: {}",
            ok_trim_args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for bad_trim_args in [
        vec![
            "media",
            "trim",
            mkv_flac.to_str().unwrap(),
            dir.0.join("never-trim.wav").to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--max-packet-bytes",
            "1",
            "--quiet",
        ],
        vec![
            "media",
            "trim",
            mkv_flac.to_str().unwrap(),
            dir.0.join("bad-trim.mp3").to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
            "--quiet",
        ],
        vec!["media", "trim", "--from", "0", "--to", "0.02", "--quiet"],
        vec!["media", "trim-pcm", "--from", "0", "--to", "0.02", "--quiet"],
        vec!["media", "plan", "trim", "--from", "0", "--to", "0.02", "--quiet"],
        vec!["media", "plan", "trim-pcm", "--from", "0", "--to", "0.02", "--quiet"],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&bad_trim_args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for args {:?}",
            bad_trim_args
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate fallback for {:?}: {stderr}",
            bad_trim_args
        );
    }
}
