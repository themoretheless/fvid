use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};
use fvid::media_info::NormalizeTarget;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-max-packets-budget")
        .join(name)
}

fn temp_wav_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fvid-batch263-{}-{}-{}.wav",
        tag,
        std::process::id(),
        nanos
    ))
}

#[test]
fn unedited_mp4_audio_packet_byte_and_controlled_budgets_scope_to_max_packets_prefix() {
    let sowt_m4a = fixture("prefix-sowt.m4a");

    // Packet 0 is 16 bytes (8 mono s16le frames at 8000 Hz) and packet 1 is 256 bytes (128 frames).
    // With max_packet_bytes = 64 and max_packets = Some(1), packet 0 is within budget and
    // packet 1 is outside the active prefix, so plan/decode/loudness/loudnorm/mix/merge must succeed
    // both with and without max_controlled_bytes.
    let prefix_opts = CopyOptions {
        max_packet_bytes: 64,
        max_packets: Some(1),
        ..CopyOptions::default()
    };
    let controlled_prefix_opts = CopyOptions {
        max_packet_bytes: 64,
        max_packets: Some(1),
        max_controlled_bytes: Some(8 * 1024 * 1024),
        ..CopyOptions::default()
    };

    for (label, opts) in [
        ("uncontrolled", &prefix_opts),
        ("controlled", &controlled_prefix_opts),
    ] {
        let plan = media::plan_decode_audio(
            &sowt_m4a,
            &AudioDecodeTransform::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("plan_decode_audio ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        assert_eq!(plan.streams.len(), 1);
        assert!(
            matches!(plan.streams[0].codec.as_str(), "pcm_s16le" | "pcm_sle"),
            "unexpected codec: {}",
            plan.streams[0].codec
        );

        let wav = temp_wav_path(&format!("decode-{label}"));
        let stats = media::decode_audio_transformed(
            &sowt_m4a,
            &wav,
            AudioDecodeTransform::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("decode_audio_transformed ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        let _ = std::fs::remove_file(&wav);
        assert_eq!(stats.decoded_frames, 1);
        assert_eq!(stats.sample_frames, 8);

        let loud_plan = media::plan_loudness(&sowt_m4a, opts)
            .unwrap_or_else(|e| panic!("plan_loudness ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        assert_eq!(loud_plan.streams.len(), 1);

        let loud_report = media::measure_loudness(&sowt_m4a, opts)
            .unwrap_or_else(|e| panic!("measure_loudness ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        assert_eq!(loud_report.sample_frames, 8);

        let norm_plan = media::plan_normalize_loudness(
            &sowt_m4a,
            opts,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_or_else(|e| panic!("plan_normalize_loudness ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        assert_eq!(norm_plan.streams.len(), 1);

        let norm_wav = temp_wav_path(&format!("norm-{label}"));
        let norm_err = media::normalize_loudness(
            &sowt_m4a,
            &norm_wav,
            opts,
            None,
            NormalizeTarget::default(),
        )
        .expect_err("8-sample prefix is shorter than 400 ms BS.1770 gating block")
        .to_string();
        let _ = std::fs::remove_file(&norm_wav);
        assert!(
            norm_err.contains("at least 400 ms"),
            "normalize_loudness ({label}) should reach BS.1770 gating instead of packet budget refusal: {norm_err}"
        );

        let ln_plan = media::plan_loudnorm(&sowt_m4a, None, false, opts)
            .unwrap_or_else(|e| panic!("plan_loudnorm ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        assert!(!ln_plan.steps.is_empty());

        let ln_wav = temp_wav_path(&format!("ln-{label}"));
        let ln_stats = media::apply_loudnorm(&sowt_m4a, &ln_wav, None, opts)
            .unwrap_or_else(|e| panic!("apply_loudnorm ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        let _ = std::fs::remove_file(&ln_wav);
        assert_eq!(ln_stats.sample_rate, 192_000);
        assert_eq!(ln_stats.sample_frames, 192);

        let mix_wav = temp_wav_path(&format!("mix-{label}"));
        let mix_inputs = vec![sowt_m4a.clone(), sowt_m4a.clone()];
        let mix_stats = media::mix_audio_with_controls(
            &mix_inputs,
            &mix_wav,
            &MixAudioOptions::default(),
            opts,
        )
        .unwrap_or_else(|e| panic!("mix_audio_with_controls ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        let _ = std::fs::remove_file(&mix_wav);
        assert_eq!(mix_stats.sample_frames, 8);

        let merge_wav = temp_wav_path(&format!("merge-{label}"));
        let merge_stats = media::merge_audio_with_controls(
            &mix_inputs,
            &merge_wav,
            opts,
        )
        .unwrap_or_else(|e| panic!("merge_audio_with_controls ({label}) on prefix-bounded MP4 sowt should succeed: {e}"));
        let _ = std::fs::remove_file(&merge_wav);
        assert_eq!(merge_stats.channels, 2);
        assert_eq!(merge_stats.sample_frames, 8);
    }

    // Also verify tight max_controlled_bytes admission does not count the 256-byte packet #2
    // when max_packets = Some(1).
    // With sowt expansion factor 10, packet 0 (16B) adds 160 bytes while packet 1 (256B) adds 2560 bytes.
    // A controlled budget of 36_000 bytes admits packet 0 (estimated 35_073 bytes) but rejects packet 1 (37_473 bytes).
    let tight_controlled_prefix_opts = CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(36_000),
        ..CopyOptions::default()
    };
    let tight_wav = temp_wav_path("tight-controlled-prefix");
    let tight_stats = media::decode_audio_transformed(
        &sowt_m4a,
        &tight_wav,
        AudioDecodeTransform::default(),
        &tight_controlled_prefix_opts,
    )
    .expect("tight controlled decode on 1-packet prefix should not charge packet #2 expansion");
    let _ = std::fs::remove_file(&tight_wav);
    assert_eq!(tight_stats.decoded_frames, 1);
    assert_eq!(tight_stats.sample_frames, 8);

    // Without max_packets = Some(1), the 36_000-byte controlled budget must refuse packet #2's expansion.
    let tight_controlled_full_opts = CopyOptions {
        max_packets: Some(2),
        max_controlled_bytes: Some(36_000),
        ..CopyOptions::default()
    };
    let tight_err = media::plan_decode_audio(
        &sowt_m4a,
        &AudioDecodeTransform::default(),
        &tight_controlled_full_opts,
    )
    .expect_err("2-packet prefix must include packet #2 in controlled memory estimate")
    .to_string();
    assert!(
        tight_err.contains("controlled memory budget exceeded"),
        "unexpected error: {tight_err}"
    );

    // When max_packets is Some(2), packet 1 (256B > 64B) is inside the prefix and must be rejected.
    for (label, max_controlled_bytes) in [
        ("uncontrolled", None),
        ("controlled", Some(2 * 1024 * 1024)),
    ] {
        let two_opts = CopyOptions {
            max_packet_bytes: 64,
            max_packets: Some(2),
            max_controlled_bytes,
            ..CopyOptions::default()
        };
        let err = media::plan_decode_audio(
            &sowt_m4a,
            &AudioDecodeTransform::default(),
            &two_opts,
        )
        .expect_err(&format!("2-packet prefix ({label}) must reject 256-byte packet #2 under 64-byte budget"))
        .to_string();
        assert!(
            err.contains("MP4 packet exceeds budget"),
            "unexpected error ({label}): {err}"
        );

        let loud_err = media::measure_loudness(&sowt_m4a, &two_opts)
            .expect_err(&format!("measure_loudness ({label}) on 2-packet prefix must reject packet #2"))
            .to_string();
        assert!(
            loud_err.contains("MP4 packet exceeds budget"),
            "unexpected loudness error ({label}): {loud_err}"
        );
    }
}
