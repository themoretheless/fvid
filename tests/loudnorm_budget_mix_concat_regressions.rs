use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, CopyOptions, MixAudioOptions};

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-loudnorm-budget-mix-concat-{name}-{}",
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
        .join("tests/fixtures/playback-errors/loudnorm-budget-mix-concat")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn native_loudnorm_and_cli_honor_max_controlled_bytes_across_adts_mp4_and_matroska() {
    let dir = fixture_dir();
    let adts = dir.join("adts-mono-48k.aac");
    let mp4 = dir.join("mp4-pcm-48k.m4a");
    let mka = dir.join("matroska-flac-48k.mka");
    let tmp = Directory::new("loudnorm-budget");

    let generous = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..CopyOptions::default()
    };
    let tiny = CopyOptions {
        max_controlled_bytes: Some(1024),
        ..CopyOptions::default()
    };

    for (name, path) in [("adts", &adts), ("mp4", &mp4), ("mka", &mka)] {
        let plan = fvid::native_loudnorm::try_plan(path, None, false, &generous)
            .unwrap_or_else(|e| panic!("try_plan error for {name}: {e}"))
            .unwrap_or_else(|| panic!("try_plan returned None for {name} with max_controlled_bytes"));
        assert_eq!(plan.command, "loudnorm");

        let plan_err = fvid::native_loudnorm::try_plan(path, None, false, &tiny)
            .expect_err("try_plan should refuse tiny memory budget");
        assert!(
            plan_err
                .to_string()
                .contains("controlled memory budget exceeded"),
            "unexpected plan_err for {name}: {plan_err}"
        );

        let out_wav = tmp.0.join(format!("{name}-loudnorm.wav"));
        let apply_err = fvid::native_loudnorm::try_apply(path, &out_wav, None, false, &tiny)
            .expect_err("try_apply should refuse tiny memory budget");
        assert!(
            apply_err
                .to_string()
                .contains("controlled memory budget exceeded"),
            "unexpected apply_err for {name}: {apply_err}"
        );
        assert!(!out_wav.exists());

        let stats = fvid::native_loudnorm::try_apply(path, &out_wav, None, false, &generous)
            .unwrap_or_else(|e| panic!("try_apply error for {name}: {e}"))
            .unwrap_or_else(|| panic!("try_apply returned None for {name} with max_controlled_bytes"));
        assert!(stats.sample_frames > 0);
        assert!(out_wav.is_file());

        // CLI plan & execution with --max-memory-mib without legacy feature
        let cli_plan = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "loudnorm",
                path.to_str().unwrap(),
                "--max-memory-mib",
                "64",
            ])
            .output()
            .expect("cli plan loudnorm");
        assert!(
            cli_plan.status.success(),
            "CLI plan loudnorm failed for {name}: {}",
            String::from_utf8_lossy(&cli_plan.stderr)
        );

        let cli_wav = tmp.0.join(format!("{name}-cli-loudnorm.wav"));
        let cli_run = Command::new(bin_path())
            .args([
                "media",
                "loudnorm",
                path.to_str().unwrap(),
                cli_wav.to_str().unwrap(),
                "--max-memory-mib",
                "64",
                "--quiet",
            ])
            .output()
            .expect("cli loudnorm");
        assert!(
            cli_run.status.success(),
            "CLI loudnorm failed for {name}: {}",
            String::from_utf8_lossy(&cli_run.stderr)
        );
        assert!(cli_wav.is_file());
    }
}

#[test]
fn mix_and_merge_audio_support_matroska_and_wave_outputs_across_owned_formats() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k.wav");
    let mulaw = dir.join("g711-mulaw-48k.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let flac = dir.join("raw-flac-48k.flac");
    let mka_flac = dir.join("matroska-flac-48k.mka");
    let tmp = Directory::new("mix-merge");

    for (label, sources, expected_frames) in [
        ("g711-pair", vec![alaw.clone(), mulaw.clone()], 1920u64),
        ("ogg-flac", vec![ogg.clone(), flac.clone()], 1920u64),
        ("flac-mka", vec![flac.clone(), mka_flac.clone()], 3840u64),
    ] {
        assert!(
            fvid::native_audio_mix::eligible(&sources).unwrap(),
            "native_audio_mix::eligible should accept {label}"
        );
        let plan_mix = media::plan_mix_audio(&sources, &MixAudioOptions::default())
            .unwrap_or_else(|e| panic!("plan_mix_audio failed for {label}: {e}"));
        assert_eq!(plan_mix.command, "mix-audio");

        let out_mix_mka = tmp.0.join(format!("{label}-mix.mka"));
        let stats_mix = media::mix_audio(&sources, &out_mix_mka, &MixAudioOptions::default())
            .unwrap_or_else(|e| panic!("mix_audio to .mka failed for {label}: {e}"));
        assert_eq!(stats_mix.sample_frames, expected_frames);
        assert_eq!(stats_mix.channels, 1);
        assert!(out_mix_mka.is_file());

        let plan_merge = media::plan_merge_audio(&sources)
            .unwrap_or_else(|e| panic!("plan_merge_audio failed for {label}: {e}"));
        assert_eq!(plan_merge.command, "merge-audio");

        let out_merge_mka = tmp.0.join(format!("{label}-merge.mka"));
        let stats_merge = media::merge_audio(&sources, &out_merge_mka)
            .unwrap_or_else(|e| panic!("merge_audio to .mka failed for {label}: {e}"));
        assert_eq!(stats_merge.sample_frames, expected_frames);
        assert_eq!(stats_merge.channels, 2);
        assert!(out_merge_mka.is_file());

        // Also verify CLI execution to .mka
        let cli_mix_mka = tmp.0.join(format!("{label}-cli-mix.mka"));
        let cli_mix = Command::new(bin_path())
            .args([
                "media",
                "mix-audio",
                cli_mix_mka.to_str().unwrap(),
                sources[0].to_str().unwrap(),
                sources[1].to_str().unwrap(),
                "--quiet",
            ])
            .output()
            .expect("cli mix-audio");
        assert!(
            cli_mix.status.success(),
            "CLI mix-audio failed for {label}: {}",
            String::from_utf8_lossy(&cli_mix.stderr)
        );
        assert!(cli_mix_mka.is_file());

        let cli_merge_mka = tmp.0.join(format!("{label}-cli-merge.mka"));
        let cli_merge = Command::new(bin_path())
            .args([
                "media",
                "merge-audio",
                cli_merge_mka.to_str().unwrap(),
                sources[0].to_str().unwrap(),
                sources[1].to_str().unwrap(),
                "--quiet",
            ])
            .output()
            .expect("cli merge-audio");
        assert!(
            cli_merge.status.success(),
            "CLI merge-audio failed for {label}: {}",
            String::from_utf8_lossy(&cli_merge.stderr)
        );
        assert!(cli_merge_mka.is_file());
    }
}

#[test]
fn sample_domain_audio_concat_supports_owned_audio_formats_to_matroska() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k.wav");
    let mulaw = dir.join("g711-mulaw-48k.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let flac = dir.join("raw-flac-48k.flac");
    let mka_flac = dir.join("matroska-flac-48k.mka");
    let video = dir.join("video.mkv");
    let tmp = Directory::new("concat");

    let video_stats = media::decode_video(&video).expect("decode synthetic video");
    assert_eq!(video_stats.video_frames, 1);

    for (label, sources, expected_frames) in [
        ("g711-pair", vec![alaw.clone(), mulaw.clone()], 3840u64),
        ("ogg-flac", vec![ogg.clone(), flac.clone()], 5760u64),
        ("flac-mka", vec![flac.clone(), mka_flac.clone()], 7680u64),
    ] {
        assert!(
            fvid::native_audio_mix::concat_eligible(&sources).unwrap(),
            "concat_eligible should accept {label}"
        );
        let plan = media::plan_concat(&sources, &CopyOptions::default())
            .unwrap_or_else(|e| panic!("media::plan_concat failed for {label}: {e}"));
        assert_eq!(plan.command, "concat");
        assert_eq!(plan.streams[0].disposition, "decode_to_pcm");

        let out_mka = tmp.0.join(format!("{label}-concat.mka"));
        let stats = media::concat(&sources, &out_mka, &CopyOptions::default())
            .unwrap_or_else(|e| panic!("media::concat failed for {label}: {e}"));
        assert_eq!(stats.payload_bytes, expected_frames * 4);
        assert_eq!(stats.segments, 2);
        assert!(out_mka.is_file());

        // Verify CLI plan concat and concat to .mka
        let cli_plan = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "concat",
                sources[0].to_str().unwrap(),
                sources[1].to_str().unwrap(),
                "--output-format",
                "mka",
            ])
            .output()
            .expect("cli plan concat");
        assert!(
            cli_plan.status.success(),
            "CLI plan concat failed for {label}: {}",
            String::from_utf8_lossy(&cli_plan.stderr)
        );

        let cli_mka = tmp.0.join(format!("{label}-cli-concat.mka"));
        let cli_concat = Command::new(bin_path())
            .args([
                "media",
                "concat",
                cli_mka.to_str().unwrap(),
                sources[0].to_str().unwrap(),
                sources[1].to_str().unwrap(),
                "--quiet",
            ])
            .output()
            .expect("cli concat");
        assert!(
            cli_concat.status.success(),
            "CLI concat failed for {label}: {}",
            String::from_utf8_lossy(&cli_concat.stderr)
        );
        assert!(cli_mka.is_file());
    }
}
