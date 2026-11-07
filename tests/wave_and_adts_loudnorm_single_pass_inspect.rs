use fvid::media::{
    apply_loudnorm, apply_loudnorm_dual, measure_loudness, plan_loudnorm, CopyOptions,
};
use std::path::Path;

#[test]
fn wave_loudnorm_and_adts_loudness_reuse_validated_readers_and_single_pass_info() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let adts = manifest.join("tests/fixtures/audio/aac-stereo.aac");

    let adts_loudness = measure_loudness(&adts, &CopyOptions::default()).expect("adts loudness");
    assert!(adts_loudness.sample_frames > 0);

    let wav_from_adts = std::env::temp_dir().join(format!(
        "fvid-batch640-from-adts-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&wav_from_adts);
    let adts_norm = apply_loudnorm(
        &adts,
        &wav_from_adts,
        Some("I=-16:TP=-1.5:LRA=11:linear=true"),
        &CopyOptions::default(),
    )
    .expect("adts loudnorm");
    assert!(adts_norm.sample_frames > 0);

    let wave_plan = plan_loudnorm(
        &wav_from_adts,
        Some("I=-18:TP=-1.5:LRA=11:linear=true"),
        false,
        &CopyOptions::default(),
    )
    .expect("wave loudnorm plan");
    assert_eq!(wave_plan.command, "loudnorm");

    let wav_linear = std::env::temp_dir().join(format!(
        "fvid-batch640-wave-linear-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&wav_linear);
    let linear_stats = apply_loudnorm(
        &wav_from_adts,
        &wav_linear,
        Some("I=-18:TP=-1.5:LRA=11:linear=true:measured_I=-16.0:measured_TP=-2.0:measured_LRA=5.0:measured_thresh=-26.0"),
        &CopyOptions::default(),
    )
    .expect("wave linear loudnorm");
    assert!(linear_stats.sample_frames > 0);
    let _ = std::fs::remove_file(&wav_linear);

    let wav_dynamic = std::env::temp_dir().join(format!(
        "fvid-batch640-wave-dynamic-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&wav_dynamic);
    let dynamic_stats = apply_loudnorm_dual(
        &wav_from_adts,
        &wav_dynamic,
        Some("I=-18:TP=-1.5:LRA=11:linear=false"),
        &CopyOptions::default(),
    )
    .expect("wave dynamic loudnorm");
    assert!(dynamic_stats.sample_frames > 0);
    let _ = std::fs::remove_file(&wav_dynamic);
    let _ = std::fs::remove_file(&wav_from_adts);
}
