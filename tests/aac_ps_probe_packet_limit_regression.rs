use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};
use fvid::media_info::NormalizeTarget;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ps-probe-packet-limit")
}

#[test]
fn mp4_mono_aac_ps_probe_honors_max_packets_budget() {
    let mp4 = fixture_dir().join("prefix-mono-aac.m4a");
    let temp = std::env::temp_dir().join(format!("fvid-batch264-mp4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Without max_packets, packet 1 (48 bytes) exceeds max_packet_bytes (16 bytes).
    let full_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: None,
        ..Default::default()
    };
    let err = media::plan_decode_audio(&mp4, &AudioDecodeTransform::default(), &full_opts).unwrap_err();
    assert!(
        err.to_string().contains("MP4 packet exceeds budget"),
        "expected packet budget refusal without max_packets, got: {err}"
    );

    // With max_packets = Some(1), only packet 0 (8 bytes <= 16 bytes) is in the prefix.
    for max_controlled_bytes in [None, Some(64 * 1024 * 1024)] {
        let opts = CopyOptions {
            max_packet_bytes: 16,
            max_packets: Some(1),
            max_controlled_bytes,
            ..Default::default()
        };

        let plan = media::plan_decode_audio(&mp4, &AudioDecodeTransform::default(), &opts).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].codec, "aac");

        let wav = temp.join(format!("decode-{:?}.wav", max_controlled_bytes));
        let stats = media::decode_audio_transformed(
            &mp4,
            &wav,
            AudioDecodeTransform::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 1024);
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.sample_rate, 48_000);

        let xform = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        let xform_plan = media::plan_decode_audio(&mp4, &xform, &opts).unwrap();
        assert_eq!(xform_plan.streams.len(), 1);
        assert_eq!(xform_plan.streams[0].codec, "aac");
        let wav_xform = temp.join(format!("decode-xform-{:?}.wav", max_controlled_bytes));
        let xform_stats = media::decode_audio_transformed(&mp4, &wav_xform, xform, &opts).unwrap();
        assert_eq!(xform_stats.sample_frames, 1024);
        assert_eq!(xform_stats.channels, 2);

        let loud_plan = media::plan_loudness(&mp4, &opts).unwrap();
        assert_eq!(loud_plan.streams.len(), 1);
        let loud = media::measure_loudness(&mp4, &opts).unwrap();
        assert_eq!(loud.sample_frames, 1024);

        let norm_plan =
            media::plan_normalize_loudness(&mp4, &opts, None, NormalizeTarget::default()).unwrap();
        assert_eq!(norm_plan.streams.len(), 1);
        let wav_norm = temp.join(format!("norm-{:?}.wav", max_controlled_bytes));
        let norm_err = media::normalize_loudness(
            &mp4,
            &wav_norm,
            &opts,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_err();
        assert!(
            !norm_err.to_string().contains("MP4 packet exceeds budget"),
            "unexpected packet budget error in normalize_loudness: {norm_err}"
        );

        let ln_plan = media::plan_loudnorm(&mp4, None, false, &opts).unwrap();
        assert!(!ln_plan.steps.is_empty());
        let wav_ln = temp.join(format!("loudnorm-{:?}.wav", max_controlled_bytes));
        let ln_stats = media::apply_loudnorm(&mp4, &wav_ln, None, &opts).unwrap();
        assert_eq!(ln_stats.sample_rate, 192_000);
        assert_eq!(ln_stats.sample_frames, 4096);

        let wav_mix = temp.join(format!("mix-{:?}.wav", max_controlled_bytes));
        let mix_stats = media::mix_audio_with_controls(
            &[mp4.clone(), mp4.clone()],
            &wav_mix,
            &MixAudioOptions::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(mix_stats.sample_frames, 1024);

        let wav_merge = temp.join(format!("merge-{:?}.wav", max_controlled_bytes));
        let merge_stats = media::merge_audio_with_controls(
            &[mp4.clone(), mp4.clone()],
            &wav_merge,
            &opts,
        )
        .unwrap();
        assert_eq!(merge_stats.channels, 2);
        assert_eq!(merge_stats.sample_frames, 1024);
    }

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn matroska_mono_aac_ps_probe_honors_max_packets_budget() {
    let mka = fixture_dir().join("prefix-mono-aac.mka");
    let temp = std::env::temp_dir().join(format!("fvid-batch264-mka-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Without max_packets, packet 1 (48 bytes) exceeds max_packet_bytes (16 bytes).
    let full_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: None,
        ..Default::default()
    };
    let err = media::plan_decode_audio(&mka, &AudioDecodeTransform::default(), &full_opts).unwrap_err();
    assert!(
        err.to_string().contains("WebM packet exceeds budget"),
        "expected packet budget refusal without max_packets, got: {err}"
    );

    // With max_packets = Some(1), only packet 0 (8 bytes <= 16 bytes) is in the prefix.
    for max_controlled_bytes in [None, Some(64 * 1024 * 1024)] {
        let opts = CopyOptions {
            max_packet_bytes: 16,
            max_packets: Some(1),
            max_controlled_bytes,
            ..Default::default()
        };

        let plan = media::plan_decode_audio(&mka, &AudioDecodeTransform::default(), &opts).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].codec, "aac");

        let wav = temp.join(format!("decode-{:?}.wav", max_controlled_bytes));
        let stats = media::decode_audio_transformed(
            &mka,
            &wav,
            AudioDecodeTransform::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 1024);
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.sample_rate, 48_000);

        let xform = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        let xform_plan = media::plan_decode_audio(&mka, &xform, &opts).unwrap();
        assert_eq!(xform_plan.streams.len(), 1);
        assert_eq!(xform_plan.streams[0].codec, "aac");
        let wav_xform = temp.join(format!("decode-xform-{:?}.wav", max_controlled_bytes));
        let xform_stats = media::decode_audio_transformed(&mka, &wav_xform, xform, &opts).unwrap();
        assert_eq!(xform_stats.sample_frames, 1024);
        assert_eq!(xform_stats.channels, 2);

        let loud_plan = media::plan_loudness(&mka, &opts).unwrap();
        assert_eq!(loud_plan.streams.len(), 1);
        let loud = media::measure_loudness(&mka, &opts).unwrap();
        assert_eq!(loud.sample_frames, 1024);

        let norm_plan =
            media::plan_normalize_loudness(&mka, &opts, None, NormalizeTarget::default()).unwrap();
        assert_eq!(norm_plan.streams.len(), 1);
        let wav_norm = temp.join(format!("norm-{:?}.wav", max_controlled_bytes));
        let norm_err = media::normalize_loudness(
            &mka,
            &wav_norm,
            &opts,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_err();
        assert!(
            !norm_err.to_string().contains("WebM packet exceeds budget"),
            "unexpected packet budget error in normalize_loudness: {norm_err}"
        );

        let ln_plan = media::plan_loudnorm(&mka, None, false, &opts).unwrap();
        assert!(!ln_plan.steps.is_empty());
        let wav_ln = temp.join(format!("loudnorm-{:?}.wav", max_controlled_bytes));
        let ln_stats = media::apply_loudnorm(&mka, &wav_ln, None, &opts).unwrap();
        assert_eq!(ln_stats.sample_rate, 192_000);
        assert_eq!(ln_stats.sample_frames, 4096);

        let wav_mix = temp.join(format!("mix-{:?}.wav", max_controlled_bytes));
        let mix_stats = media::mix_audio_with_controls(
            &[mka.clone(), mka.clone()],
            &wav_mix,
            &MixAudioOptions::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(mix_stats.sample_frames, 1024);

        let wav_merge = temp.join(format!("merge-{:?}.wav", max_controlled_bytes));
        let merge_stats = media::merge_audio_with_controls(
            &[mka.clone(), mka.clone()],
            &wav_merge,
            &opts,
        )
        .unwrap();
        assert_eq!(merge_stats.channels, 2);
        assert_eq!(merge_stats.sample_frames, 1024);
    }

    let _ = std::fs::remove_dir_all(&temp);
}
