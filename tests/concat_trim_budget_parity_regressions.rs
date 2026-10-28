//! Regression tests for packet-byte budget and error parity across sample-domain
//! audio concat and standalone audio trim planning and execution (FVID-0257).

use fvid::media::{CopyOptions, concat, plan_concat, plan_trim, trim};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/concat-trim-budget-parity")
}

fn temp_out(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-concat-trim-budget-parity-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn sample_domain_concat_enforces_wave_and_container_packet_budgets_parity_between_plan_and_execution() {
    let w16 = fixture_dir().join("wave-stereo-s16-48khz.wav");
    let wf32 = fixture_dir().join("wave-stereo-f32-48khz.wav");
    let ogg = fixture_dir().join("ogg-opus-mono-48khz.ogg");
    let mp4 = fixture_dir().join("mp4-aac-mono-48khz.m4a");
    let out_dir = temp_out("sample-domain-concat");

    // 1. Mixed-precision WAVE inputs (16-bit frame_bytes=4 + 32-bit float frame_bytes=8)
    // With max_packet_bytes = 4, w16 fits 1 frame (4 bytes) but wf32 requires 8 bytes per frame.
    // Both plan_concat and concat (to .wav and .mka) must reject with a PCM sample-frame packet limit error.
    let mixed_wave = vec![w16.clone(), wf32.clone()];
    let mut wave_ok = CopyOptions::default();
    wave_ok.max_packet_bytes = 8;
    assert!(plan_concat(&mixed_wave, &wave_ok).is_ok());
    assert!(concat(&mixed_wave, &out_dir.join("ok-mixed.wav"), &wave_ok).is_ok());
    assert!(concat(&mixed_wave, &out_dir.join("ok-mixed.mka"), &wave_ok).is_ok());

    let mut wave_tight = CopyOptions::default();
    wave_tight.max_packet_bytes = 4;
    let err_plan_wave = plan_concat(&mixed_wave, &wave_tight).unwrap_err();
    assert!(
        err_plan_wave.contains("packet byte limit is smaller than one PCM sample frame")
            || err_plan_wave.contains("PCM packet limit cannot hold one sample frame"),
        "unexpected plan_concat mixed-wave error: {err_plan_wave}"
    );
    let err_exec_wav =
        concat(&mixed_wave, &out_dir.join("tight-mixed.wav"), &wave_tight).unwrap_err();
    assert!(
        err_exec_wav.contains("packet byte limit is smaller than one PCM sample frame")
            || err_exec_wav.contains("PCM packet limit cannot hold one sample frame"),
        "unexpected concat(.wav) mixed-wave error: {err_exec_wav}"
    );
    let err_exec_mka =
        concat(&mixed_wave, &out_dir.join("tight-mixed.mka"), &wave_tight).unwrap_err();
    assert!(
        err_exec_mka.contains("packet byte limit is smaller than one PCM sample frame")
            || err_exec_mka.contains("PCM packet limit cannot hold one sample frame"),
        "unexpected concat(.mka) mixed-wave error: {err_exec_mka}"
    );

    // 2. Mixed-codec single-stream inputs (MP4 AAC 4-byte packet + Ogg Opus 6-byte packet)
    // With max_packet_bytes = 5, MP4 AAC fits (4 <= 5) and Ogg Opus exceeds budget (6 > 5);
    // with reversed order and max_packet_bytes = 3, MP4 AAC exceeds budget (4 > 3).
    // Both plan_concat and concat must reject with the corresponding packet budget error.
    let mixed_codec = vec![mp4.clone(), ogg.clone()];
    let mut codec_ok = CopyOptions::default();
    codec_ok.max_packet_bytes = 6;
    assert_eq!(plan_concat(&mixed_codec, &codec_ok).map(|_| ()), Ok(()));
    assert_eq!(concat(&mixed_codec, &out_dir.join("ok-codec.wav"), &codec_ok).map(|_| ()), Ok(()));

    let mut codec_tight_ogg = CopyOptions::default();
    codec_tight_ogg.max_packet_bytes = 5;
    let err_plan_ogg = plan_concat(&mixed_codec, &codec_tight_ogg).unwrap_err();
    assert!(
        err_plan_ogg.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_concat mixed-codec (Ogg) error: {err_plan_ogg}"
    );
    let err_exec_ogg =
        concat(&mixed_codec, &out_dir.join("tight-ogg.wav"), &codec_tight_ogg).unwrap_err();
    assert!(
        err_exec_ogg.contains("Opus compressed packet exceeds byte limit"),
        "unexpected concat(.wav) mixed-codec (Ogg) error: {err_exec_ogg}"
    );

    let mut codec_tight = CopyOptions::default();
    codec_tight.max_packet_bytes = 3;
    let err_plan_codec = plan_concat(&mixed_codec, &codec_tight).unwrap_err();
    assert!(
        err_plan_codec.contains("MP4 packet exceeds budget")
            || err_plan_codec.contains("exceeds packet byte limit"),
        "unexpected plan_concat mixed-codec error: {err_plan_codec}"
    );
    let err_exec_codec =
        concat(&mixed_codec, &out_dir.join("tight-codec.wav"), &codec_tight).unwrap_err();
    assert!(
        err_exec_codec.contains("MP4 packet exceeds budget")
            || err_exec_codec.contains("exceeds packet byte limit"),
        "unexpected concat(.wav) mixed-codec error: {err_exec_codec}"
    );

    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn standalone_audio_trim_and_plan_trim_preserve_budget_and_stream_errors_instead_of_wave_fallback() {
    let ogg = fixture_dir().join("ogg-opus-mono-48khz.ogg");
    let out_dir = temp_out("standalone-trim");
    let out_wav = out_dir.join("trimmed.wav");
    let out_mka = out_dir.join("trimmed.mka");

    // Baseline with max_packet_bytes = 6 succeeds for both plan_trim and trim
    let mut ok_opts = CopyOptions::default();
    ok_opts.max_packet_bytes = 6;
    assert!(plan_trim(&ogg, 0, 10_000, &ok_opts).is_ok());
    assert!(trim(&ogg, &out_wav, 0, 10_000, &ok_opts).is_ok());
    assert!(trim(&ogg, &out_mka, 0, 10_000, &ok_opts).is_ok());

    // Tight packet budget (max_packet_bytes = 5 < 6-byte Opus packet) must report the Opus packet budget error,
    // NOT "native PCM trim requires RIFF/WAVE"
    let mut tight_opts = CopyOptions::default();
    tight_opts.max_packet_bytes = 5;
    let err_plan = plan_trim(&ogg, 0, 10_000, &tight_opts).unwrap_err();
    assert!(
        err_plan.contains("Opus compressed packet exceeds byte limit"),
        "unexpected plan_trim tight budget error: {err_plan}"
    );
    let err_trim_wav =
        trim(&ogg, &out_dir.join("tight.wav"), 0, 10_000, &tight_opts).unwrap_err();
    assert!(
        err_trim_wav.contains("Opus compressed packet exceeds byte limit"),
        "unexpected trim(.wav) tight budget error: {err_trim_wav}"
    );
    let err_trim_mka =
        trim(&ogg, &out_dir.join("tight.mka"), 0, 10_000, &tight_opts).unwrap_err();
    assert!(
        err_trim_mka.contains("Opus compressed packet exceeds byte limit"),
        "unexpected trim(.mka) tight budget error: {err_trim_mka}"
    );

    // Invalid stream selection (streams = [1]) on Ogg must report absent Ogg stream, NOT WAVE error
    let mut stream1_opts = CopyOptions::default();
    stream1_opts.streams = vec![1];
    let err_plan_s1 = plan_trim(&ogg, 0, 10_000, &stream1_opts).unwrap_err();
    assert!(
        err_plan_s1.contains("selected Ogg audio source stream is absent"),
        "unexpected plan_trim stream 1 error: {err_plan_s1}"
    );
    let err_trim_s1 =
        trim(&ogg, &out_dir.join("s1.wav"), 0, 10_000, &stream1_opts).unwrap_err();
    assert!(
        err_trim_s1.contains("selected Ogg audio source stream is absent"),
        "unexpected trim(.wav) stream 1 error: {err_trim_s1}"
    );

    let _ = std::fs::remove_dir_all(&out_dir);
}

#[test]
fn standalone_audio_concat_preserves_stream_selection_errors_instead_of_wave_fallback() {
    let ogg = fixture_dir().join("ogg-opus-mono-48khz.ogg");
    let out_dir = temp_out("standalone-concat-streams");
    let sources = vec![ogg.clone(), ogg.clone()];

    let mut stream1_opts = CopyOptions::default();
    stream1_opts.streams = vec![1];

    let err_plan = plan_concat(&sources, &stream1_opts).unwrap_err();
    assert!(
        err_plan.contains("audio output has only stream 0"),
        "unexpected plan_concat stream 1 error: {err_plan}"
    );
    let err_exec = concat(&sources, &out_dir.join("out.wav"), &stream1_opts).unwrap_err();
    assert!(
        err_exec.contains("audio output has only stream 0"),
        "unexpected concat(.wav) stream 1 error: {err_exec}"
    );
    let err_exec_mka = concat(&sources, &out_dir.join("out.mka"), &stream1_opts).unwrap_err();
    assert!(
        err_exec_mka.contains("audio output has only stream 0"),
        "unexpected concat(.mka) stream 1 error: {err_exec_mka}"
    );

    let _ = std::fs::remove_dir_all(&out_dir);
}
