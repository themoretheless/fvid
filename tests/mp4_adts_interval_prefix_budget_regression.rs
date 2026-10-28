use std::path::PathBuf;
use std::time::Duration;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("mp4-adts-interval-prefix-budget")
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch272-{}-{}-{}",
        label,
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn mp4_interval_prefix_bounds_packet_budget_and_in_band_ps_negotiation() {
    let mp4 = fixture_dir().join("mp4-mono-then-ps.m4a");
    let temp = temp_dir("mp4");

    // Packet 0 is 8 bytes (mono, 2048 frames at 48 kHz = 42_666.67 us).
    // Packet 1 is 35 bytes (stereo in-band PS, starts at pts = 2048 -> 42_666.67 us).
    // With interval [0, 20_000) us (960 frames at 48 kHz) and max_packet_bytes = 24,
    // only Packet 0 is within the active decode prefix.
    let options = fvid::media::CopyOptions {
        max_packet_bytes: 24,
        ..Default::default()
    };
    let transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 20_000)),
        ..Default::default()
    };

    // 1. Native source-info inspection with interval must succeed and report 1-channel mono
    let aac_info = fvid::native_media::aac_source_info_selected_with_options(
        &mp4,
        None,
        Some((Duration::ZERO, Duration::from_micros(20_000))),
        &options,
    )
    .expect("aac_source_info within [0, 20ms) should succeed under 24-byte packet budget");
    assert_eq!(aac_info.sample_rate, 48_000);
    assert_eq!(aac_info.channels, 1);

    let audio_info = fvid::native_media::audio_source_info_selected_with_interval_and_options(
        &mp4,
        None,
        Some((Duration::ZERO, Duration::from_micros(20_000))),
        &options,
    )
    .expect("audio_source_info within [0, 20ms) should succeed under 24-byte packet budget");
    assert_eq!(audio_info.sample_rate, 48_000);
    assert_eq!(audio_info.channels, 1);

    // 2. Top-level and fvid_media plan_decode_audio and decode_audio_transformed must succeed and output 1 channel (960 frames)
    let plan = fvid::media::plan_decode_audio(&mp4, &transform, &options)
        .expect("fvid::media::plan_decode_audio should succeed on [0, 20ms)");
    assert_eq!(plan.command, "decode-audio");

    let fvid_media_plan = fvid_media::plan_decode_audio(&mp4, &transform, &options)
        .expect("fvid_media::plan_decode_audio should succeed on [0, 20ms)");
    assert_eq!(fvid_media_plan.command, "decode-audio");

    let wav_native = temp.join("mp4-native.wav");
    let stats_native = fvid::media::decode_audio_transformed(&mp4, &wav_native, transform, &options)
        .expect("fvid::media::decode_audio_transformed should succeed on [0, 20ms)");
    assert_eq!(stats_native.sample_rate, 48_000);
    assert_eq!(stats_native.channels, 1);
    assert_eq!(stats_native.sample_frames, 960);
    assert_eq!(stats_native.decoded_frames, 1);

    let wav_owned = temp.join("mp4-owned.wav");
    let options_controlled = fvid::media::CopyOptions {
        max_packet_bytes: 24,
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let stats_owned =
        fvid_media::decode_audio_transformed(&mp4, &wav_owned, transform, &options_controlled)
            .expect("fvid_media::decode_audio_transformed should succeed on [0, 20ms)");
    assert_eq!(stats_owned.sample_rate, 48_000);
    assert_eq!(stats_owned.channels, 1);
    assert_eq!(stats_owned.sample_frames, 960);
    assert_eq!(stats_owned.decoded_frames, 1);

    // 3. Top-level plan_trim and trim on [0, 20_000) us must also succeed under 24-byte packet budget
    let trim_plan = fvid::media::plan_trim(&mp4, 0, 20_000, &options)
        .expect("fvid::media::plan_trim should succeed on [0, 20ms)");
    assert_eq!(trim_plan.command, "trim");

    let wav_trim = temp.join("mp4-trim.wav");
    let trim_stats = fvid::media::trim(&mp4, &wav_trim, 0, 20_000, &options)
        .expect("fvid::media::trim should succeed on [0, 20ms)");
    assert_eq!(trim_stats.packets, 1);
    assert_eq!(trim_stats.payload_bytes, 960 * 4);

    // 4. Extending the interval to [0, 50_000) us reaches Packet 1 (35 bytes) and must be rejected under 24-byte packet budget
    let wide_transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 50_000)),
        ..Default::default()
    };
    let err = fvid::media::plan_decode_audio(&mp4, &wide_transform, &options)
        .expect_err("interval reaching packet 1 must fail 24-byte packet budget");
    assert!(
        err.contains("packet exceeds") || err.contains("exceeds packet size limit"),
        "unexpected error: {err}"
    );
    let err_owned = fvid_media::plan_decode_audio(&mp4, &wide_transform, &options_controlled)
        .expect_err("fvid_media plan reaching packet 1 must fail 24-byte packet budget");
    assert!(
        err_owned.contains("packet exceeds") || err_owned.contains("exceeds packet size limit"),
        "unexpected error: {err_owned}"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn adts_interval_prefix_bounds_plan_decode_and_trim_packet_budget() {
    let adts = fixture_dir().join("adts-small-then-large.aac");
    let temp = temp_dir("adts");

    // Frame 0 is 8 bytes payload / 15 bytes ADTS (1024 frames at 48 kHz = 21_333.33 us).
    // Frame 1 is 35 bytes payload / 42 bytes ADTS (starts at 1024 frames = 21_333.33 us).
    // With interval [0, 15_000) us (720 frames at 48 kHz) and max_packet_bytes = 24,
    // only Frame 0 is within the active decode prefix.
    let options = fvid::media::CopyOptions {
        max_packet_bytes: 24,
        ..Default::default()
    };
    let transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 15_000)),
        ..Default::default()
    };

    let plan = fvid::media::plan_decode_audio(&adts, &transform, &options)
        .expect("fvid::media::plan_decode_audio on ADTS [0, 15ms) should succeed");
    assert_eq!(plan.command, "decode-audio");

    let trim_plan = fvid::media::plan_trim(&adts, 0, 15_000, &options)
        .expect("fvid::media::plan_trim on ADTS [0, 15ms) should succeed");
    assert_eq!(trim_plan.command, "trim");
    assert!(
        trim_plan
            .steps
            .iter()
            .any(|s| s.detail.contains("720 float32 PCM sample frames")),
        "unexpected steps: {:?}",
        trim_plan.steps
    );

    let wav_trim = temp.join("adts-trim.wav");
    let trim_stats = fvid::media::trim(&adts, &wav_trim, 0, 15_000, &options)
        .expect("fvid::media::trim on ADTS [0, 15ms) should succeed");
    assert_eq!(trim_stats.packets, 1);
    assert_eq!(trim_stats.payload_bytes, 720 * 4);

    // Extending interval to [0, 30_000) us reaches Frame 1 (42 bytes) and must be rejected
    let err = fvid::media::plan_trim(&adts, 0, 30_000, &options)
        .expect_err("ADTS trim reaching frame 1 must fail 24-byte packet budget");
    assert!(
        err.contains("packet exceeds") || err.contains("exceeds packet size limit"),
        "unexpected error: {err}"
    );
    let _ = std::fs::remove_dir_all(&temp);
}
