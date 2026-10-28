//! Regression tests for packet-byte budgets and stream-selection parity across
//! Ogg Opus/Vorbis/joint, G.711 WAVE, and standalone container loudness and loudnorm paths (FVID-0256).

use fvid::media::{
    AudioDecodeTransform, CopyOptions, MixAudioOptions, NormalizeTarget, apply_loudnorm,
    decode_audio_transformed, measure_loudness, normalize_loudness, plan_decode_audio,
    plan_loudness, plan_loudnorm, plan_mix_audio_with_controls, plan_normalize_loudness,
};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget")
}

fn temp_out(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-ogg-g711-loudnorm-budget-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn ogg_opus_enforces_max_packet_bytes_parity_across_plan_decode_loudness_loudnorm_and_mix() {
    let ogg = fixture_dir().join("ogg-opus-mono-4byte-pkt.ogg");
    let out_dir = temp_out("ogg-pkt-budget");
    let out_decode = out_dir.join("decoded.wav");
    let out_norm = out_dir.join("loudnorm.wav");
    let out_norm_const = out_dir.join("norm.wav");

    // Baseline with max_packet_bytes = 4 (exact packet size) succeeds everywhere
    let mut ok_opts = CopyOptions::default();
    ok_opts.max_packet_bytes = 4;
    assert!(plan_decode_audio(&ogg, &AudioDecodeTransform::default(), &ok_opts).is_ok());
    assert!(plan_loudness(&ogg, &ok_opts).is_ok());
    assert!(plan_loudnorm(&ogg, None, false, &ok_opts).is_ok());
    assert!(plan_normalize_loudness(&ogg, &ok_opts, None, NormalizeTarget::default()).is_ok());
    assert!(
        plan_mix_audio_with_controls(
            &[ogg.clone(), ogg.clone()],
            &MixAudioOptions::default(),
            &ok_opts,
        )
        .is_ok()
    );
    assert!(
        decode_audio_transformed(&ogg, &out_decode, AudioDecodeTransform::default(), &ok_opts)
            .is_ok()
    );
    assert!(measure_loudness(&ogg, &ok_opts).is_ok());
    assert!(apply_loudnorm(&ogg, &out_norm, None, &ok_opts).is_ok());

    // Tight budget (max_packet_bytes = 3 < 4-byte Opus packet) must be rejected across all plan and execution paths
    let mut tight_opts = CopyOptions::default();
    tight_opts.max_packet_bytes = 3;

    let err_plan_dec =
        plan_decode_audio(&ogg, &AudioDecodeTransform::default(), &tight_opts).unwrap_err();
    assert!(
        err_plan_dec.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_decode_audio error: {err_plan_dec}"
    );
    let err_plan_loud = plan_loudness(&ogg, &tight_opts).unwrap_err();
    assert!(
        err_plan_loud.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_loudness error: {err_plan_loud}"
    );
    let err_plan_norm = plan_loudnorm(&ogg, None, false, &tight_opts).unwrap_err();
    assert!(
        err_plan_norm.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_loudnorm error: {err_plan_norm}"
    );
    let err_plan_const =
        plan_normalize_loudness(&ogg, &tight_opts, None, NormalizeTarget::default()).unwrap_err();
    assert!(
        err_plan_const.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_normalize_loudness error: {err_plan_const}"
    );
    let err_plan_mix = plan_mix_audio_with_controls(
        &[ogg.clone(), ogg.clone()],
        &MixAudioOptions::default(),
        &tight_opts,
    )
    .unwrap_err();
    assert!(
        err_plan_mix.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_mix_audio error: {err_plan_mix}"
    );

    let out_tight = out_dir.join("tight.wav");
    assert!(
        decode_audio_transformed(
            &ogg,
            &out_tight,
            AudioDecodeTransform::default(),
            &tight_opts
        )
        .unwrap_err()
        .contains("Opus compressed packet exceeds byte limit")
    );
    assert!(
        measure_loudness(&ogg, &tight_opts)
            .unwrap_err()
            .contains("Opus compressed packet exceeds byte limit")
    );
    assert!(
        apply_loudnorm(&ogg, &out_tight, None, &tight_opts)
            .unwrap_err()
            .contains("Opus compressed packet exceeds byte limit")
    );
    assert!(
        normalize_loudness(
            &ogg,
            &out_norm_const,
            &tight_opts,
            None,
            NormalizeTarget::default()
        )
        .unwrap_err()
        .contains("Opus compressed packet exceeds byte limit")
    );
    assert!(!out_tight.exists());
    assert!(!out_norm_const.exists());
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn g711_wave_enforces_max_packet_bytes_parity_across_plan_decode_loudness_loudnorm_and_mix() {
    let g711 = fixture_dir().join("wave-stereo-alaw-8khz.wav");
    let out_dir = temp_out("g711-pkt-budget");
    let out_decode = out_dir.join("decoded.wav");
    let out_norm = out_dir.join("loudnorm.wav");

    // Baseline with max_packet_bytes = 2 (1 stereo channel frame = 2 bytes) succeeds everywhere
    let mut ok_opts = CopyOptions::default();
    ok_opts.max_packet_bytes = 2;
    assert!(plan_decode_audio(&g711, &AudioDecodeTransform::default(), &ok_opts).is_ok());
    assert!(plan_loudness(&g711, &ok_opts).is_ok());
    assert!(plan_loudnorm(&g711, None, false, &ok_opts).is_ok());
    assert!(
        plan_mix_audio_with_controls(
            &[g711.clone(), g711.clone()],
            &MixAudioOptions::default(),
            &ok_opts,
        )
        .is_ok()
    );
    assert!(
        decode_audio_transformed(&g711, &out_decode, AudioDecodeTransform::default(), &ok_opts)
            .is_ok()
    );
    assert!(measure_loudness(&g711, &ok_opts).is_ok());
    assert!(apply_loudnorm(&g711, &out_norm, None, &ok_opts).is_ok());

    // Tight budget (max_packet_bytes = 1 < 2 channels) must be rejected across all plan and execution paths
    let mut tight_opts = CopyOptions::default();
    tight_opts.max_packet_bytes = 1;

    let err_plan_dec =
        plan_decode_audio(&g711, &AudioDecodeTransform::default(), &tight_opts).unwrap_err();
    assert!(
        err_plan_dec.contains("G.711 channel frame exceeds byte limit"),
        "unexpected plan_decode_audio error: {err_plan_dec}"
    );
    let err_plan_loud = plan_loudness(&g711, &tight_opts).unwrap_err();
    assert!(
        err_plan_loud.contains("G.711 channel frame exceeds byte limit"),
        "unexpected plan_loudness error: {err_plan_loud}"
    );
    let err_plan_norm = plan_loudnorm(&g711, None, false, &tight_opts).unwrap_err();
    assert!(
        err_plan_norm.contains("G.711 channel frame exceeds byte limit"),
        "unexpected plan_loudnorm error: {err_plan_norm}"
    );
    let err_plan_mix = plan_mix_audio_with_controls(
        &[g711.clone(), g711.clone()],
        &MixAudioOptions::default(),
        &tight_opts,
    )
    .unwrap_err();
    assert!(
        err_plan_mix.contains("G.711 channel frame exceeds byte limit"),
        "unexpected plan_mix_audio error: {err_plan_mix}"
    );

    let out_tight = out_dir.join("tight.wav");
    assert!(
        decode_audio_transformed(
            &g711,
            &out_tight,
            AudioDecodeTransform::default(),
            &tight_opts
        )
        .unwrap_err()
        .contains("G.711 channel frame exceeds byte limit")
    );
    assert!(
        measure_loudness(&g711, &tight_opts)
            .unwrap_err()
            .contains("G.711 channel frame exceeds byte limit")
    );
    assert!(
        apply_loudnorm(&g711, &out_tight, None, &tight_opts)
            .unwrap_err()
            .contains("G.711 channel frame exceeds byte limit")
    );
    assert!(!out_tight.exists());
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn standalone_audio_and_adts_loudnorm_preserve_stream_selection_errors_even_for_matroska_output() {
    let ogg = fixture_dir().join("ogg-opus-mono-4byte-pkt.ogg");
    let g711 = fixture_dir().join("wave-stereo-alaw-8khz.wav");
    let adts = fixture_dir().join("adts-mono-6byte-pkt.aac");
    let out_dir = temp_out("stream-selection");
    let out_mka = out_dir.join("out.mka");
    let out_wav = out_dir.join("out.wav");

    let mut stream1_opts = CopyOptions::default();
    stream1_opts.streams = vec![1];

    // Ogg with stream 1 selected must fail with absent stream error for both .wav and .mka
    let err_ogg_plan = plan_loudnorm(&ogg, None, false, &stream1_opts).unwrap_err();
    assert!(
        err_ogg_plan.contains("selected Ogg audio source stream is absent"),
        "unexpected ogg plan_loudnorm error: {err_ogg_plan}"
    );
    let err_ogg_mka = apply_loudnorm(&ogg, &out_mka, None, &stream1_opts).unwrap_err();
    assert!(
        err_ogg_mka.contains("selected Ogg audio source stream is absent"),
        "unexpected ogg apply_loudnorm(.mka) error: {err_ogg_mka}"
    );

    // G.711 WAVE with stream 1 selected must fail with WAVE stream 0 error for both .wav and .mka
    let err_g711_plan = plan_loudnorm(&g711, None, false, &stream1_opts).unwrap_err();
    assert!(
        err_g711_plan.contains("WAVE has only stream 0"),
        "unexpected g711 plan_loudnorm error: {err_g711_plan}"
    );
    let err_g711_mka = apply_loudnorm(&g711, &out_mka, None, &stream1_opts).unwrap_err();
    assert!(
        err_g711_mka.contains("WAVE has only stream 0"),
        "unexpected g711 apply_loudnorm(.mka) error: {err_g711_mka}"
    );

    // ADTS with stream 1 selected must fail with ADTS stream 0 error for both .wav and .mka
    let err_adts_plan = plan_loudnorm(&adts, None, false, &stream1_opts).unwrap_err();
    assert!(
        err_adts_plan.contains("stream 0"),
        "unexpected adts plan_loudnorm error: {err_adts_plan}"
    );
    let err_adts_mka = apply_loudnorm(&adts, &out_mka, None, &stream1_opts).unwrap_err();
    assert!(
        err_adts_mka.contains("stream 0"),
        "unexpected adts apply_loudnorm(.mka) error: {err_adts_mka}"
    );

    // Multi-stream selection [0, 1] must fail with single-stream requirement on all three
    let mut multi_opts = CopyOptions::default();
    multi_opts.streams = vec![0, 1];
    for source in [&ogg, &g711, &adts] {
        assert!(plan_loudnorm(source, None, false, &multi_opts).is_err());
        assert!(apply_loudnorm(source, &out_wav, None, &multi_opts).is_err());
        assert!(apply_loudnorm(source, &out_mka, None, &multi_opts).is_err());
    }

    assert!(!out_mka.exists());
    assert!(!out_wav.exists());
    let _ = std::fs::remove_dir_all(&out_dir);
}
