use std::path::Path;

#[test]
fn trim_audio_and_aac_plans_reuse_inspected_audio_source_info() {
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let opts = fvid::media::CopyOptions::default();
    let mka_plan =
        fvid::native_plan::trim_audio_with_options(mka, 0, 10_000, &opts).expect("trim_audio on mka");
    assert_eq!(mka_plan.command, "trim");
    assert_eq!(mka_plan.streams[0].codec, "alac");
}

#[test]
fn normalize_loudness_plan_reuses_metadata_across_wave_and_matroska_budget_checks() {
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let mut opts = fvid::media::CopyOptions::default();
    opts.max_controlled_bytes = Some(256 * 1024 * 1024);
    opts.metadata_set.push(("ARTIST".into(), "FVid".into()));
    let target = fvid::media::NormalizeTarget {
        integrated_lufs: -16.0,
        sample_peak_dbfs: -1.0,
    };
    let plan = fvid::media::plan_normalize_loudness(wav, &opts, None, target)
        .expect("plan normalize loudness with metadata budget check on wav");
    assert_eq!(plan.command, "normalize-loudness");

    let mka_plan = fvid::media::plan_normalize_loudness(mka, &opts, None, target)
        .expect("plan normalize loudness with metadata budget check on mka");
    assert_eq!(mka_plan.command, "normalize-loudness");
}
