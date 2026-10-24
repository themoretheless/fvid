use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
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
        .join("tests/fixtures/playback-errors/loudnorm-matroska-metadata")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn loudnorm_and_plan_loudnorm_support_matroska_output_and_container_stream_metadata() {
    let dir = TempDir::new("loudnorm-matroska-meta");
    let wav = fixture("tone-tagged-48k.wav");
    let aac = fixture("tone-48k.aac");
    let mp3 = fixture("tone-tagged-48k.mp3");

    let mut mka_opts = fvid::media::CopyOptions::default();
    mka_opts.metadata_delete = vec!["comment".into()];
    mka_opts.metadata_set = vec![
        ("title".into(), "Loudnorm Matroska Master".into()),
        ("LOUDNORM_TAG".into(), "ebu-r128".into()),
    ];
    mka_opts.stream_metadata_delete = vec![(0, "DROP_TRACK_TAG".into())];
    mka_opts.stream_metadata_set = vec![
        (0, "title".into(), "Normalized Audio Track 0".into()),
        (0, "language".into(), "deu".into()),
        (0, "TRACK_LOUDNORM".into(), "dual-pass".into()),
    ];

    // 1. plan_loudnorm on WAVE, ADTS AAC, and bare MP3 with Matroska container & stream metadata
    for (src, dual) in [(&wav, false), (&aac, true), (&mp3, true)] {
        let plan = fvid::media::plan_loudnorm(src, Some("I=-16:TP=-1.5:LRA=11"), dual, &mka_opts)
            .unwrap_or_else(|e| panic!("plan_loudnorm failed on {}: {e}", src.display()));
        assert_eq!(plan.command, "loudnorm");
        assert!(
            plan.steps.iter().any(|s| s.action == "metadata"),
            "expected metadata step in plan_loudnorm for {}",
            src.display()
        );
    }

    // 2. apply_loudnorm (single-pass linear and dynamic) and apply_loudnorm_dual to .mka and .mkv
    let wav_mka = dir.0.join("wav-linear.mka");
    let linear_args = "I=-16:TP=-1.5:LRA=11:measured_I=-22:measured_TP=-12:measured_LRA=2:measured_thresh=-32:linear=true";
    let wav_stats = fvid::media::apply_loudnorm(&wav, &wav_mka, Some(linear_args), &mka_opts)
        .expect("apply_loudnorm WAVE -> .mka failed");
    assert_eq!(wav_stats.sample_frames, 24_000);
    assert_eq!(wav_stats.sample_rate, 48_000);
    assert!(!wav_stats.dual_pass);

    let wav_mka_probed = fvid::media::probe(&wav_mka).unwrap();
    assert_eq!(
        wav_mka_probed.metadata.get("title").map(String::as_str),
        Some("Loudnorm Matroska Master")
    );
    assert_eq!(
        wav_mka_probed
            .metadata
            .get("LOUDNORM_TAG")
            .map(String::as_str),
        Some("ebu-r128")
    );
    assert_eq!(
        wav_mka_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Normalized Audio Track 0")
    );
    assert_eq!(
        wav_mka_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        wav_mka_probed.streams[0]
            .metadata
            .get("TRACK_LOUDNORM")
            .map(String::as_str),
        Some("dual-pass")
    );

    // Dual-pass ADTS AAC -> .mkv via API
    let aac_mkv = dir.0.join("aac-dual.mkv");
    let aac_stats = fvid::media::apply_loudnorm_dual(&aac, &aac_mkv, None, &mka_opts)
        .expect("apply_loudnorm_dual ADTS -> .mkv failed");
    assert!(aac_stats.dual_pass);
    assert!(aac_stats.sample_frames > 0);
    let aac_mkv_probed = fvid::media::probe(&aac_mkv).unwrap();
    assert_eq!(
        aac_mkv_probed.metadata.get("title").map(String::as_str),
        Some("Loudnorm Matroska Master")
    );
    assert_eq!(
        aac_mkv_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );

    // 3. CLI `plan loudnorm` and `loudnorm` to `.mka` with `--stream-metadata`, `--stream-metadata-delete`, and `--progress`
    let cli_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "loudnorm",
            mp3.to_str().unwrap(),
            "--dual-pass",
            "--metadata",
            "title=CLI Loudnorm MP3 MKA",
            "--metadata",
            "CUSTOM_MKA_KEY=cli-mka-val",
            "--metadata-delete",
            "comment",
            "--stream-metadata",
            "0:title=CLI MP3 Stream 0",
            "--stream-metadata",
            "0:language=fra",
            "--stream-metadata",
            "0:CUSTOM_STREAM_KEY=cli-stream-val",
            "--stream-metadata-delete",
            "0:OLD_STREAM_KEY",
            "--progress",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan.status.success(),
        "CLI plan loudnorm failed: {}",
        String::from_utf8_lossy(&cli_plan.stderr)
    );
    let cli_plan_json: serde_json::Value = serde_json::from_slice(&cli_plan.stdout).unwrap();
    assert_eq!(cli_plan_json["command"], "loudnorm");
    assert!(
        cli_plan_json["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["action"] == "metadata")
    );

    let cli_mp3_mka = dir.0.join("cli-mp3-dual.mka");
    let cli_run = Command::new(bin_path())
        .args([
            "media",
            "loudnorm",
            mp3.to_str().unwrap(),
            cli_mp3_mka.to_str().unwrap(),
            "--dual-pass",
            "--metadata",
            "title=CLI Loudnorm MP3 MKA",
            "--metadata",
            "CUSTOM_MKA_KEY=cli-mka-val",
            "--metadata-delete",
            "comment",
            "--stream-metadata",
            "0:title=CLI MP3 Stream 0",
            "--stream-metadata",
            "0:language=fra",
            "--stream-metadata",
            "0:CUSTOM_STREAM_KEY=cli-stream-val",
            "--stream-metadata-delete",
            "0:OLD_STREAM_KEY",
            "--progress",
        ])
        .output()
        .unwrap();
    assert!(
        cli_run.status.success(),
        "CLI loudnorm MP3 -> .mka failed: {}",
        String::from_utf8_lossy(&cli_run.stderr)
    );
    let stderr_text = String::from_utf8_lossy(&cli_run.stderr);
    assert!(
        stderr_text.contains("\"done\":true"),
        "expected progress hook completion event on stderr, got: {stderr_text}"
    );
    let cli_mp3_probed = fvid::media::probe(&cli_mp3_mka).unwrap();
    assert_eq!(
        cli_mp3_probed.metadata.get("title").map(String::as_str),
        Some("CLI Loudnorm MP3 MKA")
    );
    assert_eq!(
        cli_mp3_probed
            .metadata
            .get("CUSTOM_MKA_KEY")
            .map(String::as_str),
        Some("cli-mka-val")
    );
    assert_eq!(
        cli_mp3_probed.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI MP3 Stream 0")
    );
    assert_eq!(
        cli_mp3_probed.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );
    assert_eq!(
        cli_mp3_probed.streams[0]
            .metadata
            .get("CUSTOM_STREAM_KEY")
            .map(String::as_str),
        Some("cli-stream-val")
    );

    // 4. Refusal checks:
    // - `.wav` output with `--stream-metadata` must fail cleanly without creating output
    let bad_wav = dir.0.join("bad-stream.wav");
    let bad_wav_cli = Command::new(bin_path())
        .args([
            "media",
            "loudnorm",
            wav.to_str().unwrap(),
            bad_wav.to_str().unwrap(),
            "--stream-metadata",
            "0:title=NotAllowedOnWav",
        ])
        .output()
        .unwrap();
    assert!(
        !bad_wav_cli.status.success(),
        "loudnorm to .wav must reject --stream-metadata"
    );
    assert!(!bad_wav.exists());

    // - out-of-range stream metadata index (e.g. `1:title=Bad`) must fail on both `plan loudnorm` and `loudnorm` to `.mka`
    let mut bad_stream_opts = fvid::media::CopyOptions::default();
    bad_stream_opts.stream_metadata_set = vec![(1, "title".into(), "OutOfRange".into())];
    assert!(
        fvid::media::plan_loudnorm(&wav, None, false, &bad_stream_opts).is_err(),
        "plan_loudnorm must reject stream metadata index != 0"
    );
    let bad_mka = dir.0.join("bad-index.mka");
    assert!(
        fvid::media::apply_loudnorm(&wav, &bad_mka, None, &bad_stream_opts).is_err(),
        "apply_loudnorm to .mka must reject stream metadata index != 0"
    );
    assert!(!bad_mka.exists());
}
