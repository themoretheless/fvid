use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-budget-controls")
        .join(name)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-audio-budget-controls-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_media_cli(args: &[&str]) -> Result<String, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_fvid"))
        .arg("media")
        .args(args)
        .output()
        .expect("run fvid media");
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn decode_audio_and_plan_enforce_max_memory_mib_and_controlled_bytes_across_all_owned_formats() {
    let dir = temp_dir("decode");
    let cases = [
        ("pcm-s16le-48k-450ms.wav", "pcm-wave", 21_600u64),
        ("aac-lc-48k.adts", "adts-aac", 3_072u64),
        ("mp4-pcm-48k-450ms.m4a", "mp4-pcm", 21_600u64),
        ("matroska-flac-48k-440ms.mka", "mka-flac", 21_120u64),
        ("g711-alaw-48k-450ms.wav", "g711-alaw", 21_600u64),
        ("raw-flac-48k-440ms.flac", "raw-flac", 21_120u64),
    ];

    let small_opts = fvid::media::CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let generous_opts = fvid::media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };

    for (file_name, tag, expected_frames) in cases {
        let source = fixture(file_name);

        // Plan must accept a generous max_controlled_bytes budget
        let plan = fvid::media::plan_decode_audio(
            &source,
            &fvid::media::AudioDecodeTransform::default(),
            &generous_opts,
        )
        .unwrap_or_else(|e| panic!("{tag} plan_decode_audio with generous budget failed: {e}"));
        assert_eq!(plan.command, "decode-audio");

        // CLI plan decode-audio with --max-memory-mib
        run_media_cli(&[
            "plan",
            "decode-audio",
            source.to_str().unwrap(),
            "--max-memory-mib",
            "64",
        ])
        .unwrap_or_else(|e| panic!("{tag} CLI plan decode-audio --max-memory-mib failed: {e}"));

        for ext in ["wav", "f32le", "mka"] {
            let small_out = dir.join(format!("{tag}-small.{ext}"));
            let err = fvid::media::decode_audio_transformed(
                &source,
                &small_out,
                fvid::media::AudioDecodeTransform::default(),
                &small_opts,
            )
            .expect_err("1-byte controlled memory budget must be refused");
            assert!(
                err.contains("controlled memory budget exceeded"),
                "unexpected error for {tag} ({ext}): {err}"
            );
            assert!(!small_out.exists());

            let ok_out = dir.join(format!("{tag}-ok.{ext}"));
            let stats = fvid::media::decode_audio_transformed(
                &source,
                &ok_out,
                fvid::media::AudioDecodeTransform::default(),
                &generous_opts,
            )
            .unwrap_or_else(|e| panic!("{tag} decode_audio_transformed ({ext}) failed: {e}"));
            assert_eq!(stats.sample_frames, expected_frames);
            assert!(ok_out.is_file());

            let cli_out = dir.join(format!("{tag}-cli.{ext}"));
            run_media_cli(&[
                "decode-audio",
                source.to_str().unwrap(),
                cli_out.to_str().unwrap(),
                "--max-memory-mib",
                "64",
                "--quiet",
            ])
            .unwrap_or_else(|e| {
                panic!("{tag} CLI decode-audio --max-memory-mib ({ext}) failed: {e}")
            });
            assert!(cli_out.is_file());
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trim_and_trim_pcm_enforce_budgets_in_api_and_cli() {
    let dir = temp_dir("trim");

    let wave_source = fixture("pcm-s16le-48k-450ms.wav");
    let mp4_source = fixture("mp4-pcm-48k-450ms.m4a");
    let flac_source = fixture("raw-flac-48k-440ms.flac");

    let small_mem = fvid::media::CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let zero_pkt = fvid::media::CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    let small_pkt = fvid::media::CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    let small_rss = fvid::media::CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    let generous = fvid::media::CopyOptions {
        max_packet_bytes: 64 * 1024,
        max_packets: Some(100),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        max_rss_bytes: Some(8 * 1024 * 1024 * 1024),
        ..Default::default()
    };

    // 1. WAVE trim_pcm and plan_trim_pcm
    assert!(fvid::media::plan_trim_pcm(&wave_source, 0, 100_000, &small_mem).is_err());
    assert!(fvid::media::plan_trim_pcm(&wave_source, 0, 100_000, &small_pkt).is_err());
    assert!(fvid::media::plan_trim_pcm(&wave_source, 0, 100_000, &small_rss).is_err());
    let plan_pcm = fvid::media::plan_trim_pcm(&wave_source, 0, 100_000, &generous)
        .expect("plan_trim_pcm with generous budgets");
    assert_eq!(plan_pcm.command, "trim-pcm");

    let wave_trim_pcm_out = dir.join("wave-trim-pcm.wav");
    assert!(
        fvid::media::trim_pcm(&wave_source, &wave_trim_pcm_out, 0, 100_000, &small_mem).is_err()
    );
    assert!(!wave_trim_pcm_out.exists());
    let pcm_stats = fvid::media::trim_pcm(&wave_source, &wave_trim_pcm_out, 0, 100_000, &generous)
        .expect("trim_pcm with generous budgets");
    assert_eq!(pcm_stats.sample_frames, 4_800);

    // CLI trim-pcm and plan trim-pcm with budget flags
    let cli_trim_pcm = dir.join("cli-trim-pcm.wav");
    run_media_cli(&[
        "plan",
        "trim-pcm",
        wave_source.to_str().unwrap(),
        "--from",
        "0",
        "--to",
        "0.1",
        "--max-packets",
        "100",
        "--max-packet-bytes",
        "65536",
        "--max-memory-mib",
        "64",
        "--max-rss-mib",
        "8192",
        "--quiet",
    ])
    .expect("CLI plan trim-pcm with budgets");

    run_media_cli(&[
        "trim-pcm",
        wave_source.to_str().unwrap(),
        cli_trim_pcm.to_str().unwrap(),
        "--from",
        "0",
        "--to",
        "0.1",
        "--max-packets",
        "100",
        "--max-packet-bytes",
        "65536",
        "--max-memory-mib",
        "64",
        "--max-rss-mib",
        "8192",
        "--quiet",
    ])
    .expect("CLI trim-pcm with budgets");
    assert!(cli_trim_pcm.is_file());

    // 2. trim and plan_trim across WAVE, MP4 PCM, and bare FLAC
    for (src, tag) in [
        (&wave_source, "wave"),
        (&mp4_source, "mp4-pcm"),
        (&flac_source, "raw-flac"),
    ] {
        assert!(fvid::media::plan_trim(src, 0, 100_000, &zero_pkt).is_err());
        assert!(fvid::media::plan_trim(src, 0, 100_000, &small_rss).is_err());
        let plan = fvid::media::plan_trim(src, 0, 100_000, &generous)
            .unwrap_or_else(|e| panic!("{tag} plan_trim with generous budgets failed: {e}"));
        assert_eq!(plan.command, "trim");

        for ext in ["wav", "f32le", "mka"] {
            if tag == "wave" && ext != "wav" {
                continue;
            }
            let bad_out = dir.join(format!("{tag}-bad.{ext}"));
            assert!(
                fvid::media::trim(src, &bad_out, 0, 100_000, &small_mem).is_err(),
                "{tag} trim ({ext}) must refuse 1-byte controlled memory budget"
            );
            assert!(!bad_out.exists());

            let bad_pkt_out = dir.join(format!("{tag}-bad-pkt.{ext}"));
            assert!(
                fvid::media::trim(src, &bad_pkt_out, 0, 100_000, &small_pkt).is_err(),
                "{tag} trim ({ext}) must refuse 1-byte packet budget"
            );
            assert!(!bad_pkt_out.exists());

            let ok_out = dir.join(format!("{tag}-ok.{ext}"));
            fvid::media::trim(src, &ok_out, 0, 100_000, &generous)
                .unwrap_or_else(|e| panic!("{tag} trim ({ext}) with generous budgets failed: {e}"));
            assert!(ok_out.is_file());

            let cli_out = dir.join(format!("{tag}-cli.{ext}"));
            run_media_cli(&[
                "trim",
                src.to_str().unwrap(),
                cli_out.to_str().unwrap(),
                "--from",
                "0",
                "--to",
                "0.1",
                "--max-packets",
                "100",
                "--max-packet-bytes",
                "65536",
                "--max-memory-mib",
                "64",
                "--max-rss-mib",
                "8192",
                "--quiet",
            ])
            .unwrap_or_else(|e| panic!("{tag} CLI trim ({ext}) with budgets failed: {e}"));
            assert!(cli_out.is_file());
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loudness_and_normalize_loudness_enforce_budgets_in_api_and_cli() {
    let dir = temp_dir("loudness");

    let cases = [
        ("pcm-s16le-48k-450ms.wav", "pcm-wave", 21_600u64),
        ("mp4-pcm-48k-450ms.m4a", "mp4-pcm", 21_600u64),
        ("matroska-flac-48k-440ms.mka", "mka-flac", 21_120u64),
        ("g711-alaw-48k-450ms.wav", "g711-alaw", 21_600u64),
        ("raw-flac-48k-440ms.flac", "raw-flac", 21_120u64),
    ];

    let small_mem = fvid::media::CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let small_pkt = fvid::media::CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    let small_rss = fvid::media::CopyOptions {
        max_rss_bytes: Some(1),
        ..Default::default()
    };
    let generous = fvid::media::CopyOptions {
        max_packet_bytes: 64 * 1024,
        max_packets: Some(100),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        max_rss_bytes: Some(8 * 1024 * 1024 * 1024),
        ..Default::default()
    };
    let target = fvid::media_info::NormalizeTarget::default();

    for (file_name, tag, expected_frames) in cases {
        let source = fixture(file_name);

        // Plans must enforce packet/RSS/controlled memory budgets
        assert!(fvid::media::plan_loudness(&source, &small_rss).is_err());
        assert!(fvid::media::plan_normalize_loudness(&source, &small_rss, None, target).is_err());

        let l_plan = fvid::media::plan_loudness(&source, &generous)
            .unwrap_or_else(|e| panic!("{tag} plan_loudness with generous budgets failed: {e}"));
        assert_eq!(l_plan.command, "loudness");

        let n_plan = fvid::media::plan_normalize_loudness(&source, &generous, None, target)
            .unwrap_or_else(|e| {
                panic!("{tag} plan_normalize_loudness with generous budgets failed: {e}")
            });
        assert_eq!(n_plan.command, "normalize-loudness");

        // Execution must refuse 1-byte packet/memory/RSS budgets
        assert!(
            fvid::media::measure_loudness(&source, &small_mem).is_err(),
            "{tag} measure_loudness must refuse 1-byte controlled memory budget"
        );
        assert!(
            fvid::media::measure_loudness(&source, &small_pkt).is_err(),
            "{tag} measure_loudness must refuse 1-byte packet limit"
        );
        assert!(
            fvid::media::measure_loudness(&source, &small_rss).is_err(),
            "{tag} measure_loudness must refuse 1-byte RSS limit"
        );

        let measured = fvid::media::measure_loudness(&source, &generous)
            .unwrap_or_else(|e| panic!("{tag} measure_loudness with generous budgets failed: {e}"));
        assert_eq!(measured.sample_frames, expected_frames);
        assert!(measured.integrated_lufs.is_some());

        let bad_norm = dir.join(format!("{tag}-bad-norm.wav"));
        assert!(
            fvid::media::normalize_loudness(&source, &bad_norm, &small_mem, None, target).is_err(),
            "{tag} normalize_loudness must refuse 1-byte controlled memory budget"
        );
        assert!(!bad_norm.exists());

        for ext in ["wav", "mka"] {
            let ok_norm = dir.join(format!("{tag}-ok-norm.{ext}"));
            let report =
                fvid::media::normalize_loudness(&source, &ok_norm, &generous, None, target)
                    .unwrap_or_else(|e| panic!("{tag} normalize_loudness ({ext}) failed: {e}"));
            assert_eq!(report.sample_frames, expected_frames);
            assert!(ok_norm.is_file());
        }

        // CLI loudness, plan loudness, normalize-loudness, plan normalize-loudness with budgets
        let plan_cli_wav = dir.join(format!("{tag}-plan-cli.wav"));
        let cli_norm_mka = dir.join(format!("{tag}-cli-norm.mka"));
        for cmd in [
            vec![
                "plan",
                "loudness",
                source.to_str().unwrap(),
                "--max-packets",
                "100",
                "--max-packet-bytes",
                "65536",
                "--max-memory-mib",
                "64",
                "--max-rss-mib",
                "8192",
            ],
            vec![
                "loudness",
                source.to_str().unwrap(),
                "--max-packets",
                "100",
                "--max-packet-bytes",
                "65536",
                "--max-memory-mib",
                "64",
                "--max-rss-mib",
                "8192",
                "--quiet",
            ],
            vec![
                "plan",
                "normalize-loudness",
                source.to_str().unwrap(),
                plan_cli_wav.to_str().unwrap(),
                "--max-packets",
                "100",
                "--max-packet-bytes",
                "65536",
                "--max-memory-mib",
                "64",
                "--max-rss-mib",
                "8192",
                "--quiet",
            ],
            vec![
                "normalize-loudness",
                source.to_str().unwrap(),
                cli_norm_mka.to_str().unwrap(),
                "--max-packets",
                "100",
                "--max-packet-bytes",
                "65536",
                "--max-memory-mib",
                "64",
                "--max-rss-mib",
                "8192",
                "--quiet",
            ],
        ] {
            run_media_cli(&cmd)
                .unwrap_or_else(|e| panic!("{tag} CLI loudness/normalize with budgets failed: {e}"));
        }
        assert!(cli_norm_mka.is_file());
    }

    let _ = std::fs::remove_dir_all(&dir);
}
