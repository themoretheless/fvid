use fvid::media::{
    apply_loudnorm, apply_loudnorm_dual, decode_audio_transformed, measure_loudness,
    plan_decode_audio, plan_loudness, plan_loudnorm, probe, trim, AudioDecodeTransform,
    CopyOptions,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-audio-selection-budget")
        .join(name)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-ogg-sel-budget-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn single_audio_ogg_with_preceding_skeleton_stream_reports_actual_source_index() {
    let source = fixture("skeleton-plus-opus.ogg");
    let info = probe(&source).expect("probe skeleton+opus ogg");
    assert_eq!(info.streams.len(), 1);
    assert_eq!(info.streams[0].index, 1);
    assert_eq!(info.streams[0].codec, "opus");

    let default_options = CopyOptions::default();
    let explicit_options = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };

    let default_decode_plan =
        plan_decode_audio(&source, &AudioDecodeTransform::default(), &default_options)
            .expect("default decode plan");
    let explicit_decode_plan =
        plan_decode_audio(&source, &AudioDecodeTransform::default(), &explicit_options)
            .expect("explicit decode plan");
    assert_eq!(default_decode_plan.streams.len(), 1);
    assert_eq!(default_decode_plan.streams[0].index, 1);
    assert_eq!(default_decode_plan.streams, explicit_decode_plan.streams);

    let default_loudness_plan =
        plan_loudness(&source, &default_options).expect("default loudness plan");
    assert_eq!(default_loudness_plan.streams[0].index, 1);

    let default_loudnorm_plan =
        plan_loudnorm(&source, None, false, &default_options).expect("default loudnorm plan");
    assert_eq!(default_loudnorm_plan.streams[0].index, 1);
}

#[test]
fn discrete_family255_ogg_opus_supports_rematrix_loudness_and_loudnorm() {
    let source = fixture("discrete-family255-stereo.opus");
    let temp = TempDir::new("discrete-family255");
    let out_mono = temp.0.join("mono.wav");

    let transform = AudioDecodeTransform {
        channels: Some(1),
        ..Default::default()
    };
    let options = CopyOptions::default();

    let plan = plan_decode_audio(&source, &transform, &options)
        .expect("plan_decode_audio for discrete stereo->mono");
    assert!(plan.steps.iter().any(|s| s.action == "rematrix"));

    let stats = decode_audio_transformed(&source, &out_mono, transform, &options)
        .expect("decode_audio_transformed for discrete stereo->mono");
    assert_eq!(stats.channels, 1);
    assert_eq!(stats.sample_frames, 1920);

    // Both fvid::media and fvid_media loudness/loudnorm must qualify <=2ch discrete Ogg Opus.
    let loud_plan = plan_loudness(&source, &options).expect("plan_loudness");
    assert_eq!(loud_plan.command, "loudness");
    let measured = measure_loudness(&source, &options).expect("measure_loudness");
    assert_eq!(measured.sample_frames, 1920);
    let owned_measured =
        fvid_media::measure_loudness(&source, &options).expect("fvid_media::measure_loudness");
    assert_eq!(owned_measured.sample_frames, 1920);

    let norm_plan = plan_loudnorm(&source, None, false, &options).expect("plan_loudnorm");
    assert_eq!(norm_plan.command, "loudnorm");
    let out_norm = temp.0.join("norm.wav");
    let norm_stats =
        apply_loudnorm(&source, &out_norm, None, &options).expect("apply_loudnorm on discrete opus");
    assert_eq!(norm_stats.sample_rate, 192_000);
    assert_eq!(norm_stats.channels, 2);

    let out_norm_dual = temp.0.join("norm-dual.wav");
    let dual_stats = apply_loudnorm_dual(&source, &out_norm_dual, None, &options)
        .expect("apply_loudnorm_dual on discrete opus");
    assert_eq!(dual_stats.sample_rate, 192_000);
    assert_eq!(dual_stats.channels, 2);
}

#[test]
fn ogg_packet_byte_budget_bounds_active_prefix_while_rejecting_oversized_prefix_packets() {
    let source = fixture("prefix-budget.opus");
    let temp = TempDir::new("prefix-budget");

    // Packet 0 is 12 bytes (0..20ms = 960 samples); Packet 1 is 120 bytes (20..40ms = 960 samples).
    // With max_packet_bytes = 32 and max_packets = Some(1), only Packet 0 is in the active prefix.
    let prefix_count_options = CopyOptions {
        max_packet_bytes: 32,
        max_packets: Some(1),
        ..Default::default()
    };

    plan_decode_audio(
        &source,
        &AudioDecodeTransform::default(),
        &prefix_count_options,
    )
    .expect("plan_decode_audio should admit 1-packet prefix within 32-byte limit");

    let out_prefix_count = temp.0.join("prefix-count.wav");
    let count_stats = decode_audio_transformed(
        &source,
        &out_prefix_count,
        AudioDecodeTransform::default(),
        &prefix_count_options,
    )
    .expect("decode_audio_transformed should succeed on 1-packet prefix");
    assert_eq!(count_stats.decoded_frames, 1);
    assert_eq!(count_stats.sample_frames, 960);

    // With interval = (0, 20_000) us (first 20 ms = 960 samples) and no resampling,
    // only Packet 0 is needed; Packet 1 starts at 960 samples and is not decoded.
    let interval_transform = AudioDecodeTransform {
        interval: Some((0, 20_000)),
        ..Default::default()
    };
    let interval_options = CopyOptions {
        max_packet_bytes: 32,
        ..Default::default()
    };

    plan_decode_audio(&source, &interval_transform, &interval_options)
        .expect("plan_decode_audio should admit 20ms interval prefix within 32-byte limit");

    let out_interval = temp.0.join("prefix-interval.wav");
    let interval_stats =
        decode_audio_transformed(&source, &out_interval, interval_transform, &interval_options)
            .expect("decode_audio_transformed should succeed on 20ms interval prefix");
    assert_eq!(interval_stats.decoded_frames, 1);
    assert_eq!(interval_stats.sample_frames, 960);

    let out_trim = temp.0.join("prefix-trim.wav");
    let trim_stats = trim(&source, &out_trim, 0, 20_000, &interval_options)
        .expect("trim to .wav should succeed on 20ms interval prefix");
    assert_eq!(trim_stats.packets, 1);

    // Full-stream planning and decoding without prefix restriction must still reject Packet 1 (120 > 32).
    let full_err = plan_decode_audio(&source, &AudioDecodeTransform::default(), &interval_options)
        .expect_err("full-stream plan must reject oversized second packet");
    assert!(
        full_err.contains("exceeds byte limit"),
        "unexpected error: {full_err}"
    );

    let out_full = temp.0.join("full.wav");
    let decode_err = decode_audio_transformed(
        &source,
        &out_full,
        AudioDecodeTransform::default(),
        &interval_options,
    )
    .expect_err("full-stream decode must reject oversized second packet");
    assert!(
        decode_err.contains("exceeds byte limit"),
        "unexpected error: {decode_err}"
    );
    assert!(!out_full.exists());
}
