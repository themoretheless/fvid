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

#[test]
fn mix_merge_concat_and_matroska_publish_enforce_controlled_memory_budgets() {
    let dir = temp_dir("mix-merge-concat-matroska-budgets");
    let wave_source = fixture("pcm-s16le-48k-450ms.wav");
    let adts_source = fixture("aac-lc-48k.adts");

    // 1. mix_audio_with_controls & plan_mix_audio_with_controls must enforce max_controlled_bytes
    // against the mixer working set + output WAVE INFO / Matroska metadata buffers.
    let mut big_metadata_set = Vec::new();
    for i in 0..32 {
        big_metadata_set.push((format!("TG{i:02}"), "x".repeat(900)));
    }
    big_metadata_set.push(("title".to_string(), "t".repeat(900)));
    // Use max_packets: Some(1) so single-input WAVE decode only needs ~131 KiB,
    // while 33 metadata tags (~30 KiB * 8 = ~240 KiB) push output metadata over 200 KiB.
    let tight_mix_controls = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(200 * 1024),
        metadata_set: big_metadata_set.clone(),
        ..Default::default()
    };
    let generous_mix_controls = fvid::media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        metadata_set: vec![("title".to_string(), "Budget Mix Title".to_string())],
        ..Default::default()
    };
    let mix_sources = vec![wave_source.clone(), wave_source.clone()];
    let mix_options = fvid::media::MixAudioOptions::default();

    let mix_plan_err = fvid::media::plan_mix_audio_with_controls(
        &mix_sources,
        &mix_options,
        &tight_mix_controls,
    )
    .expect_err("plan_mix_audio_with_controls must refuse controlled memory budget exceeded by metadata + mixer buffers");
    assert!(
        mix_plan_err.contains("controlled memory budget exceeded"),
        "unexpected plan_mix_audio_with_controls error: {mix_plan_err}"
    );

    for ext in ["wav", "mka"] {
        let bad_mix_out = dir.join(format!("bad-mix.{ext}"));
        let mix_err = fvid::media::mix_audio_with_controls(
            &mix_sources,
            &bad_mix_out,
            &mix_options,
            &tight_mix_controls,
        )
        .expect_err("mix_audio_with_controls must refuse controlled memory budget exceeded");
        assert!(
            mix_err.contains("controlled memory budget exceeded"),
            "unexpected mix_audio_with_controls ({ext}) error: {mix_err}"
        );
        assert!(!bad_mix_out.exists());

        let ok_mix_out = dir.join(format!("ok-mix.{ext}"));
        fvid::media::mix_audio_with_controls(
            &mix_sources,
            &ok_mix_out,
            &mix_options,
            &generous_mix_controls,
        )
        .unwrap_or_else(|e| panic!("mix_audio_with_controls ({ext}) with generous budget failed: {e}"));
        assert!(ok_mix_out.is_file());
    }

    // 2. merge_audio_with_controls & plan_merge_audio_with_controls must enforce max_controlled_bytes
    // against the merger working set + output WAVE INFO / Matroska metadata buffers.
    let merge_plan_err = fvid::media::plan_merge_audio_with_controls(
        &mix_sources,
        &tight_mix_controls,
    )
    .expect_err("plan_merge_audio_with_controls must refuse controlled memory budget exceeded by metadata + merger buffers");
    assert!(
        merge_plan_err.contains("controlled memory budget exceeded"),
        "unexpected plan_merge_audio_with_controls error: {merge_plan_err}"
    );

    for ext in ["wav", "mka"] {
        let bad_merge_out = dir.join(format!("bad-merge.{ext}"));
        let merge_err = fvid::media::merge_audio_with_controls(
            &mix_sources,
            &bad_merge_out,
            &tight_mix_controls,
        )
        .expect_err("merge_audio_with_controls must refuse controlled memory budget exceeded");
        assert!(
            merge_err.contains("controlled memory budget exceeded"),
            "unexpected merge_audio_with_controls ({ext}) error: {merge_err}"
        );
        assert!(!bad_merge_out.exists());

        let ok_merge_out = dir.join(format!("ok-merge.{ext}"));
        fvid::media::merge_audio_with_controls(
            &mix_sources,
            &ok_merge_out,
            &generous_mix_controls,
        )
        .unwrap_or_else(|e| panic!("merge_audio_with_controls ({ext}) with generous budget failed: {e}"));
        assert!(ok_merge_out.is_file());
    }

    // 3. Sample-domain concat & plan_concat (e.g. mixed WAVE + ADTS or stream-scoped metadata)
    // must enforce max_controlled_bytes against the concat working set + output metadata.
    let concat_sources = vec![wave_source.clone(), adts_source.clone()];
    let mut tight_concat_controls = tight_mix_controls.clone();
    tight_concat_controls.max_packets = Some(1);
    tight_concat_controls.max_controlled_bytes = Some(200 * 1024);
    let concat_plan_err = fvid::media::plan_concat(&concat_sources, &tight_concat_controls)
        .expect_err("sample-domain plan_concat must refuse controlled memory budget exceeded");
    assert!(
        concat_plan_err.contains("controlled memory budget exceeded"),
        "unexpected sample-domain plan_concat error: {concat_plan_err}"
    );

    for ext in ["wav", "mka"] {
        let bad_concat_out = dir.join(format!("bad-concat.{ext}"));
        let concat_err = fvid::media::concat(&concat_sources, &bad_concat_out, &tight_concat_controls)
            .expect_err("sample-domain concat must refuse controlled memory budget exceeded");
        assert!(
            concat_err.contains("controlled memory budget exceeded"),
            "unexpected sample-domain concat ({ext}) error: {concat_err}"
        );
        assert!(!bad_concat_out.exists());

        let ok_concat_out = dir.join(format!("ok-concat.{ext}"));
        fvid::media::concat(&concat_sources, &ok_concat_out, &generous_mix_controls)
            .unwrap_or_else(|e| panic!("sample-domain concat ({ext}) with generous budget failed: {e}"));
        assert!(ok_concat_out.is_file());
    }

    // 4. Matroska metadata + publish working set enforcement across decode_audio, trim,
    // normalize_loudness, and loudnorm when stream-scoped / custom Matroska metadata is present.
    // Note: with max_packets: Some(1), a single WAVE packet decode needs ~131 KiB,
    // while 32 stream tags (~30 KiB * 8 = ~240 KiB) + Matroska sink buffers exceeds 200 KiB.
    let mut big_stream_metadata = Vec::new();
    for i in 0..32 {
        big_stream_metadata.push((0, format!("STREAM_TAG_{i:02}"), "s".repeat(900)));
    }
    let tight_mka_stream_opts = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(200 * 1024),
        stream_metadata_set: big_stream_metadata,
        ..Default::default()
    };
    let generous_mka_stream_opts = fvid::media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        stream_metadata_set: vec![(
            0,
            "COMMENT".to_string(),
            "Matroska Stream Comment".to_string(),
        )],
        ..Default::default()
    };

    assert!(
        fvid::media::plan_decode_audio(
            &wave_source,
            &fvid::media::AudioDecodeTransform::default(),
            &tight_mka_stream_opts,
        )
        .is_err(),
        "plan_decode_audio must refuse stream metadata exceeding max_controlled_bytes"
    );
    let bad_decode_mka = dir.join("bad-decode-stream.mka");
    assert!(
        fvid::media::decode_audio_transformed(
            &wave_source,
            &bad_decode_mka,
            fvid::media::AudioDecodeTransform::default(),
            &tight_mka_stream_opts,
        )
        .is_err(),
        "decode_audio_transformed (.mka) must refuse stream metadata exceeding max_controlled_bytes"
    );
    assert!(!bad_decode_mka.exists());

    let ok_decode_mka = dir.join("ok-decode-stream.mka");
    fvid::media::decode_audio_transformed(
        &wave_source,
        &ok_decode_mka,
        fvid::media::AudioDecodeTransform::default(),
        &generous_mka_stream_opts,
    )
    .expect("decode_audio_transformed (.mka) with generous budget and stream metadata");
    assert!(ok_decode_mka.is_file());

    let bad_trim_mka = dir.join("bad-trim-stream.mka");
    assert!(
        fvid::media::trim(&wave_source, &bad_trim_mka, 0, 100_000, &tight_mka_stream_opts).is_err(),
        "trim (.mka) must refuse stream metadata exceeding max_controlled_bytes"
    );
    assert!(!bad_trim_mka.exists());

    let target = fvid::media_info::NormalizeTarget::default();
    assert!(
        fvid::media::plan_normalize_loudness(&wave_source, &tight_mka_stream_opts, None, target)
            .is_err(),
        "plan_normalize_loudness must refuse stream metadata exceeding max_controlled_bytes"
    );
    let mut tight_norm_stream_opts = tight_mka_stream_opts.clone();
    tight_norm_stream_opts.max_packets = None;
    let bad_norm_mka = dir.join("bad-norm-stream.mka");
    assert!(
        fvid::media::normalize_loudness(
            &wave_source,
            &bad_norm_mka,
            &tight_norm_stream_opts,
            None,
            target,
        )
        .is_err(),
        "normalize_loudness (.mka) must refuse stream metadata exceeding max_controlled_bytes"
    );
    assert!(!bad_norm_mka.exists());

    assert!(
        fvid::media::plan_loudnorm(&wave_source, None, false, &tight_mka_stream_opts)
            .is_err(),
        "plan_loudnorm must refuse stream metadata exceeding max_controlled_bytes"
    );
    let bad_ln_mka = dir.join("bad-loudnorm-stream.mka");
    assert!(
        fvid::media::apply_loudnorm(
            &wave_source,
            &bad_ln_mka,
            None,
            &tight_mka_stream_opts,
        )
        .is_err(),
        "apply_loudnorm (.mka) must refuse stream metadata exceeding max_controlled_bytes"
    );
    assert!(!bad_ln_mka.exists());

    let ok_ln_mka = dir.join("ok-loudnorm-stream.mka");
    fvid::media::apply_loudnorm(
        &wave_source,
        &ok_ln_mka,
        None,
        &generous_mka_stream_opts,
    )
    .expect("apply_loudnorm (.mka) with generous budget and stream metadata");
    assert!(ok_ln_mka.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fvid_media_loudnorm_decode_audio_and_mp4_concat_enforce_controlled_memory_for_metadata_working_sets() {
    let dir = temp_dir("fvid-media-loudnorm-decode-concat-meta-budget");
    let wave_source = fixture("pcm-s16le-48k-450ms.wav");
    let adts_source = fixture("aac-lc-48k.adts");
    let mp4_aac = dir.join("source-mp4-aac.m4a");
    fvid::media::remux(&adts_source, &mp4_aac, &fvid::media::CopyOptions::default()).unwrap();

    // 32 RIFF INFO tags of 900 bytes each + title of 900 bytes (~30 KiB raw, ~240 KiB working set).
    let mut wave_meta_set = Vec::new();
    for i in 0..32 {
        wave_meta_set.push((format!("TG{i:02}"), "w".repeat(900)));
    }
    wave_meta_set.push(("title".to_string(), "t".repeat(900)));

    // 1. fvid_media::plan_loudnorm, apply_loudnorm, and apply_loudnorm_dual on WAVE
    // with linear gain (`measured_I=-20:measured_TP=-5:measured_LRA=3:measured_thresh=-30:linear=true`)
    // must enforce max_controlled_bytes against WAVE INFO metadata working set!
    let linear_args = "I=-16:TP=-1:LRA=11:measured_I=-20:measured_TP=-5:measured_LRA=3:measured_thresh=-30:linear=true";
    let tight_wave_ln = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(180 * 1024),
        metadata_set: wave_meta_set.clone(),
        ..Default::default()
    };
    let generous_wave_ln = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(8 * 1024 * 1024),
        metadata_set: wave_meta_set.clone(),
        ..Default::default()
    };
    assert!(
        fvid_media::plan_loudnorm(&wave_source, Some(linear_args), false, &tight_wave_ln).is_err(),
        "fvid_media::plan_loudnorm (WAVE linear) must reject metadata working set exceeding max_controlled_bytes"
    );
    let out_wave_ln_tight = dir.join("wave-ln-tight.wav");
    assert!(
        fvid_media::apply_loudnorm(&wave_source, &out_wave_ln_tight, Some(linear_args), &tight_wave_ln).is_err(),
        "fvid_media::apply_loudnorm (WAVE linear) must reject metadata working set exceeding max_controlled_bytes"
    );
    assert!(!out_wave_ln_tight.exists());
    let out_wave_ln_ok = dir.join("wave-ln-ok.wav");
    fvid_media::apply_loudnorm(&wave_source, &out_wave_ln_ok, Some(linear_args), &generous_wave_ln)
        .expect("fvid_media::apply_loudnorm (WAVE linear) with generous budget should succeed");
    assert!(out_wave_ln_ok.is_file());

    // 2. fvid_media::plan_decode_audio, plan_loudnorm, and apply_loudnorm on ADTS and MP4
    // must enforce max_controlled_bytes against output WAVE INFO metadata working set!
    // ADTS decode admission is ~142,400 bytes; with 33 WAVE INFO tags (+240 KiB), total > 300 KiB.
    let tight_adts_meta = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(260 * 1024),
        metadata_set: wave_meta_set.clone(),
        ..Default::default()
    };
    let generous_adts_meta = fvid::media::CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(64 * 1024 * 1024),
        metadata_set: wave_meta_set.clone(),
        ..Default::default()
    };
    for (label, src) in [("adts", adts_source.as_path()), ("mp4-aac", mp4_aac.as_path())] {
        assert!(
            fvid_media::plan_decode_audio(
                src,
                &fvid::media::AudioDecodeTransform::default(),
                &tight_adts_meta,
            )
            .is_err(),
            "{label} fvid_media::plan_decode_audio must reject WAVE INFO metadata working set exceeding max_controlled_bytes"
        );
        let out_dec_tight = dir.join(format!("{label}-dec-tight.wav"));
        assert!(
            fvid_media::decode_audio_transformed(
                src,
                &out_dec_tight,
                fvid::media::AudioDecodeTransform::default(),
                &tight_adts_meta,
            )
            .is_err(),
            "{label} fvid_media::decode_audio_transformed must reject WAVE INFO metadata working set exceeding max_controlled_bytes"
        );
        assert!(!out_dec_tight.exists());

        assert!(
            fvid_media::plan_loudnorm(src, Some(linear_args), false, &tight_adts_meta).is_err(),
            "{label} fvid_media::plan_loudnorm must reject WAVE INFO metadata working set exceeding max_controlled_bytes"
        );
        let out_ln_tight = dir.join(format!("{label}-ln-tight.wav"));
        assert!(
            fvid_media::apply_loudnorm(src, &out_ln_tight, Some(linear_args), &tight_adts_meta)
                .is_err(),
            "{label} fvid_media::apply_loudnorm must reject WAVE INFO metadata working set exceeding max_controlled_bytes"
        );
        assert!(!out_ln_tight.exists());

        fvid_media::plan_decode_audio(
            src,
            &fvid::media::AudioDecodeTransform::default(),
            &generous_adts_meta,
        )
        .expect("fvid_media::plan_decode_audio with generous budget should succeed");
        fvid_media::plan_loudnorm(src, Some(linear_args), false, &generous_adts_meta)
            .expect("fvid_media::plan_loudnorm with generous budget should succeed");
    }

    // 3. Compressed MP4-to-Matroska concat (`plan_concat` & `concat` to `.mka`/`.mkv`)
    // must enforce max_controlled_bytes against Matroska container/stream metadata working set!
    let mut mkv_meta_set = Vec::new();
    let mut mkv_stream_meta_set = Vec::new();
    for i in 0..32 {
        mkv_meta_set.push((format!("CONTAINER_TAG_{i:02}"), "c".repeat(900)));
        mkv_stream_meta_set.push((0usize, format!("STREAM_TAG_{i:02}"), "s".repeat(900)));
    }
    let tight_mp4_concat_meta = fvid::media::CopyOptions {
        max_controlled_bytes: Some(128 * 1024),
        metadata_set: mkv_meta_set.clone(),
        stream_metadata_set: mkv_stream_meta_set.clone(),
        ..Default::default()
    };
    let generous_mp4_concat_meta = fvid::media::CopyOptions {
        max_controlled_bytes: Some(4 * 1024 * 1024),
        metadata_set: mkv_meta_set,
        stream_metadata_set: mkv_stream_meta_set,
        ..Default::default()
    };
    let mp4_sources = vec![mp4_aac.clone(), mp4_aac.clone()];
    assert!(
        fvid::media::plan_concat(&mp4_sources, &tight_mp4_concat_meta).is_err(),
        "MP4-to-Matroska plan_concat must reject metadata working set exceeding max_controlled_bytes"
    );
    let out_mp4_concat_tight = dir.join("mp4-concat-tight.mka");
    assert!(
        fvid::media::concat(&mp4_sources, &out_mp4_concat_tight, &tight_mp4_concat_meta).is_err(),
        "MP4-to-Matroska concat must reject metadata working set exceeding max_controlled_bytes"
    );
    assert!(!out_mp4_concat_tight.exists());

    fvid::media::plan_concat(&mp4_sources, &generous_mp4_concat_meta)
        .expect("MP4-to-Matroska plan_concat with generous budget should succeed");
    let out_mp4_concat_ok = dir.join("mp4-concat-ok.mka");
    fvid::media::concat(&mp4_sources, &out_mp4_concat_ok, &generous_mp4_concat_meta)
        .expect("MP4-to-Matroska concat with generous budget should succeed");
    assert!(out_mp4_concat_ok.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}


