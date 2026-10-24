use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, AudioDecodeTransform, CopyOptions};
use fvid::media_info::NormalizeTarget;

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-owned-audio-f32le-matroska-{name}-{}",
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
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/owned-audio-f32le-matroska")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn decode_audio_and_cli_export_f32le_mka_and_mkv_for_owned_audio_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let flac = dir.join("raw-flac-48k-440ms.flac");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("decode");

    let default_options = CopyOptions::default();

    for (name, path, expected_frames) in [
        ("alaw", &alaw, 21_600u64),
        ("ogg", &ogg, 1_920u64),
        ("flac", &flac, 21_120u64),
    ] {
        for ext in ["f32le", "mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-api.{ext}"));
            let stats = media::decode_audio(path, &out_api, &default_options)
                .unwrap_or_else(|e| panic!("decode_audio .{ext} failed for {name}: {e}"));
            assert_eq!(stats.sample_frames, expected_frames);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "decode-audio",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                ])
                .output()
                .expect("run cli decode-audio");
            assert!(
                cli.status.success(),
                "cli decode-audio .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }

    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    for (name, path) in [("mka1", &mka), ("mov1", &mov)] {
        for ext in ["f32le", "mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-api.{ext}"));
            let stats = media::decode_audio_transformed(
                path,
                &out_api,
                AudioDecodeTransform::default(),
                &selected,
            )
            .unwrap_or_else(|e| panic!("decode_audio_transformed .{ext} failed for {name}: {e}"));
            assert_eq!(stats.sample_frames, 21_120);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "decode-audio",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                    "--streams",
                    "1",
                ])
                .output()
                .expect("run cli decode-audio --streams 1");
            assert!(
                cli.status.success(),
                "cli decode-audio .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }
}

#[test]
fn trim_and_cli_trim_export_f32le_mka_and_mkv_for_owned_audio_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let flac = dir.join("raw-flac-48k-440ms.flac");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("trim");

    let default_options = CopyOptions::default();

    for (name, path) in [("alaw", &alaw), ("ogg", &ogg), ("flac", &flac)] {
        for ext in ["f32le", "mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-trim-api.{ext}"));
            let stats = media::trim(path, &out_api, 0, 20_000, &default_options)
                .unwrap_or_else(|e| panic!("media::trim .{ext} failed for {name}: {e}"));
            assert_eq!(stats.payload_bytes, 960 * 4);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-trim-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "trim",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                    "--from",
                    "0",
                    "--to",
                    "0.02",
                ])
                .output()
                .expect("run cli trim");
            assert!(
                cli.status.success(),
                "cli trim .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }

    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    for (name, path) in [("mka1", &mka), ("mov1", &mov)] {
        for ext in ["f32le", "mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-trim-api.{ext}"));
            let stats = media::trim(path, &out_api, 0, 20_000, &selected)
                .unwrap_or_else(|e| panic!("media::trim .{ext} failed for {name}: {e}"));
            assert_eq!(stats.payload_bytes, 960 * 4);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-trim-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "trim",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                    "--from",
                    "0",
                    "--to",
                    "0.02",
                    "--streams",
                    "1",
                ])
                .output()
                .expect("run cli trim --streams 1");
            assert!(
                cli.status.success(),
                "cli trim .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }
}

#[test]
fn normalize_loudness_and_cli_export_mka_and_mkv_for_owned_audio_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k-450ms.wav");
    let mulaw = dir.join("g711-mulaw-48k-450ms.wav");
    let flac = dir.join("raw-flac-48k-440ms.flac");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("normalize");

    let target = NormalizeTarget::default();
    let default_options = CopyOptions::default();

    for (name, path, expected_frames) in [
        ("alaw", &alaw, 21_600u64),
        ("mulaw", &mulaw, 21_600u64),
        ("flac", &flac, 21_120u64),
    ] {
        for ext in ["mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-norm-api.{ext}"));
            let report = media::normalize_loudness(path, &out_api, &default_options, None, target)
                .unwrap_or_else(|e| panic!("normalize_loudness .{ext} failed for {name}: {e}"));
            assert_eq!(report.sample_frames, expected_frames);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-norm-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "normalize-loudness",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                ])
                .output()
                .expect("run cli normalize-loudness");
            assert!(
                cli.status.success(),
                "cli normalize-loudness .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }

    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    for (name, path) in [("mka1", &mka), ("mov1", &mov)] {
        for ext in ["mka", "mkv"] {
            let out_api = tmp.0.join(format!("{name}-norm-api.{ext}"));
            let report = media::normalize_loudness(path, &out_api, &selected, None, target)
                .unwrap_or_else(|e| panic!("normalize_loudness .{ext} failed for {name}: {e}"));
            assert_eq!(report.sample_frames, 21_120);
            assert!(out_api.is_file());

            let out_cli = tmp.0.join(format!("{name}-norm-cli.{ext}"));
            let cli = Command::new(bin_path())
                .args([
                    "media",
                    "normalize-loudness",
                    path.to_str().unwrap(),
                    out_cli.to_str().unwrap(),
                    "--streams",
                    "1",
                ])
                .output()
                .expect("run cli normalize-loudness --streams 1");
            assert!(
                cli.status.success(),
                "cli normalize-loudness .{ext} failed for {name}: {}",
                String::from_utf8_lossy(&cli.stderr)
            );
            assert!(out_cli.is_file());
        }
    }
}
