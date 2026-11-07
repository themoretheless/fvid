use fvid::media::{
    apply_loudnorm, decode_audio, measure_loudness, plan_loudnorm, plan_mix_audio_with_controls,
    CopyOptions, MixAudioOptions,
};
use std::path::{Path, PathBuf};

#[test]
fn container_loudnorm_matroska_export_and_loudness_reuse_descriptors() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mka = manifest.join("tests/fixtures/audio/aac-stereo.mka");

    let mut controlled = CopyOptions::default();
    controlled.max_controlled_bytes = Some(32 * 1024 * 1024);

    let plan = plan_loudnorm(&mka, None, false, &controlled).expect("container loudnorm plan");
    assert_eq!(plan.command, "loudnorm");
    assert_eq!(plan.streams.len(), 1);

    let tmp_loudnorm = std::env::temp_dir().join(format!(
        "fvid-batch639-loudnorm-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp_loudnorm);
    let stats = apply_loudnorm(&mka, &tmp_loudnorm, None, &controlled)
        .expect("container loudnorm execution");
    assert!(stats.sample_frames > 0);
    let _ = std::fs::remove_file(&tmp_loudnorm);

    let tmp_decode = std::env::temp_dir().join(format!(
        "fvid-batch639-decode-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp_decode);
    let decoded = decode_audio(&mka, &tmp_decode, &controlled).expect("matroska decode_audio");
    assert!(decoded.sample_frames > 0);

    let loudness = measure_loudness(&tmp_decode, &CopyOptions::default()).expect("wave loudness");
    assert!(loudness.sample_frames > 0);
    let _ = std::fs::remove_file(&tmp_decode);

    let mka_loudness = measure_loudness(&mka, &controlled).expect("controlled matroska loudness");
    assert!(mka_loudness.sample_frames > 0);

    let mix_sources = vec![PathBuf::from(&mka), PathBuf::from(&mka)];
    let mix_plan = plan_mix_audio_with_controls(
        &mix_sources,
        &MixAudioOptions::default(),
        &controlled,
    )
    .expect("controlled mix plan");
    assert_eq!(mix_plan.command, "mix-audio");
}
