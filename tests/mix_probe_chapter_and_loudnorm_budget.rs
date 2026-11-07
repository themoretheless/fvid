use fvid_control::CopyOptions;
use fvid_media::{
    apply_loudnorm, merge_audio_with_controls, mix_audio_with_controls,
    owned_audio_plan::standalone_packet_work, owned_probe::probe_with_options, plan_loudnorm,
    plan_merge_audio_with_controls, plan_mix_audio_with_controls, MixAudioOptions,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("mix-probe-chapter-and-loudnorm-budget")
}

fn temp_wav(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch585-{label}-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn ogg_audio_mix_and_merge_plans_bound_estimated_frames_by_max_packets() {
    let opus = fixture_dir().join("ogg-opus-mix-prefix.ogg");
    let sources = vec![opus.clone(); 8];
    let full_tight = CopyOptions {
        max_controlled_bytes: Some(5_000_000),
        ..Default::default()
    };
    let mix_opts = MixAudioOptions::default();
    let full_err = plan_mix_audio_with_controls(&sources, &mix_opts, &full_tight).unwrap_err();
    assert!(
        full_err.contains("controlled memory budget exceeded by audio mix/merge output buffer"),
        "unexpected full mix plan error: {full_err}"
    );

    let bounded = CopyOptions {
        max_controlled_bytes: Some(5_000_000),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_mix_audio_with_controls(&sources, &mix_opts, &bounded).unwrap();
    let pair = vec![opus.clone(), opus.clone()];
    let pair_full = CopyOptions {
        max_controlled_bytes: Some(5_000_000),
        ..Default::default()
    };
    let merge_err = plan_merge_audio_with_controls(&pair, &pair_full).unwrap_err();
    assert!(
        merge_err.contains("controlled memory budget exceeded by audio mix/merge output buffer"),
        "unexpected full merge plan error: {merge_err}"
    );
    plan_merge_audio_with_controls(&pair, &bounded).unwrap();

    let out_mix = temp_wav("mix-bounded");
    let _ = std::fs::remove_file(&out_mix);
    let mix_stats = mix_audio_with_controls(&sources, &out_mix, &mix_opts, &bounded).unwrap();
    let _ = std::fs::remove_file(&out_mix);
    assert_eq!(mix_stats.sample_frames, 960);

    let out_merge = temp_wav("merge-bounded");
    let _ = std::fs::remove_file(&out_merge);
    let merge_stats = merge_audio_with_controls(&pair, &out_merge, &bounded).unwrap();
    let _ = std::fs::remove_file(&out_merge);
    assert_eq!(merge_stats.sample_frames, 960);
    assert_eq!(merge_stats.channels, 4);
}

#[test]
fn ogg_and_mpeg_probe_with_options_enforce_audio_packet_work_limits() {
    let opus = fixture_dir().join("ogg-opus-mix-prefix.ogg");
    assert_eq!(
        standalone_packet_work(&opus, &CopyOptions::default()).unwrap(),
        Some(150)
    );
    let exact_ogg = CopyOptions {
        max_packets: Some(150),
        ..Default::default()
    };
    let ogg_info = probe_with_options(&opus, &exact_ogg).unwrap();
    assert_eq!(ogg_info.streams[0].codec, "opus");

    let short_ogg = CopyOptions {
        max_packets: Some(149),
        ..Default::default()
    };
    let ogg_err = probe_with_options(&opus, &short_ogg).unwrap_err();
    assert!(
        ogg_err.contains("Ogg packet count exceeds packet limit"),
        "unexpected Ogg probe error: {ogg_err}"
    );

    let mp3 = fixture_dir().join("mp3-info-probe-prefix.mp3");
    assert_eq!(
        standalone_packet_work(&mp3, &CopyOptions::default()).unwrap(),
        Some(4)
    );
    let exact_mp3 = CopyOptions {
        max_packets: Some(4),
        ..Default::default()
    };
    let mp3_info = probe_with_options(&mp3, &exact_mp3).unwrap();
    assert_eq!(mp3_info.streams[0].codec, "mp3");

    let short_mp3 = CopyOptions {
        max_packets: Some(3),
        ..Default::default()
    };
    let mp3_err = probe_with_options(&mp3, &short_mp3).unwrap_err();
    assert!(
        mp3_err.contains("MPEG packet count exceeds packet limit"),
        "unexpected MPEG probe error: {mp3_err}"
    );
}

#[test]
fn matroska_frontend_probe_omits_empty_title_for_untitled_chapters() {
    let mka = fixture_dir().join("mka-untitled-chapter.mka");
    let backend = probe_with_options(&mka, &CopyOptions::default()).unwrap();
    let frontend = fvid::native_probe::probe(&mka).unwrap();
    assert_eq!(backend.chapters.len(), 2);
    assert_eq!(frontend.chapters.len(), 2);
    assert!(backend.chapters[0].metadata.is_empty());
    assert!(
        frontend.chapters[0].metadata.is_empty(),
        "expected empty metadata for untitled chapter, got: {:?}",
        frontend.chapters[0].metadata
    );
    assert_eq!(
        frontend.chapters[1].metadata.get("title").map(String::as_str),
        Some("Act II")
    );
}

#[test]
fn container_loudnorm_plan_and_apply_enforce_actual_prefix_frames_in_spool_budget() {
    let opus = fixture_dir().join("ogg-opus-mix-prefix.ogg");
    let budget = 7 * 1024 * 1024;
    let tight = CopyOptions {
        max_controlled_bytes: Some(budget),
        ..Default::default()
    };
    let plan_err = plan_loudnorm(&opus, None, false, &tight).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "unexpected loudnorm plan error: {plan_err}"
    );
    let out = temp_wav("loudnorm-tight");
    let _ = std::fs::remove_file(&out);
    let apply_err = apply_loudnorm(&opus, &out, None, &tight).unwrap_err();
    assert!(!out.exists());
    assert!(
        apply_err.contains("controlled memory budget exceeded"),
        "unexpected loudnorm apply error: {apply_err}"
    );

    let bounded = CopyOptions {
        max_controlled_bytes: Some(budget),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_loudnorm(&opus, None, false, &bounded).unwrap();
    let stats = apply_loudnorm(&opus, &out, None, &bounded).unwrap();
    let _ = std::fs::remove_file(&out);
    assert_eq!(stats.sample_frames, 3840);
}
