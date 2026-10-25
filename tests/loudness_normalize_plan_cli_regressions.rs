use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/loudness-normalize-plan-cli")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn loudness_and_normalize_loudness_plan_cli_and_api_regressions() {
    let dir = fixture_dir();
    let wav = dir.join("stereo.wav");
    let mka = dir.join("stereo-pcm.mka");

    // 1. Public `fvid::media::plan_loudness` and `fvid::media::plan_normalize_loudness`
    //    must ignore `options.progress` (read-only planning never emits progress events).
    let mut opts_with_progress = fvid::media::CopyOptions::default();
    opts_with_progress.progress = Some(fvid::media_control::ProgressHook::new(|_| {
        panic!("plan must never invoke progress hook");
    }));

    for source in [&wav, &mka] {
        let base_loudness = fvid::media::plan_loudness(source, &Default::default())
            .expect("baseline plan_loudness");
        let prog_loudness = fvid::media::plan_loudness(source, &opts_with_progress)
            .expect("plan_loudness with progress hook should succeed");
        assert_eq!(
            serde_json::to_value(&prog_loudness).unwrap(),
            serde_json::to_value(&base_loudness).unwrap(),
            "plan_loudness with progress hook must match baseline for {}",
            source.display()
        );

        let target = fvid::media::NormalizeTarget::default();
        let base_norm = fvid::media::plan_normalize_loudness(source, &Default::default(), None, target)
            .expect("baseline plan_normalize_loudness");
        let prog_norm = fvid::media::plan_normalize_loudness(source, &opts_with_progress, None, target)
            .expect("plan_normalize_loudness with progress hook should succeed");
        assert_eq!(
            serde_json::to_value(&prog_norm).unwrap(),
            serde_json::to_value(&base_norm).unwrap(),
            "plan_normalize_loudness with progress hook must match baseline for {}",
            source.display()
        );
    }

    // 2. CLI `fvid media plan loudness INPUT --progress` must succeed and match baseline JSON.
    for source in [&wav, &mka] {
        let cli_loud = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "loudness",
                source.to_str().unwrap(),
                "--progress",
            ])
            .output()
            .unwrap();
        assert!(
            cli_loud.status.success(),
            "CLI plan loudness --progress failed for {}: {}",
            source.display(),
            String::from_utf8_lossy(&cli_loud.stderr)
        );
        assert!(
            cli_loud.stderr.is_empty(),
            "plan loudness --progress must not emit progress on stderr"
        );
        let plan_json: serde_json::Value = serde_json::from_slice(&cli_loud.stdout).unwrap();
        assert_eq!(plan_json["command"], "loudness");
    }

    // 3. CLI `fvid media plan normalize-loudness INPUT` (without requiring a destination path!)
    //    must succeed with options (`--target-lufs`, `--sample-peak-dbfs`, `--channel-weights`,
    //    `--streams`, `--metadata`, `--stream-metadata`, `--progress`, `--max-packets`, etc.)
    //    and match the plan produced when an explicit destination path is also given.
    for source in [&wav, &mka] {
        let single_arg_plan = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "normalize-loudness",
                source.to_str().unwrap(),
                "--target-lufs",
                "-18",
                "--sample-peak-dbfs",
                "-2",
                "--channel-weights",
                "1,1",
                "--streams",
                "0",
                "--metadata",
                "title=Planned Norm Title",
                "--metadata",
                "CUSTOM_NORM_KEY=CustomVal",
                "--stream-metadata",
                "0:language=eng",
                "--progress",
            ])
            .output()
            .unwrap();
        assert!(
            single_arg_plan.status.success(),
            "CLI plan normalize-loudness INPUT (without OUTPUT) failed for {}: {}",
            source.display(),
            String::from_utf8_lossy(&single_arg_plan.stderr)
        );
        assert!(single_arg_plan.stderr.is_empty());
        let single_json: serde_json::Value =
            serde_json::from_slice(&single_arg_plan.stdout).unwrap();
        assert_eq!(single_json["command"], "normalize-loudness");
        assert!(
            single_json["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["action"] == "metadata"),
            "expected metadata step in single-input plan normalize-loudness"
        );

        let two_arg_plan = Command::new(bin_path())
            .args([
                "media",
                "plan",
                "normalize-loudness",
                source.to_str().unwrap(),
                "planned-output.mka",
                "--target-lufs",
                "-18",
                "--sample-peak-dbfs",
                "-2",
                "--channel-weights",
                "1,1",
                "--streams",
                "0",
                "--metadata",
                "title=Planned Norm Title",
                "--metadata",
                "CUSTOM_NORM_KEY=CustomVal",
                "--stream-metadata",
                "0:language=eng",
                "--progress",
            ])
            .output()
            .unwrap();
        assert!(
            two_arg_plan.status.success(),
            "CLI plan normalize-loudness INPUT OUTPUT.mka failed: {}",
            String::from_utf8_lossy(&two_arg_plan.stderr)
        );
        let two_json: serde_json::Value = serde_json::from_slice(&two_arg_plan.stdout).unwrap();
        assert_eq!(single_json, two_json);
    }

    // 4. When an explicit destination is supplied to `plan normalize-loudness`, destination-specific
    //    constraints must still be enforced:
    //    - `.wav` destination rejects `--stream-metadata` and non-INFO container keys
    //    - unsupported extension (e.g. `.mp3`) is rejected
    //    - more than 2 positional paths is rejected
    let wav_with_stream_meta = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            wav.to_str().unwrap(),
            "planned-output.wav",
            "--stream-metadata",
            "0:title=BadOnWav",
        ])
        .output()
        .unwrap();
    assert!(
        !wav_with_stream_meta.status.success(),
        "plan normalize-loudness INPUT OUTPUT.wav must reject --stream-metadata"
    );

    let bad_ext_plan = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            wav.to_str().unwrap(),
            "planned-output.mp3",
        ])
        .output()
        .unwrap();
    assert!(
        !bad_ext_plan.status.success(),
        "plan normalize-loudness must reject unsupported output extension .mp3"
    );

    let too_many_paths = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "normalize-loudness",
            wav.to_str().unwrap(),
            "out1.wav",
            "out2.wav",
        ])
        .output()
        .unwrap();
    assert!(
        !too_many_paths.status.success(),
        "plan normalize-loudness must reject more than 2 positional paths"
    );

    // Execution `normalize-loudness` (without `plan`) must still require OUTPUT!
    let exec_missing_out = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            wav.to_str().unwrap(),
            "--target-lufs",
            "-18",
        ])
        .output()
        .unwrap();
    assert!(
        !exec_missing_out.status.success(),
        "normalize-loudness execution without OUTPUT must fail"
    );

    // 5. CLI `plan mix-audio` and `plan merge-audio` accept an optional leading destination path
    //    `[OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv] INPUT INPUT...` or `--output-format wav|mka|mkv|matroska`,
    //    and validate destination-specific metadata rules:
    //    - `.wav` / `--output-format wav` rejects `--stream-metadata` and non-RIFF-INFO container keys
    //    - `.mka` / `.mkv` / `--output-format mka|mkv` accepts Matroska container and stream 0 metadata
    //    - unsupported destination extensions (e.g. `.mp3`) are rejected cleanly.
    for ok_mix_merge_args in [
        vec![
            "media",
            "plan",
            "mix-audio",
            "planned-mix.mka",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "CUSTOM_MIX_TAG=MixVal",
            "--stream-metadata",
            "0:title=MixTrackZero",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "mix-audio",
            "--output-format",
            "wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "title=ValidWavMixTitle",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            "planned-merge.mka",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "CUSTOM_MERGE_TAG=MergeVal",
            "--stream-metadata",
            "0:title=MergeTrackZero",
            "--quiet",
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            "--output-format",
            "wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "title=ValidWavMergeTitle",
            "--quiet",
        ],
    ] {
        let out = Command::new(bin_path())
            .args(&ok_mix_merge_args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "expected success for {:?}: {}",
            ok_mix_merge_args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for bad_mix_merge_args in [
        vec![
            "media",
            "plan",
            "mix-audio",
            "planned-mix.wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--stream-metadata",
            "0:title=BadOnWavMix",
        ],
        vec![
            "media",
            "plan",
            "mix-audio",
            "--output-format",
            "wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "CUSTOM_NON_RIFF=BadOnWavMix",
        ],
        vec![
            "media",
            "plan",
            "mix-audio",
            "planned-mix.mp3",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            "planned-merge.wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--stream-metadata",
            "0:title=BadOnWavMerge",
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            "--output-format",
            "wav",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
            "--metadata",
            "CUSTOM_NON_RIFF=BadOnWavMerge",
        ],
        vec![
            "media",
            "plan",
            "merge-audio",
            "planned-merge.mp3",
            wav.to_str().unwrap(),
            mka.to_str().unwrap(),
        ],
    ] {
        let out = Command::new(bin_path())
            .args(&bad_mix_merge_args)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "expected failure for {:?}: {}",
            bad_mix_merge_args,
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("requires cargo build --release --features media"),
            "unexpected feature-gate fallback for {:?}: {stderr}",
            bad_mix_merge_args
        );
    }
}
