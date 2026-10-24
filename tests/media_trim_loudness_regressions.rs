use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, CopyOptions};
use fvid::media_info::NormalizeTarget;

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-media-trim-loudness-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/media-trim-loudness")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn top_level_and_fvid_media_trim_and_plan_trim_support_g711_ogg_flac_and_secondary_container_tracks() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let flac = dir.join("raw-flac-48k-440ms.flac");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("trim");

    let default_options = CopyOptions::default();

    for (name, path) in [("alaw", &alaw), ("ogg", &ogg), ("flac", &flac)] {
        let plan = media::plan_trim(path, 0, 20_000, &default_options)
            .unwrap_or_else(|e| panic!("media::plan_trim failed for {name}: {e}"));
        assert_eq!(plan.command, "trim");

        let out_wav = tmp.0.join(format!("{name}-trim.wav"));
        let stats = media::trim(path, &out_wav, 0, 20_000, &default_options)
            .unwrap_or_else(|e| panic!("media::trim failed for {name}: {e}"));
        assert!(stats.payload_bytes > 0);
        assert!(out_wav.is_file());
    }

    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    for (name, path) in [("mka-track1", &mka), ("mov-track1", &mov)] {
        let plan = media::plan_trim(path, 0, 20_000, &selected)
            .unwrap_or_else(|e| panic!("media::plan_trim failed for {name}: {e}"));
        assert_eq!(plan.command, "trim");
        assert_eq!(plan.streams[0].index, 1);

        let out_wav = tmp.0.join(format!("{name}-trim.wav"));
        let stats = media::trim(path, &out_wav, 0, 20_000, &selected)
            .unwrap_or_else(|e| panic!("media::trim failed for {name}: {e}"));
        assert!(stats.payload_bytes > 0);
        assert!(out_wav.is_file());
    }
}

#[test]
fn top_level_media_and_native_pcm_loudness_and_normalize_loudness_support_owned_audio_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let mulaw = dir.join("g711-mulaw-48k-450ms.wav");
    let flac = dir.join("raw-flac-48k-440ms.flac");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("loudness-normalize");

    let target = NormalizeTarget::default();
    let default_options = CopyOptions::default();

    for (name, path) in [("alaw", &alaw), ("mulaw", &mulaw), ("flac", &flac)] {
        let l_plan = media::plan_loudness(path, &default_options)
            .unwrap_or_else(|e| panic!("media::plan_loudness failed for {name}: {e}"));
        assert_eq!(l_plan.command, "loudness");

        let l_stats = media::measure_loudness(path, &default_options)
            .unwrap_or_else(|e| panic!("media::measure_loudness failed for {name}: {e}"));
        assert!(l_stats.sample_frames >= 21_120);
        assert!(l_stats.integrated_lufs.is_some());

        let n_plan = media::plan_normalize_loudness(path, &default_options, None, target)
            .unwrap_or_else(|e| panic!("media::plan_normalize_loudness failed for {name}: {e}"));
        assert_eq!(n_plan.command, "normalize-loudness");

        let out_wav = tmp.0.join(format!("{name}-norm.wav"));
        let n_report = media::normalize_loudness(path, &out_wav, &default_options, None, target)
            .unwrap_or_else(|e| panic!("media::normalize_loudness failed for {name}: {e}"));
        assert_eq!(n_report.sample_frames, l_stats.sample_frames);
        assert!(out_wav.is_file());
    }

    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    for (name, path) in [("mka-track1", &mka), ("mov-track1", &mov)] {
        let l_plan = media::plan_loudness(path, &selected)
            .unwrap_or_else(|e| panic!("media::plan_loudness failed for {name}: {e}"));
        assert_eq!(l_plan.command, "loudness");
        assert_eq!(l_plan.streams[0].index, 1);

        let l_stats = media::measure_loudness(path, &selected)
            .unwrap_or_else(|e| panic!("media::measure_loudness failed for {name}: {e}"));
        assert_eq!(l_stats.sample_frames, 21_120);
        assert!(l_stats.integrated_lufs.is_some());

        let n_plan = media::plan_normalize_loudness(path, &selected, None, target)
            .unwrap_or_else(|e| panic!("media::plan_normalize_loudness failed for {name}: {e}"));
        assert_eq!(n_plan.command, "normalize-loudness");
        assert_eq!(n_plan.streams[0].index, 1);

        let out_wav = tmp.0.join(format!("{name}-norm.wav"));
        let n_report = media::normalize_loudness(path, &out_wav, &selected, None, target)
            .unwrap_or_else(|e| panic!("media::normalize_loudness failed for {name}: {e}"));
        assert_eq!(n_report.sample_frames, 21_120);
        assert!(out_wav.is_file());
    }
}

#[test]
fn media_cli_routes_trim_loudness_and_normalize_loudness_across_owned_audio_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("cli");

    let plan_trim_alaw = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "trim",
            alaw.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
        ])
        .output()
        .expect("run cli plan trim alaw");
    assert!(
        plan_trim_alaw.status.success(),
        "cli plan trim alaw failed: {}",
        String::from_utf8_lossy(&plan_trim_alaw.stderr)
    );

    let trimmed_alaw = tmp.0.join("alaw-trimmed.wav");
    let trim_alaw = Command::new(bin_path())
        .args([
            "media",
            "trim",
            alaw.to_str().unwrap(),
            trimmed_alaw.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.02",
        ])
        .output()
        .expect("run cli trim alaw");
    assert!(
        trim_alaw.status.success(),
        "cli trim alaw failed: {}",
        String::from_utf8_lossy(&trim_alaw.stderr)
    );
    assert!(trimmed_alaw.is_file());

    let trimmed_mka = tmp.0.join("mka1-trimmed.wav");
    let trim_mka = Command::new(bin_path())
        .args([
            "media",
            "trim",
            mka.to_str().unwrap(),
            trimmed_mka.to_str().unwrap(),
            "--from",
            "0",
            "--to",
            "0.04",
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli trim mka stream 1");
    assert!(
        trim_mka.status.success(),
        "cli trim mka stream 1 failed: {}",
        String::from_utf8_lossy(&trim_mka.stderr)
    );
    assert!(trimmed_mka.is_file());

    let plan_loudness_mov = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "loudness",
            mov.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli plan loudness mov stream 1");
    assert!(
        plan_loudness_mov.status.success(),
        "cli plan loudness mov stream 1 failed: {}",
        String::from_utf8_lossy(&plan_loudness_mov.stderr)
    );

    let loudness_mov = Command::new(bin_path())
        .args([
            "media",
            "loudness",
            mov.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli loudness mov stream 1");
    assert!(
        loudness_mov.status.success(),
        "cli loudness mov stream 1 failed: {}",
        String::from_utf8_lossy(&loudness_mov.stderr)
    );

    let norm_mka = tmp.0.join("mka1-norm.wav");
    let plan_norm_mka = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            mka.to_str().unwrap(),
            norm_mka.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli plan normalize-loudness mka stream 1");
    assert!(
        plan_norm_mka.status.success(),
        "cli plan normalize-loudness mka stream 1 failed: {}",
        String::from_utf8_lossy(&plan_norm_mka.stderr)
    );

    let run_norm_mka = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            mka.to_str().unwrap(),
            norm_mka.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli normalize-loudness mka stream 1");
    assert!(
        run_norm_mka.status.success(),
        "cli normalize-loudness mka stream 1 failed: {}",
        String::from_utf8_lossy(&run_norm_mka.stderr)
    );
    assert!(norm_mka.is_file());
}
