use fvid_control::CopyOptions;
use fvid_media::{decode_audio, decode_audio_transformed, plan_decode_audio, probe};
use fvid_media_info::AudioDecodeTransform;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("flac-ogg-info-cap-and-spool-budget")
}

fn temp_wav(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch584-{label}-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn flac_and_ogg_wave_export_preserve_canonical_info_tags_past_64_custom_entries() {
    let flac = fixture_dir().join("flac-66-tags.flac");
    let out_flac = temp_wav("flac-66");
    let _ = std::fs::remove_file(&out_flac);
    decode_audio(&flac, &out_flac, &CopyOptions::default()).unwrap();
    let flac_info = probe(&out_flac).unwrap();
    let _ = std::fs::remove_file(&out_flac);
    assert_eq!(
        flac_info.metadata.get("title").map(String::as_str),
        Some("Canonical FLAC Title")
    );
    assert_eq!(
        flac_info.metadata.get("language").map(String::as_str),
        Some("deu")
    );
    assert!(!flac_info.metadata.contains_key("T000"));
    assert!(!flac_info.metadata.contains_key("T001"));
    assert_eq!(
        flac_info.metadata.get("T002").map(String::as_str),
        Some("v2")
    );

    let opus = fixture_dir().join("ogg-opus-66-tags.ogg");
    let out_opus = temp_wav("opus-66");
    let _ = std::fs::remove_file(&out_opus);
    decode_audio(&opus, &out_opus, &CopyOptions::default()).unwrap();
    let opus_info = probe(&out_opus).unwrap();
    let _ = std::fs::remove_file(&out_opus);
    assert_eq!(
        opus_info.metadata.get("title").map(String::as_str),
        Some("Canonical Opus Title")
    );
    assert_eq!(
        opus_info.metadata.get("language").map(String::as_str),
        Some("fra")
    );
    assert!(!opus_info.metadata.contains_key("T000"));
    assert!(!opus_info.metadata.contains_key("T001"));
    assert_eq!(
        opus_info.metadata.get("T002").map(String::as_str),
        Some("v2")
    );
}

#[test]
fn ogg_opus_plan_and_export_enforce_actual_prefix_frames_in_spool_budget() {
    let opus = fixture_dir().join("ogg-opus-long-spool.ogg");
    let transform_up = AudioDecodeTransform {
        sample_rate: Some(384_000),
        channels: Some(8),
        ..Default::default()
    };
    let budget = 5 * 1024 * 1024;
    let tight = CopyOptions {
        max_controlled_bytes: Some(budget),
        ..Default::default()
    };
    let plan_err = plan_decode_audio(&opus, &transform_up, &tight).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "unexpected plan error: {plan_err}"
    );
    let out = temp_wav("opus-tight");
    let _ = std::fs::remove_file(&out);
    let apply_err = decode_audio_transformed(&opus, &out, transform_up, &tight).unwrap_err();
    assert!(!out.exists());
    assert!(
        apply_err.contains("controlled memory budget exceeded"),
        "unexpected apply error: {apply_err}"
    );

    let bounded = CopyOptions {
        max_controlled_bytes: Some(budget),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_decode_audio(&opus, &transform_up, &bounded).unwrap();
    let stats = decode_audio_transformed(&opus, &out, transform_up, &bounded).unwrap();
    let _ = std::fs::remove_file(&out);
    assert_eq!(stats.sample_frames, 7680);
}

#[test]
fn flac_and_mp3_apply_enforce_actual_prefix_frames_in_spool_budget() {
    let flac = fixture_dir().join("flac-long-spool.flac");
    let transform_up = AudioDecodeTransform {
        sample_rate: Some(384_000),
        channels: Some(8),
        ..Default::default()
    };
    let flac_budget = 33 * 1024 * 1024;
    let tight_flac = CopyOptions {
        max_controlled_bytes: Some(flac_budget),
        ..Default::default()
    };
    assert!(
        plan_decode_audio(&flac, &transform_up, &tight_flac)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    let out_flac = temp_wav("flac-tight");
    let _ = std::fs::remove_file(&out_flac);
    assert!(
        decode_audio_transformed(&flac, &out_flac, transform_up, &tight_flac)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    let bounded_flac = CopyOptions {
        max_controlled_bytes: Some(flac_budget),
        max_packets: Some(2),
        ..Default::default()
    };
    plan_decode_audio(&flac, &transform_up, &bounded_flac).unwrap();
    let flac_stats =
        decode_audio_transformed(&flac, &out_flac, transform_up, &bounded_flac).unwrap();
    let _ = std::fs::remove_file(&out_flac);
    assert_eq!(flac_stats.sample_frames, 3072);

    let mp3 = fixture_dir().join("mp3-long-spool.mp3");
    let mp3_budget = 2 * 1024 * 1024;
    let tight_mp3 = CopyOptions {
        max_controlled_bytes: Some(mp3_budget),
        ..Default::default()
    };
    assert!(
        plan_decode_audio(&mp3, &transform_up, &tight_mp3)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    let out_mp3 = temp_wav("mp3-tight");
    let _ = std::fs::remove_file(&out_mp3);
    assert!(
        decode_audio_transformed(&mp3, &out_mp3, transform_up, &tight_mp3)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    let bounded_mp3 = CopyOptions {
        max_controlled_bytes: Some(mp3_budget),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_decode_audio(&mp3, &transform_up, &bounded_mp3).unwrap();
    let mp3_stats = decode_audio_transformed(&mp3, &out_mp3, transform_up, &bounded_mp3).unwrap();
    let _ = std::fs::remove_file(&out_mp3);
    assert!(mp3_stats.sample_frames > 0);
}

#[test]
fn adts_plan_and_apply_enforce_actual_prefix_frames_in_spool_budget() {
    let adts = fixture_dir().join("adts-long-spool.aac");
    let transform = AudioDecodeTransform {
        sample_rate: Some(384_000),
        channels: Some(8),
        ..Default::default()
    };
    let budget = 20 * 1024 * 1024;
    let tight = CopyOptions {
        max_controlled_bytes: Some(budget),
        ..Default::default()
    };
    assert!(
        plan_decode_audio(&adts, &transform, &tight)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    let out = temp_wav("adts-tight");
    let _ = std::fs::remove_file(&out);
    assert!(
        decode_audio_transformed(&adts, &out, transform, &tight)
            .unwrap_err()
            .contains("controlled memory budget exceeded")
    );
    assert!(!out.exists());

    let bounded = CopyOptions {
        max_controlled_bytes: Some(budget),
        max_packets: Some(1),
        ..Default::default()
    };
    plan_decode_audio(&adts, &transform, &bounded).unwrap();
    let stats = decode_audio_transformed(&adts, &out, transform, &bounded).unwrap();
    let _ = std::fs::remove_file(&out);
    assert!(stats.sample_frames > 0);
}
