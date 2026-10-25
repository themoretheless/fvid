use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};
use fvid::media_info::NormalizeTarget;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-source-info-prefix-budget")
}

#[test]
fn mp4_and_matroska_mono_then_ps_aac_scope_geometry_to_max_packets_prefix() {
    let mp4 = fixture_dir().join("mono-then-ps.m4a");
    let mka = fixture_dir().join("mono-then-ps.mka");
    let temp = std::env::temp_dir().join(format!("fvid-batch269-containers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Without max_packets, packet 1 triggers in-band PS (stereo, 2 channels, 48 kHz).
    let full_mp4_info = fvid::native_media::audio_source_info_selected(&mp4, None).unwrap();
    assert_eq!(full_mp4_info.channels, 2);
    assert_eq!(full_mp4_info.sample_rate, 48_000);
    let full_mka_info = fvid::native_media::audio_source_info_selected(&mka, None).unwrap();
    assert_eq!(full_mka_info.channels, 2);
    assert_eq!(full_mka_info.sample_rate, 48_000);

    // With max_packets = Some(1) and max_packet_bytes = 16:
    // Packet 0 is 8 bytes (mono, no PS) and packet 1 is > 16 bytes (with PS).
    // All inspection, planning, loudness, loudnorm, mix, merge, and concat routes must
    // negotiate packet 0 as 1 channel (mono) and never read packet 1.
    for (label, src) in [("mp4", &mp4), ("mka", &mka)] {
        for max_controlled_bytes in [None, Some(64 * 1024 * 1024)] {
            let opts = CopyOptions {
                max_packet_bytes: 16,
                max_packets: Some(1),
                max_controlled_bytes,
                ..Default::default()
            };

            let plan =
                media::plan_decode_audio(src, &AudioDecodeTransform::default(), &opts).unwrap();
            assert_eq!(plan.streams.len(), 1);
            assert_eq!(plan.streams[0].codec, "aac");

            let wav = temp.join(format!("{label}-decode-{max_controlled_bytes:?}.wav"));
            let stats = media::decode_audio_transformed(
                src,
                &wav,
                AudioDecodeTransform::default(),
                &opts,
            )
            .unwrap();
            assert_eq!(stats.channels, 1, "{label} prefix decode should be mono");
            assert_eq!(stats.sample_rate, 48_000);

            // Rematrixing to 1 channel is a no-op when the 1-packet prefix is mono.
            let mono_xform = AudioDecodeTransform {
                channels: Some(1),
                ..Default::default()
            };
            let mono_plan = media::plan_decode_audio(src, &mono_xform, &opts).unwrap();
            assert!(
                !mono_plan.steps.iter().any(|s| s.action == "rematrix"),
                "{label} 1-packet mono prefix should not plan stereo->mono rematrix"
            );

            // Loudness plan and measurement must agree on 1-channel default weights.
            let loud_plan = media::plan_loudness(src, &opts).unwrap();
            assert!(
                loud_plan.steps.iter().any(|s| s.detail.contains("[1.0]")),
                "{label} loudness plan should use 1-channel weight [1.0], got: {:?}",
                loud_plan.steps
            );
            let loud = media::measure_loudness(src, &opts).unwrap();
            assert_eq!(loud.sample_frames, stats.sample_frames);

            // Normalize loudness plan and execution must not fail with channel-weight mismatch.
            let norm_plan =
                media::plan_normalize_loudness(src, &opts, None, NormalizeTarget::default())
                    .unwrap();
            assert_eq!(norm_plan.streams.len(), 1);
            let wav_norm = temp.join(format!("{label}-norm-{max_controlled_bytes:?}.wav"));
            let norm_err = media::normalize_loudness(
                src,
                &wav_norm,
                &opts,
                None,
                NormalizeTarget::default(),
            )
            .unwrap_err();
            assert!(
                !norm_err.contains("channel weights must match")
                    && !norm_err.contains("packet exceeds budget"),
                "unexpected {label} normalize_loudness error: {norm_err}"
            );

            // Loudnorm plan and execution
            let ln_plan = media::plan_loudnorm(src, None, false, &opts).unwrap();
            assert!(!ln_plan.steps.is_empty());
            let wav_ln = temp.join(format!("{label}-loudnorm-{max_controlled_bytes:?}.wav"));
            let ln_stats = media::apply_loudnorm(src, &wav_ln, None, &opts).unwrap();
            assert_eq!(ln_stats.sample_rate, 192_000);

            // Mix and merge must agree between preflight inspection and decoded spool (1 channel per input).
            let mix_plan = media::plan_mix_audio_with_controls(
                &[src.clone(), src.clone()],
                &MixAudioOptions::default(),
                &opts,
            )
            .unwrap();
            assert!(!mix_plan.steps.is_empty());
            let wav_mix = temp.join(format!("{label}-mix-{max_controlled_bytes:?}.wav"));
            let mix_stats = media::mix_audio_with_controls(
                &[src.clone(), src.clone()],
                &wav_mix,
                &MixAudioOptions::default(),
                &opts,
            )
            .unwrap();
            assert_eq!(mix_stats.channels, 1);

            let merge_plan =
                media::plan_merge_audio_with_controls(&[src.clone(), src.clone()], &opts).unwrap();
            assert!(
                merge_plan.steps.iter().any(|s| s.detail.contains("into 2 channels")),
                "{label} merge of two 1-channel prefixes must produce 2 channels, got: {:?}",
                merge_plan.steps
            );
            let wav_merge = temp.join(format!("{label}-merge-{max_controlled_bytes:?}.wav"));
            let merge_stats =
                media::merge_audio_with_controls(&[src.clone(), src.clone()], &wav_merge, &opts)
                    .unwrap();
            assert_eq!(merge_stats.channels, 2);

            // Trim plan and execution
            let trim_plan = media::plan_trim(src, 0, 20_000, &opts).unwrap();
            assert_eq!(trim_plan.command, "trim");
            let wav_trim = temp.join(format!("{label}-trim-{max_controlled_bytes:?}.wav"));
            let trim_stats = media::trim(src, &wav_trim, 0, 20_000, &opts).unwrap();
            assert_eq!(trim_stats.packets, 1);
        }
    }

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn adts_mono_then_ps_aac_scopes_negotiation_and_packet_budget_to_max_packets_prefix() {
    let adts = fixture_dir().join("mono-then-ps.aac");
    let temp = std::env::temp_dir().join(format!("fvid-batch269-adts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Without max_packets, packet 1 (payload > 16 bytes) has SBR + in-band PS -> 48 kHz, 2 channels.
    let full_info = fvid::native_media::audio_source_info_selected(&adts, None).unwrap();
    assert_eq!(full_info.channels, 2);
    assert_eq!(full_info.sample_rate, 48_000);

    // Without max_packets and max_packet_bytes = 16, packet 1 exceeds the packet byte budget.
    let full_tight = CopyOptions {
        max_packet_bytes: 16,
        max_packets: None,
        ..Default::default()
    };
    let err =
        media::plan_decode_audio(&adts, &AudioDecodeTransform::default(), &full_tight).unwrap_err();
    assert!(
        err.contains("ADTS packet exceeds budget"),
        "expected ADTS packet budget refusal without max_packets, got: {err}"
    );

    // With max_packets = Some(1) and max_packet_bytes = 16:
    // Only packet 0 (8-byte payload, 24 kHz mono LC) is in the prefix!
    for max_controlled_bytes in [None, Some(64 * 1024 * 1024)] {
        let opts = CopyOptions {
            max_packet_bytes: 16,
            max_packets: Some(1),
            max_controlled_bytes,
            ..Default::default()
        };

        let plan =
            media::plan_decode_audio(&adts, &AudioDecodeTransform::default(), &opts).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(plan.streams[0].codec, "aac");

        let wav = temp.join(format!("adts-decode-{max_controlled_bytes:?}.wav"));
        let stats = media::decode_audio_transformed(
            &adts,
            &wav,
            AudioDecodeTransform::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(stats.channels, 1);
        assert_eq!(stats.sample_rate, 24_000);
        assert_eq!(stats.sample_frames, 1024);

        // Loudness plan and measurement
        let loud_plan = media::plan_loudness(&adts, &opts).unwrap();
        assert!(
            loud_plan.steps.iter().any(|s| s.detail.contains("[1.0]")),
            "ADTS loudness plan should use 1-channel weight [1.0], got: {:?}",
            loud_plan.steps
        );
        let loud = media::measure_loudness(&adts, &opts).unwrap();
        assert_eq!(loud.sample_frames, 1024);

        // Normalize loudness plan and execution
        let norm_plan =
            media::plan_normalize_loudness(&adts, &opts, None, NormalizeTarget::default()).unwrap();
        assert_eq!(norm_plan.streams.len(), 1);
        let wav_norm = temp.join(format!("adts-norm-{max_controlled_bytes:?}.wav"));
        let norm_err = media::normalize_loudness(
            &adts,
            &wav_norm,
            &opts,
            None,
            NormalizeTarget::default(),
        )
        .unwrap_err();
        assert!(
            !norm_err.contains("channel weights must match")
                && !norm_err.contains("packet exceeds budget"),
            "unexpected ADTS normalize_loudness error: {norm_err}"
        );

        // Loudnorm plan and execution
        let ln_plan = media::plan_loudnorm(&adts, None, false, &opts).unwrap();
        assert!(!ln_plan.steps.is_empty());
        let wav_ln = temp.join(format!("adts-loudnorm-{max_controlled_bytes:?}.wav"));
        let ln_stats = media::apply_loudnorm(&adts, &wav_ln, None, &opts).unwrap();
        assert_eq!(ln_stats.sample_rate, 192_000);

        // Mix and merge
        let wav_mix = temp.join(format!("adts-mix-{max_controlled_bytes:?}.wav"));
        let mix_stats = media::mix_audio_with_controls(
            &[adts.clone(), adts.clone()],
            &wav_mix,
            &MixAudioOptions::default(),
            &opts,
        )
        .unwrap();
        assert_eq!(mix_stats.channels, 1);
        assert_eq!(mix_stats.sample_rate, 24_000);
        assert_eq!(mix_stats.sample_frames, 1024);

        let wav_merge = temp.join(format!("adts-merge-{max_controlled_bytes:?}.wav"));
        let merge_stats = media::merge_audio_with_controls(
            &[adts.clone(), adts.clone()],
            &wav_merge,
            &opts,
        )
        .unwrap();
        assert_eq!(merge_stats.channels, 2);
        assert_eq!(merge_stats.sample_rate, 24_000);
        assert_eq!(merge_stats.sample_frames, 1024);

        // Trim plan and execution
        let trim_plan = media::plan_trim(&adts, 0, 20_000, &opts).unwrap();
        assert_eq!(trim_plan.command, "trim");
        let wav_trim = temp.join(format!("adts-trim-{max_controlled_bytes:?}.wav"));
        let trim_stats = media::trim(&adts, &wav_trim, 0, 20_000, &opts).unwrap();
        assert_eq!(trim_stats.packets, 1);

        // Sample-domain concat with max_packets = Some(1) across two segments where segment 0's
        // packet 0 is 24 kHz mono (8 bytes) and packet 1 is oversized PS (> 16 bytes):
        let wav_concat = temp.join(format!("adts-concat-{max_controlled_bytes:?}.wav"));
        let concat_plan = media::plan_concat(&[adts.clone(), adts.clone()], &opts).unwrap();
        assert_eq!(concat_plan.command, "concat");
        let concat_stats = media::concat(&[adts.clone(), adts.clone()], &wav_concat, &opts).unwrap();
        assert_eq!(concat_stats.packets, 1);
    }

    let _ = std::fs::remove_dir_all(&temp);
}
