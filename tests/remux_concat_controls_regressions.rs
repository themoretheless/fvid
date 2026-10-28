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

#[test]
fn export_y4m_planning_and_remux_duplicate_streams_and_metadata_validation() {
    let dir = TempDir::new("export-y4m-remux-val");
    let wav_a = fixture("pcm-48k-a.wav");
    let mp4_aac = fixture("mp4-aac-48k-a.m4a");
    let y4m = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/source-8x4.y4m");

    // 1. Public API `native_plan::export_y4m_with_options` and CLI `plan export-y4m`
    //    validate geometry, pixel filters, interval, and budgets, and reject metadata mutations.
    let export_plan = fvid::native_plan::export_y4m_with_options(
        &y4m,
        Some((
            std::time::Duration::from_micros(0),
            std::time::Duration::from_micros(80_000),
        )),
        &fvid::native_geometry::VideoGeometry {
            horizontal_flip: true,
            ..Default::default()
        },
        &fvid::native_pixels::PixelFilters::default(),
        &CopyOptions {
            max_controlled_bytes: Some(1024 * 1024),
            ..Default::default()
        },
    )
    .expect("native_plan::export_y4m_with_options should succeed");
    assert_eq!(export_plan.command, "export-y4m");
    assert!(
        export_plan
            .steps
            .iter()
            .any(|s| s.action == "geometry" && s.detail.contains("horizontal flip true"))
    );
    assert!(
        export_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit: Some(1048576)"))
    );

    let cli_plan_export = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "plan",
            "export-y4m",
            y4m.to_str().unwrap(),
            dir.0.join("planned-export.y4m").to_str().unwrap(),
            "--hflip",
            "--from",
            "0",
            "--to",
            "0.08",
            "--max-memory-mib",
            "4",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_export.status.success(),
        "cli plan export-y4m failed: {}",
        String::from_utf8_lossy(&cli_plan_export.stderr)
    );
    let cli_plan_export_json: serde_json::Value =
        serde_json::from_slice(&cli_plan_export.stdout).unwrap();
    assert_eq!(cli_plan_export_json["command"], "export-y4m");

    // 2. Both `export-y4m` and `plan export-y4m` reject metadata mutations with
    //    `"Y4M output does not support metadata mutations"`.
    for meta_args in [
        vec![
            "media",
            "export-y4m",
            y4m.to_str().unwrap(),
            dir.0.join("never-meta.y4m").to_str().unwrap(),
            "--metadata",
            "title=Bad",
        ],
        vec![
            "media",
            "plan",
            "export-y4m",
            y4m.to_str().unwrap(),
            "--metadata-delete",
            "title",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&meta_args)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("Y4M output does not support metadata mutations"),
            "expected Y4M metadata refusal for {meta_args:?}, got: {stderr}"
        );
    }

    // 3. `remux` and `plan remux` reject duplicate `--streams` with `"duplicate streams option"`,
    //    and `remux` rejects WAVE stream metadata (`"WAVE remux does not support stream metadata edits"`)
    //    and `.mp4`/`.m4a` metadata mutations (`"remux to .mp4/.m4a does not support metadata mutations"`).
    for (args, expected_err) in [
        (
            vec![
                "media",
                "remux",
                wav_a.to_str().unwrap(),
                dir.0.join("never-dup.wav").to_str().unwrap(),
                "--streams",
                "0",
                "--streams",
                "0",
            ],
            "duplicate streams option",
        ),
        (
            vec![
                "media",
                "plan",
                "remux",
                wav_a.to_str().unwrap(),
                "--streams",
                "0",
                "--streams",
                "0",
            ],
            "duplicate streams option",
        ),
        (
            vec![
                "media",
                "remux",
                wav_a.to_str().unwrap(),
                dir.0.join("never-stream-meta.wav").to_str().unwrap(),
                "--stream-metadata",
                "0:title=Bad",
            ],
            "WAVE remux does not support stream metadata edits",
        ),
        (
            vec![
                "media",
                "remux",
                mp4_aac.to_str().unwrap(),
                dir.0.join("never-mp4-meta.m4a").to_str().unwrap(),
                "--metadata",
                "title=Bad",
            ],
            "remux to .mp4/.m4a does not support metadata mutations",
        ),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for {args:?}"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(expected_err),
            "expected `{expected_err}` for {args:?}, got: {stderr}"
        );
    }
}

#[test]
fn remux_and_concat_admit_and_enforce_controlled_memory_budgets_across_matroska_mp4_and_adts() {
    let dir = TempDir::new("remux-concat-mem-budget");
    let matroska = fixture("video.mkv");
    let adts = fixture("adts-mono-48k.aac");
    let mp4_aac_a = dir.0.join("source-mp4-aac-a.m4a");
    let mp4_aac_b = dir.0.join("source-mp4-aac-b.m4a");
    fvid::media::remux(&adts, &mp4_aac_a, &CopyOptions::default()).unwrap();
    fvid::media::remux(&adts, &mp4_aac_b, &CopyOptions::default()).unwrap();
    let adts_sources = vec![adts.clone(), adts.clone()];
    let mp4_sources = vec![mp4_aac_a.clone(), mp4_aac_b.clone()];

    let generous = CopyOptions {
        max_controlled_bytes: Some(4 * 1024 * 1024),
        ..Default::default()
    };
    let tight = CopyOptions {
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };

    // 1. Matroska identity remux (`plan_remux` and `remux`).
    let mkv_plan = fvid::media::plan_remux(&matroska, &generous)
        .expect("Matroska plan_remux should accept generous controlled memory budget");
    assert!(
        mkv_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit: Some(4194304)")),
        "unexpected notes: {:?}",
        mkv_plan.notes
    );
    let mkv_out = dir.0.join("identity-mem.mkv");
    let mkv_stats = fvid::media::remux(&matroska, &mkv_out, &generous)
        .expect("Matroska remux should accept generous controlled memory budget");
    assert!(mkv_stats.packets >= 1);
    assert!(fvid::media::plan_remux(&matroska, &tight).is_err());
    let mkv_tight_out = dir.0.join("identity-tight.mkv");
    assert!(fvid::media::remux(&matroska, &mkv_tight_out, &tight).is_err());
    assert!(!mkv_tight_out.exists());

    // 2. MP4 fast-start relocation (`plan_remux` and `remux` to `.m4a`).
    let mp4_reloc_plan = fvid::media::plan_remux(&mp4_aac_a, &generous)
        .expect("MP4 fast-start plan_remux should accept generous controlled memory budget");
    assert!(
        mp4_reloc_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit: Some(4194304)")),
        "unexpected notes: {:?}",
        mp4_reloc_plan.notes
    );
    let mp4_reloc_out = dir.0.join("reloc-mem.m4a");
    fvid::media::remux(&mp4_aac_a, &mp4_reloc_out, &generous)
        .expect("MP4 fast-start remux should accept generous controlled memory budget");
    assert!(mp4_reloc_out.exists());
    assert!(fvid::media::plan_remux(&mp4_aac_a, &tight).is_err());
    let mp4_reloc_tight_out = dir.0.join("reloc-tight.m4a");
    assert!(fvid::media::remux(&mp4_aac_a, &mp4_reloc_tight_out, &tight).is_err());
    assert!(!mp4_reloc_tight_out.exists());

    // 3. MP4-to-Matroska remux (`fvid_media::plan_remux`, `fvid_media::remux`, `fvid::media::remux` to `.mka`).
    let mp4_mka_plan = fvid_media::plan_remux(&mp4_aac_a, &generous)
        .expect("MP4-to-Matroska plan_remux should accept generous controlled memory budget");
    assert!(
        mp4_mka_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit:")),
        "unexpected notes: {:?}",
        mp4_mka_plan.notes
    );
    let mp4_mka_out = dir.0.join("mp4-to-mka-mem.mka");
    let mp4_mka_stats = fvid::media::remux(&mp4_aac_a, &mp4_mka_out, &generous)
        .expect("MP4-to-Matroska remux should accept generous controlled memory budget");
    assert!(mp4_mka_stats.packets >= 1);
    assert!(fvid_media::plan_remux(&mp4_aac_a, &tight).is_err());
    let mp4_mka_tight_out = dir.0.join("mp4-to-mka-tight.mka");
    assert!(fvid::media::remux(&mp4_aac_a, &mp4_mka_tight_out, &tight).is_err());
    assert!(!mp4_mka_tight_out.exists());

    // 4. Compressed MP4-to-Matroska concat (`plan_concat` and `concat` to `.mka`).
    let mp4_concat_plan = fvid::media::plan_concat(&mp4_sources, &generous)
        .expect("MP4-to-Matroska plan_concat should accept generous controlled memory budget");
    assert!(
        mp4_concat_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit: Some(4194304)")),
        "unexpected notes: {:?}",
        mp4_concat_plan.notes
    );
    let mp4_concat_out = dir.0.join("mp4-concat-mem.mka");
    let mp4_concat_stats = fvid::media::concat(&mp4_sources, &mp4_concat_out, &generous)
        .expect("MP4-to-Matroska concat should accept generous controlled memory budget");
    assert_eq!(mp4_concat_stats.segments, 2);
    assert!(fvid::media::plan_concat(&mp4_sources, &tight).is_err());
    let mp4_concat_tight_out = dir.0.join("mp4-concat-tight.mka");
    assert!(fvid::media::concat(&mp4_sources, &mp4_concat_tight_out, &tight).is_err());
    assert!(!mp4_concat_tight_out.exists());

    // 5. ADTS remux & concat (`plan_remux`, `remux`, `plan_concat`, `concat` to `.mka` and `.m4a`).
    let adts_plan = fvid::media::plan_remux(&adts, &generous)
        .expect("ADTS plan_remux should accept generous controlled memory budget");
    assert!(
        adts_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit:")),
        "unexpected notes: {:?}",
        adts_plan.notes
    );
    let adts_mka_out = dir.0.join("adts-remux-mem.mka");
    fvid::media::remux(&adts, &adts_mka_out, &generous)
        .expect("ADTS remux to .mka should accept generous controlled memory budget");
    let adts_m4a_out = dir.0.join("adts-remux-mem.m4a");
    fvid::media::remux(&adts, &adts_m4a_out, &generous)
        .expect("ADTS remux to .m4a should accept generous controlled memory budget");
    assert!(fvid::media::plan_remux(&adts, &tight).is_err());
    let adts_tight_mka = dir.0.join("adts-tight.mka");
    assert!(fvid::media::remux(&adts, &adts_tight_mka, &tight).is_err());
    assert!(!adts_tight_mka.exists());

    let adts_concat_plan = fvid::media::plan_concat(&adts_sources, &generous)
        .expect("ADTS plan_concat should accept generous controlled memory budget");
    assert!(
        adts_concat_plan
            .notes
            .iter()
            .any(|n| n.contains("controlled memory limit:")),
        "unexpected notes: {:?}",
        adts_concat_plan.notes
    );
    let adts_concat_mka = dir.0.join("adts-concat-mem.mka");
    fvid::media::concat(&adts_sources, &adts_concat_mka, &generous)
        .expect("ADTS concat to .mka should accept generous controlled memory budget");
    let adts_concat_m4a = dir.0.join("adts-concat-mem.m4a");
    fvid::media::concat(&adts_sources, &adts_concat_m4a, &generous)
        .expect("ADTS concat to .m4a should accept generous controlled memory budget");
    assert!(fvid::media::plan_concat(&adts_sources, &tight).is_err());
    let adts_concat_tight = dir.0.join("adts-concat-tight.mka");
    assert!(fvid::media::concat(&adts_sources, &adts_concat_tight, &tight).is_err());
    assert!(!adts_concat_tight.exists());

    // 6. CLI `remux`, `plan remux`, `concat`, and `plan concat` with `--max-memory-mib 4`.
    for cli_args in [
        vec![
            "media",
            "plan",
            "remux",
            matroska.to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "remux",
            matroska.to_str().unwrap(),
            dir.0.join("cli-mkv.mkv").to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "remux",
            mp4_aac_a.to_str().unwrap(),
            dir.0.join("cli-mp4.m4a").to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "remux",
            mp4_aac_a.to_str().unwrap(),
            dir.0.join("cli-mp4-mka.mka").to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "remux",
            adts.to_str().unwrap(),
            dir.0.join("cli-adts.mka").to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "concat",
            mp4_aac_a.to_str().unwrap(),
            mp4_aac_b.to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "concat",
            dir.0.join("cli-mp4-concat.mka").to_str().unwrap(),
            mp4_aac_a.to_str().unwrap(),
            mp4_aac_b.to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
        vec![
            "media",
            "concat",
            dir.0.join("cli-adts-concat.m4a").to_str().unwrap(),
            adts.to_str().unwrap(),
            adts.to_str().unwrap(),
            "--max-memory-mib",
            "4",
            "--quiet",
        ],
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(&cli_args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected CLI success for {cli_args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn y4m_video_export_trim_and_concat_enforce_controlled_memory_budgets_across_containers() {
    let dir = TempDir::new("y4m-mem-budget");
    let mkv_video = fixture("video.mkv");
    let mp4_video = fixture("video.mp4");
    let y4m_video = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/source-8x4.y4m");

    let tight_mem = CopyOptions {
        max_controlled_bytes: Some(16),
        ..Default::default()
    };

    for (label, source) in [
        ("y4m", y4m_video.as_path()),
        ("mkv", mkv_video.as_path()),
        ("mp4", mp4_video.as_path()),
    ] {
        assert!(
            fvid::native_plan::export_y4m_with_options(
                source,
                None,
                &fvid::native_geometry::VideoGeometry::default(),
                &fvid::native_pixels::PixelFilters::default(),
                &tight_mem,
            )
            .is_err(),
            "export_y4m_with_options must reject tight max_controlled_bytes on {label}"
        );
        let out_export = dir.0.join(format!("export-tight-{label}.y4m"));
        assert!(
            fvid::native_export::export_y4m_pipeline_with_options(
                source,
                &out_export,
                None,
                &fvid::native_geometry::VideoGeometry::default(),
                &fvid::native_pixels::PixelFilters::default(),
                &tight_mem,
            )
            .is_err(),
            "export_y4m_pipeline_with_options must reject tight max_controlled_bytes on {label}"
        );
        assert!(!out_export.exists());

        assert!(
            fvid::media::plan_trim_y4m(source, 0, 80_000, &tight_mem).is_err(),
            "plan_trim_y4m must reject tight max_controlled_bytes on {label}"
        );
        let out_trim = dir.0.join(format!("trim-tight-{label}.y4m"));
        assert!(
            fvid::media::trim(source, &out_trim, 0, 80_000, &tight_mem).is_err(),
            "trim to .y4m must reject tight max_controlled_bytes on {label}"
        );
        assert!(!out_trim.exists());

        let concat_sources = vec![source.to_path_buf(), source.to_path_buf()];
        assert!(
            fvid::media::plan_concat_y4m(&concat_sources, &tight_mem).is_err(),
            "plan_concat_y4m must reject tight max_controlled_bytes on {label}"
        );
        let out_concat = dir.0.join(format!("concat-tight-{label}.y4m"));
        assert!(
            fvid::media::concat(&concat_sources, &out_concat, &tight_mem).is_err(),
            "concat to .y4m must reject tight max_controlled_bytes on {label}"
        );
        assert!(!out_concat.exists());
    }

    // Also verify Y4M upscale transform where source frame fits in max_controlled_bytes (e.g. 64 bytes)
    // but transformed output frame (64x64 = 6144 bytes) exceeds max_controlled_bytes.
    let mid_mem = CopyOptions {
        max_controlled_bytes: Some(256),
        ..Default::default()
    };
    let upscale = fvid::native_geometry::VideoGeometry {
        scale: Some([64, 64]),
        ..Default::default()
    };
    assert!(
        fvid::native_plan::export_y4m_with_options(
            &y4m_video,
            None,
            &upscale,
            &fvid::native_pixels::PixelFilters::default(),
            &mid_mem,
        )
        .is_err(),
        "export_y4m_with_options must reject upscale output frame exceeding max_controlled_bytes"
    );
    let out_upscale = dir.0.join("upscale-tight.y4m");
    assert!(
        fvid::native_export::export_y4m_pipeline_with_options(
            &y4m_video,
            &out_upscale,
            None,
            &upscale,
            &fvid::native_pixels::PixelFilters::default(),
            &mid_mem,
        )
        .is_err(),
        "export_y4m_pipeline_with_options must reject upscale output frame exceeding max_controlled_bytes"
    );
    assert!(!out_upscale.exists());
}

#[test]
fn adts_and_mp4_to_matroska_remux_and_concat_enforce_controlled_memory_for_metadata_and_sample_tables() {
    let dir = TempDir::new("adts-mp4-mkv-mem-budget");
    let adts_src = fixture("adts-mono-48k.aac");
    let mp4_aac = dir.0.join("source-mp4-aac.m4a");
    fvid::media::remux(&adts_src, &mp4_aac, &CopyOptions::default()).unwrap();

    // 32 container tags + 32 stream 0 tags of 900 bytes each (~60 KiB raw, ~480 KiB serialized/buffered working set).
    let metadata_set: Vec<(String, String)> = (0..32)
        .map(|i| (format!("CONTAINER_TAG_{i:02}"), "c".repeat(900)))
        .collect();
    let stream_metadata_set: Vec<(usize, String, String)> = (0..32)
        .map(|i| (0, format!("STREAM_TAG_{i:02}"), "s".repeat(900)))
        .collect();

    let tight_meta = CopyOptions {
        metadata_set: metadata_set.clone(),
        stream_metadata_set: stream_metadata_set.clone(),
        max_controlled_bytes: Some(128 * 1024),
        ..Default::default()
    };
    let generous_meta = CopyOptions {
        metadata_set: metadata_set.clone(),
        stream_metadata_set: stream_metadata_set.clone(),
        max_controlled_bytes: Some(4 * 1024 * 1024),
        ..Default::default()
    };

    // 1. ADTS -> Matroska plan_remux & remux must enforce max_controlled_bytes on container/stream metadata
    assert!(
        fvid::media::plan_remux(&adts_src, &tight_meta).is_err(),
        "ADTS plan_remux must reject metadata working set exceeding max_controlled_bytes"
    );
    for ext in ["mka", "mkv"] {
        let out_remux = dir.0.join(format!("adts-tight.{ext}"));
        assert!(
            fvid::media::remux(&adts_src, &out_remux, &tight_meta).is_err(),
            "ADTS remux to .{ext} must reject metadata working set exceeding max_controlled_bytes"
        );
        assert!(!out_remux.exists());

        let out_ok = dir.0.join(format!("adts-ok.{ext}"));
        fvid::media::remux(&adts_src, &out_ok, &generous_meta)
            .expect("ADTS remux with generous max_controlled_bytes should succeed");
        assert!(out_ok.exists());
    }
    fvid::media::plan_remux(&adts_src, &generous_meta)
        .expect("ADTS plan_remux with generous max_controlled_bytes should succeed");

    // 2. ADTS -> Matroska plan_concat & concat must enforce max_controlled_bytes on container/stream metadata
    let adts_sources = vec![adts_src.clone(), adts_src.clone()];
    assert!(
        fvid::media::plan_concat(&adts_sources, &tight_meta).is_err(),
        "ADTS plan_concat must reject metadata working set exceeding max_controlled_bytes"
    );
    for ext in ["mka", "mkv"] {
        let out_concat = dir.0.join(format!("adts-concat-tight.{ext}"));
        assert!(
            fvid::media::concat(&adts_sources, &out_concat, &tight_meta).is_err(),
            "ADTS concat to .{ext} must reject metadata working set exceeding max_controlled_bytes"
        );
        assert!(!out_concat.exists());

        let out_ok = dir.0.join(format!("adts-concat-ok.{ext}"));
        fvid::media::concat(&adts_sources, &out_ok, &generous_meta)
            .expect("ADTS concat with generous max_controlled_bytes should succeed");
        assert!(out_ok.exists());
    }
    fvid::media::plan_concat(&adts_sources, &generous_meta)
        .expect("ADTS plan_concat with generous max_controlled_bytes should succeed");

    // 3. MP4 -> Matroska plan_remux & remux must enforce max_controlled_bytes on container/stream metadata
    for (label, src, ext) in [
        ("mp4-aac-mka", mp4_aac.as_path(), "mka"),
        ("mp4-aac-mkv", mp4_aac.as_path(), "mkv"),
    ] {
        assert!(
            fvid_media::plan_remux(src, &tight_meta).is_err(),
            "{label} fvid_media::plan_remux must reject metadata working set exceeding max_controlled_bytes"
        );
        let out_remux = dir.0.join(format!("{label}-tight.{ext}"));
        assert!(
            fvid::media::remux(src, &out_remux, &tight_meta).is_err(),
            "{label} remux to .{ext} must reject metadata working set exceeding max_controlled_bytes"
        );
        assert!(!out_remux.exists());

        let out_ok = dir.0.join(format!("{label}-ok.{ext}"));
        fvid::media::remux(src, &out_ok, &generous_meta)
            .expect("MP4->Matroska remux with generous max_controlled_bytes should succeed");
        assert!(out_ok.exists());
    }

    // 4. ADTS -> MP4 (.m4a) remux & concat must enforce max_controlled_bytes on accumulated MP4 sample-table (stsz/moov) working set
    let adts_frame = std::fs::read(&adts_src).unwrap();
    let adts_many = dir.0.join("many.aac");
    std::fs::write(&adts_many, adts_frame.repeat(300)).unwrap();
    // 600 packets * 32 bytes/packet + 16384 = 35,584 bytes > 24,576 bytes (24 KiB)
    let tight_mp4_table = CopyOptions {
        max_controlled_bytes: Some(24 * 1024),
        ..Default::default()
    };
    assert!(
        fvid::media::plan_remux(&adts_many, &tight_mp4_table).is_err(),
        "ADTS plan_remux must reject MP4 sample-table working set exceeding max_controlled_bytes"
    );
    let out_m4a_tight = dir.0.join("adts-many-tight.m4a");
    assert!(
        fvid::media::remux(&adts_many, &out_m4a_tight, &tight_mp4_table).is_err(),
        "ADTS remux to .m4a must reject MP4 sample-table working set exceeding max_controlled_bytes"
    );
    assert!(!out_m4a_tight.exists());

    let adts_many_sources = vec![adts_many.clone(), adts_many.clone()];
    let tight_concat_mp4_table = CopyOptions {
        max_controlled_bytes: Some(48 * 1024),
        ..Default::default()
    };
    assert!(
        fvid::media::plan_concat(&adts_many_sources, &tight_concat_mp4_table).is_err(),
        "ADTS plan_concat must reject MP4 sample-table working set exceeding max_controlled_bytes"
    );
    let out_concat_m4a_tight = dir.0.join("adts-many-concat-tight.m4a");
    assert!(
        fvid::media::concat(&adts_many_sources, &out_concat_m4a_tight, &tight_concat_mp4_table)
            .is_err(),
        "ADTS concat to .m4a must reject MP4 sample-table working set exceeding max_controlled_bytes"
    );
    assert!(!out_concat_m4a_tight.exists());
}




