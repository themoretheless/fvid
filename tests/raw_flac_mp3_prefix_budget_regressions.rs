//! Regressions for bare FLAC and bare MPEG Layer I/II/III prefix-bounded `max_packet_bytes`
//! validation across `plan_decode_audio`, `plan_trim`, `decode_audio_transformed`, `trim`,
//! `decode_raw_flac_pcm`, and `decode_raw_mpeg_pcm` (FVID-0266).

use fvid::media::{self, AudioDecodeTransform, CopyOptions};
use std::path::PathBuf;
use std::time::Duration;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/raw-flac-mp3-prefix-budget")
        .join(name)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-raw-flac-mp3-budget-{label}-{}-{}",
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
fn raw_flac_packet_byte_budget_scopes_to_active_decode_prefix_and_rejects_full_stream_oversized_packet()
 {
    let source = fixture("prefix-budget.flac");
    // Frame 0 is 11 bytes (192 samples = 4_000 us at 48 kHz); Frame 1 is 393 bytes.
    let mut prefix_opts = CopyOptions::default();
    prefix_opts.max_packet_bytes = 32;
    prefix_opts.max_packets = Some(1);

    let full_transform = AudioDecodeTransform::default();
    let plan = media::plan_decode_audio(&source, &full_transform, &prefix_opts)
        .expect("plan_decode_audio on 1-packet raw FLAC prefix within 32-byte packet budget must succeed");
    assert_eq!(plan.streams[0].codec, "flac");

    let mut raw_pcm = Vec::new();
    let raw_stats = fvid_media::owned_raw_flac::decode_raw_flac_pcm(
        &source,
        &mut raw_pcm,
        None,
        &prefix_opts,
    )
    .expect("decode_raw_flac_pcm with max_packets=1 must succeed when frame 0 is 11 bytes <= 32");
    assert_eq!(raw_stats.decoded_frames, 1);
    assert_eq!(raw_stats.sample_frames, 192);

    let tmp = TempDir::new("flac");
    let out_prefix = tmp.0.join("flac-prefix-max-packets.wav");
    let stats = media::decode_audio_transformed(&source, &out_prefix, full_transform, &prefix_opts)
        .expect("decode_audio_transformed on 1-packet raw FLAC prefix must succeed");
    assert_eq!(stats.sample_frames, 192);

    // Interval-bounded prefix without max_packets: [0, 4_000) us covers only Frame 0 (pts 0..192).
    let mut interval_opts = CopyOptions::default();
    interval_opts.max_packet_bytes = 32;
    let interval_transform = AudioDecodeTransform {
        interval: Some((0, 4_000)),
        ..Default::default()
    };
    media::plan_decode_audio(&source, &interval_transform, &interval_opts)
        .expect("plan_decode_audio on 4ms raw FLAC interval must succeed when frame 0 is 11 bytes <= 32");
    let out_interval = tmp.0.join("flac-prefix-interval.wav");
    let interval_stats = media::decode_audio_transformed(
        &source,
        &out_interval,
        interval_transform,
        &interval_opts,
    )
    .expect("decode_audio_transformed on 4ms raw FLAC interval must succeed");
    assert_eq!(interval_stats.sample_frames, 192);

    media::plan_trim(&source, 0, 4_000, &interval_opts)
        .expect("plan_trim on 4ms raw FLAC interval must succeed");
    let out_trim = tmp.0.join("flac-trim.wav");
    media::trim(&source, &out_trim, 0, 4_000, &interval_opts)
        .expect("trim on 4ms raw FLAC interval must succeed");

    // Full decode without prefix bounds must still reject Frame 1 (393 bytes > 32).
    let plan_err = media::plan_decode_audio(&source, &full_transform, &interval_opts)
        .expect_err("full raw FLAC plan must reject 393-byte frame 1 when max_packet_bytes=32");
    assert!(
        plan_err.to_string().contains("FLAC packet bytes exceed limit"),
        "unexpected error: {plan_err}"
    );
    let out_full = tmp.0.join("flac-full.wav");
    let exec_err = media::decode_audio_transformed(&source, &out_full, full_transform, &interval_opts)
        .expect_err("full raw FLAC decode must reject 393-byte frame 1 when max_packet_bytes=32");
    assert!(
        exec_err.to_string().contains("FLAC packet bytes exceed limit"),
        "unexpected error: {exec_err}"
    );
    assert!(!out_full.exists());
}

#[test]
fn raw_mp3_packet_byte_budget_scopes_to_active_decode_prefix_and_rejects_full_stream_oversized_packet()
 {
    let source = fixture("prefix-budget.mp3");
    // Frame 0 is 960 bytes (1152 samples = 24_000 us at 48 kHz); Frame 1 is 961 bytes (padded).
    let mut prefix_opts = CopyOptions::default();
    prefix_opts.max_packet_bytes = 960;
    prefix_opts.max_packets = Some(1);

    let full_transform = AudioDecodeTransform::default();
    let plan = media::plan_decode_audio(&source, &full_transform, &prefix_opts)
        .expect("plan_decode_audio on 1-packet raw MP3 prefix within 960-byte packet budget must succeed");
    assert_eq!(plan.streams[0].codec, "mp3");

    let mut raw_pcm = Vec::new();
    let raw_stats = fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
        &source,
        &mut raw_pcm,
        None,
        &prefix_opts,
    )
    .expect("decode_raw_mp3_pcm with max_packets=1 must succeed when frame 0 is 960 bytes");
    assert_eq!(raw_stats.decoded_frames, 1);
    assert_eq!(raw_stats.sample_frames, 1152);

    let tmp = TempDir::new("mp3");
    let out_prefix = tmp.0.join("mp3-prefix-max-packets.wav");
    let stats = media::decode_audio_transformed(&source, &out_prefix, full_transform, &prefix_opts)
        .expect("decode_audio_transformed on 1-packet raw MP3 prefix must succeed");
    assert_eq!(stats.sample_frames, 1152);

    // Interval-bounded prefix without max_packets: [0, 24_000) us covers only Frame 0 (pts 0..1152).
    let mut interval_opts = CopyOptions::default();
    interval_opts.max_packet_bytes = 960;
    let interval_transform = AudioDecodeTransform {
        interval: Some((0, 24_000)),
        ..Default::default()
    };
    media::plan_decode_audio(&source, &interval_transform, &interval_opts)
        .expect("plan_decode_audio on 24ms raw MP3 interval must succeed when frame 0 is 960 bytes");
    let out_interval = tmp.0.join("mp3-prefix-interval.wav");
    let interval_stats = media::decode_audio_transformed(
        &source,
        &out_interval,
        interval_transform,
        &interval_opts,
    )
    .expect("decode_audio_transformed on 24ms raw MP3 interval must succeed");
    assert_eq!(interval_stats.sample_frames, 1152);

    let mut raw_interval_pcm = Vec::new();
    let raw_interval_stats = fvid_media::owned_raw_mp3::decode_raw_mpeg_pcm(
        &source,
        &mut raw_interval_pcm,
        Some((Duration::ZERO, Duration::from_micros(24_000))),
        &interval_opts,
    )
    .expect("decode_raw_mpeg_pcm on 24ms interval must succeed");
    assert_eq!(raw_interval_stats.sample_frames, 1152);

    media::plan_trim(&source, 0, 24_000, &interval_opts)
        .expect("plan_trim on 24ms raw MP3 interval must succeed");
    let out_trim = tmp.0.join("mp3-trim.wav");
    media::trim(&source, &out_trim, 0, 24_000, &interval_opts)
        .expect("trim on 24ms raw MP3 interval must succeed");

    // Full decode without prefix bounds must still reject Frame 1 (961 bytes > 960).
    let plan_err = media::plan_decode_audio(&source, &full_transform, &interval_opts)
        .expect_err("full raw MP3 plan must reject 961-byte frame 1 when max_packet_bytes=960");
    assert!(
        plan_err.to_string().contains("MPEG packet bytes exceed limit"),
        "unexpected error: {plan_err}"
    );
    let out_full = tmp.0.join("mp3-full.wav");
    let exec_err = media::decode_audio_transformed(&source, &out_full, full_transform, &interval_opts)
        .expect_err("full raw MP3 decode must reject 961-byte frame 1 when max_packet_bytes=960");
    assert!(
        exec_err.to_string().contains("MPEG packet bytes exceed limit"),
        "unexpected error: {exec_err}"
    );
    assert!(!out_full.exists());
}
