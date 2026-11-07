use fvid_control::CopyOptions;
use fvid_media::{
    apply_loudnorm, decode_audio_transformed, measure_loudness, merge_audio_with_controls,
    mix_audio_with_controls, plan_decode_audio, plan_loudness, plan_loudnorm,
    plan_merge_audio_with_controls, plan_mix_audio_with_controls, AudioDecodeTransform,
    MixAudioOptions,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("container-stage2-frames-and-spool-budget")
}

fn temp_wav(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch587-{label}-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn mp4_and_matroska_stage2_frames_enforce_decode_loudness_and_loudnorm_budgets() {
    for (name, label) in [
        ("mp4-sowt-8k-8pkt.m4a", "mp4"),
        ("mka-pcm-8k-8pkt.mka", "mka"),
    ] {
        let source = fixture_dir().join(name);
        // Both fixtures are 8000 Hz mono 16-bit PCM with 8 packets of 1000 frames = 8000 frames.
        // Stage-2 spool for PCM uses float64 (8 bytes/frame):
        // For 1024 frames, Stage-2 float64 wave export budget is 8192 + 24576 + 96 + 16384 = 49_248 bytes.
        // For 8000 frames, Stage-2 float64 wave export budget is 64000 + 192000 + 96 + 16384 = 272_480 bytes.
        // Stage-1 container decode admission is ~60_000 bytes.
        // With max_controlled_bytes = 120_000:
        // - Full 8000 frames must be refused by both plan_decode_audio and decode_audio_transformed!
        // - With max_packets = 1 (1000 frames -> clamped to 1024 frames = 49_248 bytes), both succeed!
        let decode_tight = CopyOptions {
            max_controlled_bytes: Some(120_000),
            ..Default::default()
        };
        let plan_err =
            plan_decode_audio(&source, &AudioDecodeTransform::default(), &decode_tight)
                .unwrap_err();
        assert!(
            plan_err.contains("controlled memory budget exceeded"),
            "expected {label} plan_decode_audio refusal for 8000 Stage-2 frames, got: {plan_err}"
        );
        let out = temp_wav(&format!("{label}-decode"));
        let _ = std::fs::remove_file(&out);
        let apply_err = decode_audio_transformed(
            &source,
            &out,
            AudioDecodeTransform::default(),
            &decode_tight,
        )
        .unwrap_err();
        assert!(!out.exists());
        assert!(
            apply_err.contains("controlled memory budget exceeded"),
            "expected {label} decode_audio_transformed refusal for 8000 Stage-2 frames, got: {apply_err}"
        );

        let decode_bounded = CopyOptions {
            max_controlled_bytes: Some(120_000),
            max_packets: Some(1),
            ..Default::default()
        };
        plan_decode_audio(&source, &AudioDecodeTransform::default(), &decode_bounded).unwrap();
        let stats = decode_audio_transformed(
            &source,
            &out,
            AudioDecodeTransform::default(),
            &decode_bounded,
        )
        .unwrap();
        let _ = std::fs::remove_file(&out);
        assert_eq!(stats.sample_frames, 1000);

        // Container loudness histogram bins at 8000 Hz mono:
        // Base meter budget without histogram bins (1024 frames < 3200) is 300_032 bytes.
        // With 8000 frames, integrated_blocks = 1 + (8000 - 3200) / 800 = 7 -> +3584 bytes = 303_616 bytes.
        let loudness_tight = CopyOptions {
            max_controlled_bytes: Some(301_500),
            ..Default::default()
        };
        let l_plan_err = plan_loudness(&source, &loudness_tight).unwrap_err();
        assert!(
            l_plan_err.contains("controlled memory budget exceeded"),
            "expected {label} plan_loudness refusal for histogram bins, got: {l_plan_err}"
        );
        let l_meas_err = measure_loudness(&source, &loudness_tight).unwrap_err();
        assert!(
            l_meas_err.contains("controlled memory budget exceeded"),
            "expected {label} measure_loudness refusal for histogram bins, got: {l_meas_err}"
        );

        let loudness_bounded = CopyOptions {
            max_controlled_bytes: Some(301_500),
            max_packets: Some(1),
            ..Default::default()
        };
        plan_loudness(&source, &loudness_bounded).unwrap();
        let l_stats = measure_loudness(&source, &loudness_bounded).unwrap();
        assert_eq!(l_stats.sample_frames, 1000);

        // Container loudnorm (dynamic 192 kHz resampling):
        // Fixed 192 kHz ring/scratch base is ~6.2 MiB;
        // for 1000 frames at 8000 Hz mono (24_000 resampled frames), check_spool_loudnorm_budget is 6_406_512 bytes.
        // For 8000 frames at 8000 Hz mono (192_000 resampled frames), check_spool_loudnorm_budget is 9_822_512 bytes.
        let loudnorm_tight = CopyOptions {
            max_controlled_bytes: Some(7_000_000),
            ..Default::default()
        };
        let ln_plan_err = plan_loudnorm(&source, None, false, &loudnorm_tight).unwrap_err();
        assert!(
            ln_plan_err.contains("controlled memory budget exceeded"),
            "expected {label} plan_loudnorm refusal for 8000 frames, got: {ln_plan_err}"
        );
        let out_ln = temp_wav(&format!("{label}-loudnorm"));
        let _ = std::fs::remove_file(&out_ln);
        let ln_apply_err = apply_loudnorm(&source, &out_ln, None, &loudnorm_tight).unwrap_err();
        assert!(!out_ln.exists());
        assert!(
            ln_apply_err.contains("controlled memory budget exceeded"),
            "expected {label} apply_loudnorm refusal for 8000 frames, got: {ln_apply_err}"
        );

        let loudnorm_bounded = CopyOptions {
            max_controlled_bytes: Some(7_000_000),
            max_packets: Some(1),
            ..Default::default()
        };
        plan_loudnorm(&source, None, false, &loudnorm_bounded).unwrap();
        let ln_stats = apply_loudnorm(&source, &out_ln, None, &loudnorm_bounded).unwrap();
        let _ = std::fs::remove_file(&out_ln);
        assert_eq!(ln_stats.sample_frames, 24_000);
    }
}

#[test]
fn mp4_and_matroska_mix_and_merge_plans_charge_decoded_input_and_output_buffers() {
    for (name, label) in [
        ("mp4-sowt-8k-8pkt.m4a", "mp4"),
        ("mka-pcm-8k-8pkt.mka", "mka"),
    ] {
        let source = fixture_dir().join(name);
        let mix_opts = MixAudioOptions::default();
        // Both fixtures decode to float64 spools (precision = "float64"), which mix/merge
        // execution refuses because read_float_wave_with_admission(Some(32)) requires float32.
        // Verify that plan_mix_audio_with_controls and plan_merge_audio_with_controls now
        // refuse float64-spooled container inputs consistently with execution:
        let pcm_pair = vec![source.clone(), source.clone()];
        let pcm_mix_err =
            plan_mix_audio_with_controls(&pcm_pair, &mix_opts, &CopyOptions::default())
                .unwrap_err();
        assert!(
            pcm_mix_err.contains("mix-audio requires float PCM inputs (flt)"),
            "expected {label} PCM mix plan refusal for float64 spool, got: {pcm_mix_err}"
        );
        let pcm_merge_err =
            plan_merge_audio_with_controls(&pcm_pair, &CopyOptions::default()).unwrap_err();
        assert!(
            pcm_merge_err.contains("mix-audio requires float PCM inputs (flt)"),
            "expected {label} PCM merge plan refusal for float64 spool, got: {pcm_merge_err}"
        );
    }

    // For float32-spooled MP4 (`ima4`) and Matroska (`A_MPEG/L3`) inputs, verify that
    // est_frames is populated from stage2_frames and enforces the mix/merge output buffer budget:
    for (name, label, input_count, full_budget, expected_one_pkt_frames) in [
        ("mp4-ima4-8k-125pkt.m4a", "mp4-ima4", 8usize, 200_000usize, 64u64),
        ("mka-mp3-44k-8pkt.mka", "mka-mp3", 12usize, 350_000usize, 1152u64),
    ] {
        let source = fixture_dir().join(name);
        let sources = vec![source.clone(); input_count];
        let tight = CopyOptions {
            max_controlled_bytes: Some(full_budget),
            ..Default::default()
        };
        let mix_opts = MixAudioOptions::default();
        let mix_err = plan_mix_audio_with_controls(&sources, &mix_opts, &tight).unwrap_err();
        assert!(
            mix_err.contains("controlled memory budget exceeded by audio mix/merge output buffer"),
            "expected {label} mix plan output buffer refusal, got: {mix_err}"
        );

        let bounded = CopyOptions {
            max_controlled_bytes: Some(full_budget),
            max_packets: Some(1),
            ..Default::default()
        };
        plan_mix_audio_with_controls(&sources, &mix_opts, &bounded).unwrap();
        plan_merge_audio_with_controls(&sources[..2], &bounded).unwrap();

        let out_mix = temp_wav(&format!("{label}-mix"));
        let _ = std::fs::remove_file(&out_mix);
        let mix_stats =
            mix_audio_with_controls(&sources[..2], &out_mix, &mix_opts, &bounded).unwrap();
        let _ = std::fs::remove_file(&out_mix);
        assert_eq!(mix_stats.sample_frames, expected_one_pkt_frames);

        let out_merge = temp_wav(&format!("{label}-merge"));
        let _ = std::fs::remove_file(&out_merge);
        let merge_stats = merge_audio_with_controls(&sources[..2], &out_merge, &bounded).unwrap();
        let _ = std::fs::remove_file(&out_merge);
        assert_eq!(merge_stats.sample_frames, expected_one_pkt_frames);
        assert_eq!(merge_stats.channels, 2);
    }
}

#[test]
fn identity_spool_export_and_g711_mix_plans_enforce_stage2_wave_budget_without_metadata() {
    let g711 = fixture_dir().join("g711-alaw-8k-nometa.wav");
    // g711-alaw-8k-nometa.wav has 20000 frames at 8000 Hz mono and metadata_bytes == 0.
    // Stage-1 G.711 decoder reserve is 1 * 2048 * 9 + 262_144 = 280_576 bytes.
    // Stage-2 float32 WAVE export budget for 20000 frames is:
    // input (80_000) + 3 * output (240_000) + 96 + 16_384 = 336_480 bytes.
    // With max_controlled_bytes = 300_000:
    // - Full 20000 frames must be refused by plan_decode_audio, decode_audio_transformed,
    //   plan_mix_audio_with_controls, and plan_merge_audio_with_controls!
    // - With max_packets = 1 (2048 frames -> Stage-2 = 49_248 bytes <= 300_000), all succeed!
    let tight = CopyOptions {
        max_controlled_bytes: Some(300_000),
        ..Default::default()
    };
    let plan_err =
        plan_decode_audio(&g711, &AudioDecodeTransform::default(), &tight).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected G.711 plan_decode_audio refusal for 20000 identity frames, got: {plan_err}"
    );
    let out = temp_wav("g711-nometa-decode");
    let _ = std::fs::remove_file(&out);
    let apply_err =
        decode_audio_transformed(&g711, &out, AudioDecodeTransform::default(), &tight).unwrap_err();
    assert!(!out.exists());
    assert!(
        apply_err.contains("controlled memory budget exceeded"),
        "expected G.711 decode_audio_transformed refusal for 20000 identity frames, got: {apply_err}"
    );

    let g711_pair = vec![g711.clone(), g711.clone()];
    let mix_opts = MixAudioOptions::default();
    let mix_err = plan_mix_audio_with_controls(&g711_pair, &mix_opts, &tight).unwrap_err();
    assert!(
        mix_err.contains("controlled memory budget exceeded"),
        "expected G.711 plan_mix_audio_with_controls refusal for Stage-2 spool budget, got: {mix_err}"
    );
    let merge_err = plan_merge_audio_with_controls(&g711_pair, &tight).unwrap_err();
    assert!(
        merge_err.contains("controlled memory budget exceeded"),
        "expected G.711 plan_merge_audio_with_controls refusal for Stage-2 spool budget, got: {merge_err}"
    );

    let bounded = CopyOptions {
        max_controlled_bytes: Some(300_000),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_decode_audio(&g711, &AudioDecodeTransform::default(), &bounded).unwrap();
    plan_mix_audio_with_controls(&g711_pair, &mix_opts, &bounded).unwrap();
    plan_merge_audio_with_controls(&g711_pair, &bounded).unwrap();

    let out_mix = temp_wav("g711-nometa-mix");
    let _ = std::fs::remove_file(&out_mix);
    let mix_stats =
        mix_audio_with_controls(&g711_pair, &out_mix, &mix_opts, &bounded).unwrap();
    let _ = std::fs::remove_file(&out_mix);
    assert_eq!(mix_stats.sample_frames, 2048);
}
