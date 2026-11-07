use fvid::media::{
    apply_loudnorm, apply_loudnorm_dual, measure_loudness, CopyOptions,
};
use std::path::Path;

#[test]
fn spool_loudness_and_wave_loudnorm_reuse_validated_handles() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mka = manifest.join("tests/fixtures/audio/aac-stereo.mka");
    let adts = manifest.join("tests/fixtures/audio/aac-stereo.aac");

    let mka_loudness = measure_loudness(&mka, &CopyOptions::default()).expect("mka loudness");
    assert!(mka_loudness.sample_frames > 0);

    let adts_loudness = measure_loudness(&adts, &CopyOptions::default()).expect("adts loudness");
    assert!(adts_loudness.sample_frames > 0);

    let mka_wav = std::env::temp_dir().join(format!(
        "fvid-batch641-mka-norm-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&mka_wav);
    let mka_norm = apply_loudnorm_dual(&mka, &mka_wav, None, &CopyOptions::default())
        .expect("mka dual loudnorm");
    assert!(mka_norm.sample_frames > 0);

    let adts_wav = std::env::temp_dir().join(format!(
        "fvid-batch641-adts-norm-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&adts_wav);
    let adts_norm = apply_loudnorm(&adts, &adts_wav, None, &CopyOptions::default())
        .expect("adts loudnorm");
    assert!(adts_norm.sample_frames > 0);

    let wave_dual = std::env::temp_dir().join(format!(
        "fvid-batch641-wave-dual-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&wave_dual);
    let dual_stats = apply_loudnorm_dual(&adts_wav, &wave_dual, None, &CopyOptions::default())
        .expect("wave dual loudnorm");
    assert!(dual_stats.sample_frames > 0);
    assert!(dual_stats.dual_pass);

    let _ = std::fs::remove_file(&mka_wav);
    let _ = std::fs::remove_file(&adts_wav);
    let _ = std::fs::remove_file(&wave_dual);
}
