use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-interval-prefix-budget")
}

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "fvid-mka-interval-budget-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn matroska_aac_interval_prefix_bounds_packet_budget_and_in_band_ps_negotiation() {
    let mka = fixture_dir().join("matroska-aac-mono-then-ps.mka");
    let dir = temp_dir("aac");

    // Packet 0..=3 are 8 bytes (mono AAC pulse at 0, 43, 85, 128 ms);
    // Packet 4 is 31 bytes (SBR + in-band PS at 171 ms).
    // Full scan with max_packet_bytes = 16 must refuse packet 4.
    let tight_budget = fvid::media::CopyOptions {
        max_packet_bytes: 16,
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    let full_err = fvid::media::plan_decode_audio(
        &mka,
        &fvid::media::AudioDecodeTransform::default(),
        &tight_budget,
    )
    .expect_err("full Matroska scan must refuse packet 4 exceeding 16-byte budget");
    assert!(
        full_err.contains("WebM packet exceeds budget"),
        "unexpected error: {full_err}"
    );

    // Without interval or packet limit, in-band PS on packet 4 expands the track to 2 channels.
    let full_info = fvid::native_media::audio_source_info_selected(&mka, None)
        .expect("full Matroska audio_source_info");
    assert_eq!(full_info.channels, 2);

    // Interval [0, 40_000 us) covers only packet 0 (with delayed lookahead + resample radius
    // reaching at most packets 0..=2, well before packet 4 at 171 ms).
    let interval_transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 40_000)),
        ..Default::default()
    };

    // 1. Native source-info inspection with interval must succeed and report 1-channel mono
    let aac_info = fvid::native_media::aac_source_info_selected_with_options(
        &mka,
        None,
        Some((Duration::ZERO, Duration::from_micros(40_000))),
        &tight_budget,
    )
    .expect("aac_source_info within [0, 40ms) should succeed under 16-byte packet budget");
    assert_eq!(aac_info.sample_rate, 48_000);
    assert_eq!(aac_info.channels, 1);

    let audio_info = fvid::native_media::audio_source_info_selected_with_interval_and_options(
        &mka,
        None,
        Some((Duration::ZERO, Duration::from_micros(40_000))),
        &tight_budget,
    )
    .expect("audio_source_info within [0, 40ms) should succeed under 16-byte packet budget");
    assert_eq!(audio_info.sample_rate, 48_000);
    assert_eq!(audio_info.channels, 1);

    // 2. Top-level fvid::media::plan_decode_audio and decode_audio_transformed
    let plan = fvid::media::plan_decode_audio(&mka, &interval_transform, &tight_budget)
        .expect("interval-bounded plan_decode_audio must not reject packet 4 or negotiate PS");
    assert_eq!(plan.command, "decode-audio");

    let out_wav = dir.join("mka-interval-decode.wav");
    let stats_wav =
        fvid::media::decode_audio_transformed(&mka, &out_wav, interval_transform, &tight_budget)
            .expect("interval-bounded decode_audio must succeed with 16-byte budget");
    assert_eq!(stats_wav.channels, 1, "interval prefix must stay mono");

    let out_mka = dir.join("mka-interval-decode.mka");
    let native_budget = fvid::media::CopyOptions {
        max_packet_bytes: 16,
        ..Default::default()
    };
    let stats_mka =
        fvid::media::decode_audio_transformed(&mka, &out_mka, interval_transform, &native_budget)
            .expect("interval-bounded decode_audio to .mka must succeed with 16-byte budget");
    assert_eq!(stats_mka.channels, 1);

    // 3. Direct fvid_media::plan_decode_audio and decode_audio_transformed
    let fm_plan = fvid_media::plan_decode_audio(&mka, &interval_transform, &tight_budget)
        .expect("fvid_media::plan_decode_audio must bound Matroska packet budget and PS to interval prefix");
    assert_eq!(fm_plan.command, "decode-audio");
    assert!(
        !fm_plan.steps.iter().any(|s| s.action == "rematrix"),
        "fvid_media plan must negotiate 1 channel on interval prefix without rematrixing: {:?}",
        fm_plan.steps
    );
    let mono_target_transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 40_000)),
        channels: Some(1),
        ..Default::default()
    };
    let fm_mono_plan = fvid_media::plan_decode_audio(&mka, &mono_target_transform, &tight_budget)
        .expect("fvid_media::plan_decode_audio with channels=Some(1) must succeed on interval prefix");
    assert!(
        !fm_mono_plan.steps.iter().any(|s| s.action == "rematrix"),
        "channels=Some(1) on mono interval prefix must not add a rematrix step: {:?}",
        fm_mono_plan.steps
    );
    let fm_wav = dir.join("fm-mka-interval.wav");
    let fm_stats = fvid_media::decode_audio_transformed(
        &mka,
        &fm_wav,
        interval_transform,
        &tight_budget,
    )
    .expect("fvid_media::decode_audio_transformed must succeed on interval prefix");
    assert_eq!(fm_stats.channels, 1);

    // 4. plan_trim and trim
    let trim_plan = fvid::media::plan_trim(&mka, 0, 40_000, &native_budget)
        .expect("interval-bounded plan_trim must succeed on Matroska");
    assert_eq!(trim_plan.command, "trim");

    let trim_wav = dir.join("mka-interval-trim.wav");
    let trim_stats = fvid::media::trim(&mka, &trim_wav, 0, 40_000, &native_budget)
        .expect("interval-bounded trim must succeed on Matroska");
    assert_eq!(trim_stats.payload_bytes, 1920 * 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn matroska_pcm_interval_prefix_bounds_packet_budget_across_f32_and_f64_paths() {
    let mka = fixture_dir().join("matroska-pcm-small-then-large.mka");
    let dir = temp_dir("pcm");

    // Packets 0..=2 are 20 bytes (10 samples at 0, 10, 20 ms);
    // Packet 3 is 80 bytes (40 samples at 100 ms).
    let tight_budget = fvid::media::CopyOptions {
        max_packet_bytes: 32,
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };
    let native_budget = fvid::media::CopyOptions {
        max_packet_bytes: 32,
        ..Default::default()
    };
    let full_err = fvid::media::plan_decode_audio(
        &mka,
        &fvid::media::AudioDecodeTransform::default(),
        &tight_budget,
    )
    .expect_err("full Matroska PCM scan must refuse 80-byte packet 3 under 32-byte budget");
    assert!(
        full_err.contains("WebM packet exceeds budget"),
        "unexpected error: {full_err}"
    );

    // Interval [0, 20_000 us) covers packets 0..=1 (20 samples total).
    let interval_transform = fvid::media::AudioDecodeTransform {
        interval: Some((0, 20_000)),
        ..Default::default()
    };
    let plan = fvid::media::plan_decode_audio(&mka, &interval_transform, &tight_budget)
        .expect("interval-bounded Matroska PCM plan_decode_audio must succeed");
    assert_eq!(plan.command, "decode-audio");

    let out_wav = dir.join("pcm-interval-decode.wav");
    let stats_wav =
        fvid::media::decode_audio_transformed(&mka, &out_wav, interval_transform, &native_budget)
            .expect("interval-bounded Matroska PCM decode_audio must succeed");
    assert_eq!(stats_wav.channels, 1);
    assert_eq!(stats_wav.sample_frames, 20);

    fvid_media::plan_decode_audio(&mka, &interval_transform, &tight_budget)
        .expect("fvid_media::plan_decode_audio must bound Matroska PCM packet budget to interval prefix");
    let fm_wav = dir.join("fm-pcm-interval.wav");
    let fm_stats = fvid_media::decode_audio_transformed(
        &mka,
        &fm_wav,
        interval_transform,
        &tight_budget,
    )
    .expect("fvid_media::decode_audio_transformed must succeed on Matroska PCM interval prefix");
    assert_eq!(fm_stats.sample_frames, 20);

    let trim_wav = dir.join("pcm-interval-trim.wav");
    fvid::media::plan_trim(&mka, 0, 20_000, &native_budget)
        .expect("interval-bounded Matroska PCM plan_trim must succeed");
    let trim_stats = fvid::media::trim(&mka, &trim_wav, 0, 20_000, &native_budget)
        .expect("interval-bounded Matroska PCM trim must succeed");
    assert_eq!(trim_stats.payload_bytes, 20 * 4);
    let _ = std::fs::remove_dir_all(&dir);
}
