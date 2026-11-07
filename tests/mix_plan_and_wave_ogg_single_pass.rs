use fvid_control::CopyOptions;
use fvid_media::{
    decode_audio_transformed, measure_loudness, mix_audio_with_controls, plan_decode_audio,
    plan_merge_audio_with_controls, plan_mix_audio_with_controls,
};
use fvid_media_info::{AudioDecodeTransform, MixAudioOptions};
use std::path::Path;

#[test]
fn mix_plan_adts_matroska_wave_and_ogg_single_pass_export() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let opus = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg");
    let vorbis =
        root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-stereo.ogg");
    let aac = root.join("tests/fixtures/audio/aac-stereo.aac");
    let mka = root.join("tests/fixtures/audio/aac-stereo.mka");

    let opts = CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };

    // FVID-0735: plan_mix_audio_with_controls and plan_merge_audio_with_controls reuse info.stage2_frames
    let mix_plan =
        plan_mix_audio_with_controls(&[opus.clone(), mka.clone()], &MixAudioOptions::default(), &opts)
            .unwrap();
    assert_eq!(mix_plan.command, "mix-audio");
    let merge_plan =
        plan_merge_audio_with_controls(&[opus.clone(), mka.clone()], &opts).unwrap();
    assert_eq!(merge_plan.command, "merge-audio");

    let out_mix = std::env::temp_dir().join(format!(
        "fvid-batch601-mix-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_mix);
    let mix_stats = mix_audio_with_controls(
        &[opus.clone(), mka.clone()],
        &out_mix,
        &MixAudioOptions::default(),
        &opts,
    )
    .unwrap();
    assert!(mix_stats.sample_frames > 0);

    // FVID-0736 & FVID-0737: ADTS and Matroska single-open in owned_audio_plan
    let aac_plan = plan_decode_audio(&aac, &AudioDecodeTransform::default(), &opts).unwrap();
    assert_eq!(aac_plan.command, "decode-audio");
    let mka_plan = plan_decode_audio(&mka, &AudioDecodeTransform::default(), &opts).unwrap();
    assert_eq!(mka_plan.command, "decode-audio");

    // FVID-0738: WAVE decode_audio_transformed reuses WaveInfo and INFO chunks from read_wave_with_admission
    let mut wav_opts = opts.clone();
    wav_opts
        .metadata_set
        .push(("artist".to_string(), "Batch601".to_string()));
    let out_wav_xform = std::env::temp_dir().join(format!(
        "fvid-batch601-wav-xform-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav_xform);
    let wav_stats = decode_audio_transformed(
        &out_mix,
        &out_wav_xform,
        AudioDecodeTransform {
            channels: Some(1),
            sample_rate: Some(24000),
            ..Default::default()
        },
        &wav_opts,
    )
    .unwrap();
    assert_eq!(wav_stats.channels, 1);
    assert_eq!(wav_stats.sample_rate, 24000);
    std::fs::remove_file(&out_wav_xform).unwrap();
    std::fs::remove_file(&out_mix).unwrap();

    // FVID-0739: Ogg single-pass open_selected_ogg across decode_audio_transformed and measure_loudness
    for (idx, src) in [&opus, &vorbis].iter().enumerate() {
        let loudness = measure_loudness(src, &opts).unwrap();
        assert_eq!(loudness.backend, "owned container audio loudness");
        let out_ogg = std::env::temp_dir().join(format!(
            "fvid-batch601-ogg-{idx}-{}.wav",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_ogg);
        let ogg_stats =
            decode_audio_transformed(src, &out_ogg, AudioDecodeTransform::default(), &wav_opts)
                .unwrap();
        assert!(ogg_stats.sample_frames > 0);
        std::fs::remove_file(&out_ogg).unwrap();
    }
}
