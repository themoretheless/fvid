use fvid_control::CopyOptions;
use fvid_media::{
    apply_loudnorm, apply_loudnorm_dual, decode_audio_transformed, measure_loudness,
    plan_decode_audio, plan_loudness, plan_loudnorm,
};
use fvid_media_info::AudioDecodeTransform;
use std::path::Path;

#[test]
fn ogg_opus_export_and_loudnorm_single_pass_info_and_geometry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let opus = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg");
    let aac = root.join("tests/fixtures/audio/aac-stereo.aac");
    let mka = root.join("tests/fixtures/audio/aac-stereo.mka");
    let flac_mka = root.join("tests/fixtures/playback-errors/container-loudness-plan/flac-loudness.mka");

    let mut options = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    options
        .metadata_set
        .push(("title".to_string(), "Batch600".to_string()));

    // FVID-0730: Ogg Opus decode/apply reuses source_info from open_with_transform_and_info
    let out_opus = std::env::temp_dir().join(format!(
        "fvid-batch600-opus-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_opus);
    let plan = plan_decode_audio(&opus, &AudioDecodeTransform::default(), &options).unwrap();
    assert_eq!(plan.command, "decode-audio");
    let stats =
        decode_audio_transformed(&opus, &out_opus, AudioDecodeTransform::default(), &options)
            .unwrap();
    assert!(stats.sample_frames > 0);
    std::fs::remove_file(&out_opus).unwrap();

    // FVID-0731: ADTS loudness and loudnorm single-pass configuration & channels validation
    let loudness_options = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let aac_plan = plan_loudness(&aac, &loudness_options).unwrap();
    assert_eq!(aac_plan.command, "loudness");
    let aac_ln_plan = plan_loudnorm(&aac, Some("print_format=json"), true, &options).unwrap();
    assert_eq!(aac_ln_plan.command, "loudnorm");
    let out_aac = std::env::temp_dir().join(format!(
        "fvid-batch600-aac-ln-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_aac);
    let aac_ln = apply_loudnorm_dual(&aac, &out_aac, None, &options).unwrap();
    assert!(aac_ln.dual_pass);
    std::fs::remove_file(&out_aac).unwrap();

    // FVID-0732: Container loudness and loudnorm reuse geometry via decode_to_wave_from_geometry
    let mka_loudness = measure_loudness(&mka, &loudness_options).unwrap();
    assert_eq!(mka_loudness.backend, "owned container audio loudness");
    let mka_ln_plan = plan_loudnorm(&mka, Some("print_format=summary"), false, &options).unwrap();
    assert_eq!(mka_ln_plan.command, "loudnorm");
    let out_mka = std::env::temp_dir().join(format!(
        "fvid-batch600-mka-ln-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_mka);
    let mka_ln = apply_loudnorm(&mka, &out_mka, None, &options).unwrap();
    assert!(!mka_ln.dual_pass);
    std::fs::remove_file(&out_mka).unwrap();

    // FVID-0733: WAVE loudnorm supports/supports_dual single header inspection
    let out_wav_src = std::env::temp_dir().join(format!(
        "fvid-batch600-wav-src-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav_src);
    decode_audio_transformed(&aac, &out_wav_src, AudioDecodeTransform::default(), &loudness_options).unwrap();
    let wav_ln_plan = plan_loudnorm(&out_wav_src, Some("print_format=json"), true, &options).unwrap();
    assert_eq!(wav_ln_plan.command, "loudnorm");
    let out_wav = std::env::temp_dir().join(format!(
        "fvid-batch600-wav-ln-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav);
    let wav_ln = apply_loudnorm_dual(&out_wav_src, &out_wav, None, &options).unwrap();
    assert!(wav_ln.dual_pass);
    std::fs::remove_file(&out_wav).unwrap();
    std::fs::remove_file(&out_wav_src).unwrap();

    // FVID-0734: AAC export routing reuses opened File handle across Matroska and ADTS
    for (idx, src) in [&flac_mka, &mka, &aac].iter().enumerate() {
        let out_fold = std::env::temp_dir().join(format!(
            "fvid-batch600-fold-{idx}-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_fold);
        let transform = AudioDecodeTransform {
            channels: Some(1),
            ..Default::default()
        };
        let fold_stats = decode_audio_transformed(src, &out_fold, transform, &options).unwrap();
        assert_eq!(fold_stats.channels, 1);
        std::fs::remove_file(&out_fold).unwrap();
    }
}
