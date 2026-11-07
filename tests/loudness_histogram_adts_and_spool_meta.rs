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
        .join("loudness-histogram-adts-and-spool-meta")
}

fn temp_wav(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch586-{label}-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn container_and_adts_loudness_plans_charge_histogram_bins_and_negotiated_geometry() {
    let g711 = fixture_dir().join("g711-alaw-8k-meta.wav");
    // At 8000 Hz mono with 8000 frames:
    // base meter bytes without histogram bins = (8000*3 + 3200)*8 + 4096*4 + 4096*8 + 512 + 16384 = 283_648.
    // integrated_blocks = 1 + (8000 - 3200) / 800 = 7 -> +3584 bytes = 287_232.
    let without_bins = CopyOptions {
        max_controlled_bytes: Some(285_000),
        ..Default::default()
    };
    let plan_err = plan_loudness(&g711, &without_bins).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected histogram bin budget refusal in plan_loudness, got: {plan_err}"
    );
    let measure_err = measure_loudness(&g711, &without_bins).unwrap_err();
    assert!(
        measure_err.contains("controlled memory budget exceeded"),
        "expected histogram bin budget refusal in measure_loudness, got: {measure_err}"
    );

    // With max_packets = 1 (2048 frames < 3200 frames -> integrated_blocks = 0):
    // meter budget is 283_648 <= 315_000, and Stage-1/Stage-2 metadata working set is 312_848 <= 315_000.
    // Note that without max_packets = 1 (full 8000 frames), Stage-2 export budget is 372_288 > 315_000.
    let one_packet = CopyOptions {
        max_controlled_bytes: Some(315_000),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_loudness(&g711, &one_packet).unwrap();
    let stats = measure_loudness(&g711, &one_packet).unwrap();
    assert_eq!(stats.sample_frames, 2048);

    // Also verify ADTS SBR+PS negotiation in check_meter_budget:
    let lc_then_ps = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-loudness-prefix-sbr/lc-then-ps-24k.aac");
    // Un-negotiated header is 24000 Hz 1 ch (meter budget ~709_120 bytes),
    // whereas negotiated SBR+PS output is 48000 Hz 2 ch (meter budget ~1_401_856 bytes).
    let unnegotiated_only = CopyOptions {
        max_controlled_bytes: Some(1_100_000),
        ..Default::default()
    };
    let adts_plan_err = plan_loudness(&lc_then_ps, &unnegotiated_only).unwrap_err();
    assert!(
        adts_plan_err.contains("controlled memory budget exceeded"),
        "expected negotiated SBR+PS meter budget refusal in ADTS plan_loudness, got: {adts_plan_err}"
    );
    let adts_measure_err = measure_loudness(&lc_then_ps, &unnegotiated_only).unwrap_err();
    assert!(
        adts_measure_err.contains("controlled memory budget exceeded"),
        "expected negotiated SBR+PS meter budget refusal in ADTS measure_loudness, got: {adts_measure_err}"
    );
}

#[test]
fn adts_loudnorm_enforces_negotiated_geometry_and_actual_prefix_frames_in_spool_budget() {
    let adts = fixture_dir().join("adts-lc-48k-30pkt.aac");
    // 250 packets = 256000 frames at 48000 Hz mono:
    // ADTS mono decode admission (with SBR+PS discovery reserve) is 13_478_528 bytes.
    // Stage-2 dynamic loudnorm spool budget for 1024 frames is 6_776_320 bytes,
    // whereas for 256000 frames it is 59_114_496 bytes > 14_000_000 bytes!
    let tight = CopyOptions {
        max_controlled_bytes: Some(14_000_000),
        ..Default::default()
    };
    let plan_err = plan_loudnorm(&adts, None, false, &tight).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected ADTS loudnorm plan refusal for 256000 frames, got: {plan_err}"
    );
    let out = temp_wav("adts-loudnorm-tight");
    let _ = std::fs::remove_file(&out);
    let apply_err = apply_loudnorm(&adts, &out, None, &tight).unwrap_err();
    assert!(!out.exists());
    assert!(
        apply_err.contains("controlled memory budget exceeded"),
        "expected ADTS loudnorm apply refusal for 256000 frames, got: {apply_err}"
    );

    let bounded = CopyOptions {
        max_controlled_bytes: Some(14_000_000),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_loudnorm(&adts, None, false, &bounded).unwrap();
    let stats = apply_loudnorm(&adts, &out, None, &bounded).unwrap();
    let _ = std::fs::remove_file(&out);
    assert_eq!(stats.sample_frames, 4096);
}

#[test]
fn g711_and_matroska_decode_audio_plans_account_for_spool_metadata_bytes() {
    let g711 = fixture_dir().join("g711-alaw-8k-meta.wav");
    // With 1 packet (2048 frames) and ~4022 bytes of RIFF INFO metadata:
    // metadata working bytes = (4022 + 12) * 8 = 32_272 bytes;
    // Stage-2 float wave export budget is 2048*4 + 2048*8 + 2048*4 + 32_272 = 65_040 bytes.
    // Without stage2_meta_bytes in source_descriptor_for_decode, check_spool_wave_export_budget
    // saw metadata_bytes == 0 and skipped the entire Stage-2 estimate!
    let g711_tight = CopyOptions {
        max_controlled_bytes: Some(40_000),
        max_packets: Some(1),
        ..Default::default()
    };
    let g711_plan_err =
        plan_decode_audio(&g711, &AudioDecodeTransform::default(), &g711_tight).unwrap_err();
    assert!(
        g711_plan_err.contains("controlled memory budget exceeded"),
        "expected G.711 plan_decode_audio refusal for Stage-2 metadata, got: {g711_plan_err}"
    );
    let out_g711 = temp_wav("g711-meta");
    let _ = std::fs::remove_file(&out_g711);
    let g711_apply_err = decode_audio_transformed(
        &g711,
        &out_g711,
        AudioDecodeTransform::default(),
        &g711_tight,
    )
    .unwrap_err();
    assert!(!out_g711.exists());
    assert!(
        g711_apply_err.contains("controlled memory budget exceeded"),
        "expected G.711 decode_audio_transformed refusal for Stage-2 metadata, got: {g711_apply_err}"
    );

    let mka = fixture_dir().join("mka-pcm-tagged.mka");
    // Matroska PCM (1024 frames, float64 spool = 8 bytes/frame) + ~4232 bytes of INFO tags:
    // index_bytes + wave_meta_bytes in matroska_source_info is ~34_100 bytes,
    // while Stage-2 float64 wave export budget is 1024*8*3 + 33_952 = 58_528 bytes.
    let mka_tight = CopyOptions {
        max_controlled_bytes: Some(45_000),
        ..Default::default()
    };
    let mka_plan_err =
        plan_decode_audio(&mka, &AudioDecodeTransform::default(), &mka_tight).unwrap_err();
    assert!(
        mka_plan_err.contains("controlled memory budget exceeded"),
        "expected Matroska plan_decode_audio refusal for Stage-2 metadata, got: {mka_plan_err}"
    );
    let out_mka = temp_wav("mka-meta");
    let _ = std::fs::remove_file(&out_mka);
    let mka_apply_err = decode_audio_transformed(
        &mka,
        &out_mka,
        AudioDecodeTransform::default(),
        &mka_tight,
    )
    .unwrap_err();
    assert!(!out_mka.exists());
    assert!(
        mka_apply_err.contains("controlled memory budget exceeded"),
        "expected Matroska decode_audio_transformed refusal for Stage-2 metadata, got: {mka_apply_err}"
    );
}

#[test]
fn adts_mix_and_merge_audio_plans_enforce_output_buffer_budget() {
    let adts = fixture_dir().join("adts-lc-48k-30pkt.aac");
    // Each input has 256000 frames * 1 ch * 4 bytes = 1_024_000 bytes of decoded PCM.
    // Single-file ADTS mono decode admission is 13_478_528 bytes.
    // With 16 inputs + 1 output = 17 * 1_024_000 = 17_408_000 bytes > 14_000_000 bytes!
    let sources = vec![adts.clone(); 16];
    let full_tight = CopyOptions {
        max_controlled_bytes: Some(14_000_000),
        ..Default::default()
    };
    let mix_opts = MixAudioOptions::default();
    let mix_err = plan_mix_audio_with_controls(&sources, &mix_opts, &full_tight).unwrap_err();
    assert!(
        mix_err.contains("controlled memory budget exceeded by audio mix/merge output buffer"),
        "expected ADTS mix plan output buffer refusal, got: {mix_err}"
    );
    let merge_err = plan_merge_audio_with_controls(&sources, &full_tight).unwrap_err();
    assert!(
        merge_err.contains("merge-audio v1 requires exactly two inputs"),
        "unexpected merge error for >2 inputs: {merge_err}"
    );

    // With max_packets = 1 (1024 frames * 1 ch * 4 bytes = 4096 bytes per input):
    // mix working_bytes = 17 * 4096 = 69_632 <= 14_000_000 bytes.
    let bounded = CopyOptions {
        max_controlled_bytes: Some(14_000_000),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_mix_audio_with_controls(&sources, &mix_opts, &bounded).unwrap();
    plan_merge_audio_with_controls(&sources[..2], &bounded).unwrap();

    let out_mix = temp_wav("adts-mix");
    let _ = std::fs::remove_file(&out_mix);
    let mix_stats = mix_audio_with_controls(&sources[..2], &out_mix, &mix_opts, &bounded).unwrap();
    let _ = std::fs::remove_file(&out_mix);
    assert_eq!(mix_stats.sample_frames, 1024);

    let out_merge = temp_wav("adts-merge");
    let _ = std::fs::remove_file(&out_merge);
    let merge_stats = merge_audio_with_controls(&sources[..2], &out_merge, &bounded).unwrap();
    let _ = std::fs::remove_file(&out_merge);
    assert_eq!(merge_stats.sample_frames, 1024);
    assert_eq!(merge_stats.channels, 2);
}
