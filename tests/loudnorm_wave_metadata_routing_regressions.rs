use fvid::media::CopyOptions;
use fvid::media_info::NormalizeTarget;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-loudnorm-wave-metadata-routing-{tag}-{}",
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
        .join("tests/fixtures/playback-errors/loudnorm-wave-metadata-routing")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn loudnorm_and_normalize_loudness_preserve_and_edit_wave_info_metadata_in_api_and_cli() {
    let dir = TempDir::new("meta-routing");
    let wav = fixture("tagged-48k-450ms.wav");
    let ogg = fixture("tagged-opus-48k.ogg");
    let mka = fixture("multitrack-pcm-flac.mka");

    let meta_opts = CopyOptions {
        metadata_delete: vec!["artist".into()],
        metadata_set: vec![("title".into(), "Normalized Title".into())],
        max_rss_bytes: Some(4096 * 1024 * 1024),
        ..Default::default()
    };

    // 1. Public `fvid::media::{plan_loudnorm, apply_loudnorm, apply_loudnorm_dual}` on tagged WAVE preserves unedited INFO tags and applies metadata mutations.
    let wav_ln_plan = fvid::media::plan_loudnorm(&wav, None, false, &meta_opts)
        .expect("media::plan_loudnorm WAVE with metadata edits");
    assert_eq!(wav_ln_plan.command, "loudnorm");
    assert!(wav_ln_plan.steps.iter().any(|s| s.action == "metadata"));

    let wav_ln_out = dir.0.join("wav-loudnorm.wav");
    let wav_ln_stats = fvid::media::apply_loudnorm(&wav, &wav_ln_out, None, &meta_opts)
        .expect("media::apply_loudnorm WAVE with metadata edits");
    assert_eq!(wav_ln_stats.sample_rate, 192_000);
    let wav_ln_probed = fvid::media::probe(&wav_ln_out).unwrap();
    assert_eq!(
        wav_ln_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );
    assert!(!wav_ln_probed.metadata.contains_key("artist"));
    assert_eq!(
        wav_ln_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve WAV Comment")
    );

    let wav_dual_out = dir.0.join("wav-loudnorm-dual.wav");
    let wav_dual_stats = fvid::media::apply_loudnorm_dual(&wav, &wav_dual_out, None, &meta_opts)
        .expect("media::apply_loudnorm_dual WAVE with metadata edits");
    assert!(wav_dual_stats.dual_pass);
    let wav_dual_probed = fvid::media::probe(&wav_dual_out).unwrap();
    assert_eq!(
        wav_dual_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );
    assert!(!wav_dual_probed.metadata.contains_key("artist"));
    assert_eq!(
        wav_dual_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve WAV Comment")
    );

    // 2. Public `fvid::media::{plan_loudnorm, apply_loudnorm}` on tagged Ogg Opus preserves OpusTags in WAVE INFO and applies metadata mutations.
    let ogg_ln_plan = fvid::media::plan_loudnorm(&ogg, None, false, &meta_opts)
        .expect("media::plan_loudnorm Ogg Opus with metadata edits");
    assert_eq!(ogg_ln_plan.command, "loudnorm");
    assert!(ogg_ln_plan.steps.iter().any(|s| s.action == "metadata"));

    let ogg_ln_out = dir.0.join("ogg-loudnorm.wav");
    let ogg_ln_stats = fvid::media::apply_loudnorm(&ogg, &ogg_ln_out, None, &meta_opts)
        .expect("media::apply_loudnorm Ogg Opus with metadata edits");
    assert_eq!(ogg_ln_stats.sample_frames, 1920 * 4);
    let ogg_ln_probed = fvid::media::probe(&ogg_ln_out).unwrap();
    assert_eq!(
        ogg_ln_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );
    assert!(!ogg_ln_probed.metadata.contains_key("artist"));
    assert_eq!(
        ogg_ln_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve Ogg Comment")
    );

    // 3. Public `fvid::media::{plan_loudnorm, apply_loudnorm}` on multitrack Matroska with selected stream and metadata mutations.
    let mut mka_opts = meta_opts.clone();
    mka_opts.streams = vec![1];
    let mka_ln_plan = fvid::media::plan_loudnorm(&mka, None, false, &mka_opts)
        .expect("media::plan_loudnorm multitrack MKA stream 1 with metadata edits");
    assert_eq!(mka_ln_plan.streams[0].index, 1);

    let mka_ln_out = dir.0.join("mka-loudnorm.wav");
    let mka_ln_stats = fvid::media::apply_loudnorm(&mka, &mka_ln_out, None, &mka_opts)
        .expect("media::apply_loudnorm multitrack MKA stream 1 with metadata edits");
    assert_eq!(mka_ln_stats.sample_frames, 11 * 1920 * 4);
    let mka_ln_probed = fvid::media::probe(&mka_ln_out).unwrap();
    assert_eq!(
        mka_ln_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );

    // 4. CLI `plan loudnorm` and `loudnorm` with `--metadata` and `--metadata-delete`.
    let cli_plan_ln = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "loudnorm",
            wav.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Loudnorm Title",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_ln.status.success(),
        "cli plan loudnorm failed: {}",
        String::from_utf8_lossy(&cli_plan_ln.stderr)
    );
    let cli_plan_ln_json: serde_json::Value =
        serde_json::from_slice(&cli_plan_ln.stdout).unwrap();
    assert_eq!(cli_plan_ln_json["command"], "loudnorm");

    let cli_ln_out = dir.0.join("cli-loudnorm.wav");
    let cli_ln = Command::new(bin_path())
        .args([
            "media",
            "loudnorm",
            ogg.to_str().unwrap(),
            cli_ln_out.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Loudnorm Ogg",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        cli_ln.status.success(),
        "cli loudnorm failed: {}",
        String::from_utf8_lossy(&cli_ln.stderr)
    );
    let cli_ln_probed = fvid::media::probe(&cli_ln_out).unwrap();
    assert_eq!(
        cli_ln_probed.metadata.get("title").map(String::as_str),
        Some("CLI Loudnorm Ogg")
    );
    assert!(!cli_ln_probed.metadata.contains_key("artist"));
    assert_eq!(
        cli_ln_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve Ogg Comment")
    );

    // 5. Public `fvid::media::{plan_normalize_loudness, normalize_loudness}` and CLI `normalize-loudness` with WAVE INFO metadata preservation and mutations.
    let target = NormalizeTarget::default();
    let norm_plan = fvid::media::plan_normalize_loudness(&wav, &meta_opts, None, target)
        .expect("media::plan_normalize_loudness with metadata edits");
    assert_eq!(norm_plan.command, "normalize-loudness");
    assert!(norm_plan.steps.iter().any(|s| s.action == "metadata"));

    let norm_wav_out = dir.0.join("norm-wave.wav");
    let norm_report = fvid::media::normalize_loudness(&wav, &norm_wav_out, &meta_opts, None, target)
        .expect("media::normalize_loudness to .wav with metadata edits");
    assert_eq!(norm_report.sample_frames, 21_600);
    let norm_wav_probed = fvid::media::probe(&norm_wav_out).unwrap();
    assert_eq!(
        norm_wav_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );
    assert!(!norm_wav_probed.metadata.contains_key("artist"));
    assert_eq!(
        norm_wav_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve WAV Comment")
    );

    // Matroska output (.mka) with metadata mutations in normalize-loudness now succeeds and writes container tags.
    let norm_mka_out = dir.0.join("norm-wave.mka");
    fvid::media::normalize_loudness(&wav, &norm_mka_out, &meta_opts, None, target)
        .expect("media::normalize_loudness to .mka with metadata edits");
    let norm_mka_probed = fvid::media::probe(&norm_mka_out).unwrap();
    assert_eq!(
        norm_mka_probed.metadata.get("title").map(String::as_str),
        Some("Normalized Title")
    );

    let cli_plan_norm = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            wav.to_str().unwrap(),
            norm_wav_out.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Norm Title",
        ])
        .output()
        .unwrap();
    assert!(
        cli_plan_norm.status.success(),
        "cli plan normalize-loudness failed: {}",
        String::from_utf8_lossy(&cli_plan_norm.stderr)
    );

    let cli_norm_out = dir.0.join("cli-norm-wave.wav");
    let cli_norm = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            wav.to_str().unwrap(),
            cli_norm_out.to_str().unwrap(),
            "--metadata-delete",
            "artist",
            "--metadata",
            "title=CLI Norm Title",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        cli_norm.status.success(),
        "cli normalize-loudness failed: {}",
        String::from_utf8_lossy(&cli_norm.stderr)
    );
    let cli_norm_probed = fvid::media::probe(&cli_norm_out).unwrap();
    assert_eq!(
        cli_norm_probed.metadata.get("title").map(String::as_str),
        Some("CLI Norm Title")
    );
    assert!(!cli_norm_probed.metadata.contains_key("artist"));
    assert_eq!(
        cli_norm_probed.metadata.get("comment").map(String::as_str),
        Some("Preserve WAV Comment")
    );
}
