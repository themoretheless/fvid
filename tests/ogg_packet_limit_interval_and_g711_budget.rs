use std::path::PathBuf;

use fvid_control::CopyOptions;
use fvid_media::{decode_audio_transformed, plan_decode_audio};
use fvid_media_info::AudioDecodeTransform;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("ogg-packet-limit-interval-and-g711-budget")
}

#[test]
fn ogg_opus_plan_decode_audio_rejects_interval_past_max_packets_prefix() {
    let opus = fixture_dir().join("ogg-opus-two-packet.ogg");
    // Packet 0 covers [0, 960) samples = [0, 20_000) us at 48 kHz;
    // Packet 1 covers [960, 1920) samples = [20_000, 40_000) us.
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((25_000, 35_000));

    let mut limited = CopyOptions::default();
    limited.max_packets = Some(1);
    let err = plan_decode_audio(&opus, &transform, &limited)
        .expect_err("Ogg Opus interval in packet 1 must be rejected when max_packets=1");
    assert!(
        err.contains("no decoded audio samples"),
        "unexpected error: {err}"
    );

    limited.max_packets = Some(2);
    let plan = plan_decode_audio(&opus, &transform, &limited)
        .expect("Ogg Opus interval within 2-packet prefix should succeed");
    assert!(!plan.steps.is_empty());
}

#[test]
fn ogg_vorbis_plan_decode_audio_rejects_interval_past_max_packets_prefix() {
    let vorbis = fixture_dir().join("ogg-vorbis-six-packet.ogg");
    // Packet 0 ends at 0 samples; packet 1 ends at 80 samples (~= 1666 us at 48 kHz);
    // full stream ends at 411 samples (~= 8562 us).
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((5_000, 8_000));

    let mut limited = CopyOptions::default();
    limited.max_packets = Some(2);
    let err = plan_decode_audio(&vorbis, &transform, &limited)
        .expect_err("Ogg Vorbis interval after packet 1 must be rejected when max_packets=2");
    assert!(
        err.contains("no decoded audio samples"),
        "unexpected error: {err}"
    );

    limited.max_packets = Some(6);
    let plan = plan_decode_audio(&vorbis, &transform, &limited)
        .expect("Ogg Vorbis interval within 6-packet prefix should succeed");
    assert!(!plan.steps.is_empty());
}

#[test]
fn ogg_joint_plan_decode_audio_rejects_interval_past_max_packets_prefix() {
    let joint = fixture_dir().join("ogg-joint-vorbis-opus.ogg");
    // Link 0 (Vorbis) has 6 packets covering [0, 411) samples (~= 8562 us);
    // Link 1 (Opus) has 2 packets covering [411, 2331) samples (~= 8562..48562 us).
    let mut transform = AudioDecodeTransform::default();
    transform.interval = Some((20_000, 30_000));

    let mut limited = CopyOptions::default();
    limited.max_packets = Some(6);
    let err = plan_decode_audio(&joint, &transform, &limited)
        .expect_err("Ogg Joint interval in link 1 must be rejected when max_packets=6 stops after link 0");
    assert!(
        err.contains("no decoded audio samples"),
        "unexpected error: {err}"
    );

    limited.max_packets = Some(8);
    let plan = plan_decode_audio(&joint, &transform, &limited)
        .expect("Ogg Joint interval within 8-packet prefix should succeed");
    assert!(!plan.steps.is_empty());
}

#[test]
fn g711_wave_export_and_plan_bound_spool_budget_by_max_packets_and_full_frames() {
    let g711 = fixture_dir().join("g711-alaw-multi-packet.wav");
    // 16384 frames at 8 kHz mono. With volume=0.5 (non-identity Stage-2 float32 WAVE export),
    // full stream requires 16384 * 4 * 3 = 196_608 bytes + 65536 = 262_144+ bytes in Stage-2,
    // whereas max_packets=1 (2048 frames) requires 2048 * 4 * 3 + 65536 = 90_112 bytes
    // (and open() requires ~274_432 bytes).
    // If we use 8-channel upmix (`channels: Some(8)`):
    // - Full 16384 frames -> estimate_float_wave_export_bytes needs > 1.5 MiB!
    // - 1 packet (2048 frames) -> estimate_float_wave_export_bytes needs ~330 KiB!
    let mut transform = AudioDecodeTransform::default();
    transform.channels = Some(8);

    // 1. Without max_packets, 500 KiB is too small for all 16384 frames of 8-channel output,
    // so both plan_decode_audio and decode_audio_transformed must reject it!
    let mut unbounded_packets = CopyOptions::default();
    unbounded_packets.max_controlled_bytes = Some(500 * 1024);
    let plan_err = plan_decode_audio(&g711, &transform, &unbounded_packets)
        .expect_err("plan_decode_audio must reject G.711 8ch export when full frame count exceeds 500 KiB budget");
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "unexpected error: {plan_err}"
    );

    // 2. With max_packets=1 (2048 frames), 500 KiB is plenty for 1 packet of 8-channel output,
    // so both plan_decode_audio and decode_audio_transformed must succeed!
    let mut one_packet = CopyOptions::default();
    one_packet.max_packets = Some(1);
    one_packet.max_controlled_bytes = Some(500 * 1024);
    let plan = plan_decode_audio(&g711, &transform, &one_packet)
        .expect("plan_decode_audio with max_packets=1 should fit within 500 KiB");
    assert!(!plan.steps.is_empty());

    let out_wav = std::env::temp_dir().join(format!(
        "fvid-g711-max-packets-budget-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav);
    let stats = decode_audio_transformed(&g711, &out_wav, transform, &one_packet)
        .expect("decode_audio_transformed with max_packets=1 should fit within 500 KiB");
    let _ = std::fs::remove_file(&out_wav);
    assert_eq!(stats.sample_frames, 2048);
    assert_eq!(stats.channels, 8);
}
