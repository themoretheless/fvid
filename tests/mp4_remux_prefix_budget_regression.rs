use fvid::media;
use fvid::media_control::CopyOptions;
use fvid::media_info::AudioDecodeTransform;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "fvid-batch267-{}-{}-{}",
            tag,
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-remux-prefix-budget")
}

#[test]
fn mp4_matroska_remux_and_plan_remux_scope_packet_budget_to_selected_prefix_and_tracks() {
    let dir = fixture_dir();
    let single_mixed = dir.join("single-mixed.m4a");
    let dual_mixed = dir.join("dual-track-mixed.m4a");
    let tmp = TempDir::new("remux");

    // 1. Single-track MP4 where sample #0 is 8 bytes and sample #1 is 48 bytes:
    // With max_packets = Some(1) and max_packet_bytes = 16, only sample #0 is copied,
    // so both plan_remux and remux to .mka/.mkv must succeed.
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        ..Default::default()
    };
    let plan = media::plan_remux(&single_mixed, &prefix_opts)
        .expect("media::plan_remux should admit 1-packet prefix within 16-byte budget");
    assert_eq!(plan.command, "remux");
    assert!(
        plan.steps[2].detail.contains("copy 1 compressed packets, 8 payload bytes"),
        "unexpected plan detail: {}",
        plan.steps[2].detail
    );

    let out_mka = tmp.path().join("single-prefix.mka");
    let stats = media::remux(&single_mixed, &out_mka, &prefix_opts)
        .expect("media::remux should succeed for 1-packet prefix within 16-byte budget");
    assert_eq!(stats.packets, 1);
    assert_eq!(stats.payload_bytes, 8);
    let loudness = media::measure_loudness(&out_mka, &CopyOptions::default()).unwrap();
    assert_eq!(loudness.sample_frames, 1024);

    // Also verify fvid_media::owned_mp4_remux directly with default max_packets when an unselected track exceeds max_packet_bytes:
    let fvid_stream0_opts = CopyOptions {
        streams: vec![0],
        max_packet_bytes: 16,
        ..Default::default()
    };
    assert!(
        fvid_media::owned_mp4_remux::plan_remux(&dual_mixed, &fvid_stream0_opts).is_ok(),
        "fvid_media::owned_mp4_remux::plan_remux should ignore unselected track 1 packet sizes"
    );

    // Without max_packets (or with max_packets = Some(2)), sample #1 (48 bytes) is visited and must be refused:
    let full_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(2),
        ..Default::default()
    };
    let err_plan = media::plan_remux(&single_mixed, &full_opts).unwrap_err();
    assert!(
        err_plan.contains("MP4 packet exceeds budget"),
        "unexpected plan error: {err_plan}"
    );
    let out_mka_refused = tmp.path().join("single-refused.mka");
    let err_remux = media::remux(&single_mixed, &out_mka_refused, &full_opts).unwrap_err();
    assert!(
        err_remux.contains("MP4 packet exceeds budget"),
        "unexpected remux error: {err_remux}"
    );
    assert!(!out_mka_refused.exists());

    // 2. Dual-track MP4 where track 0 has two 8-byte packets and track 1 has two 48-byte packets:
    // Selecting only stream 0 (`streams: vec![0]`) with `max_packet_bytes: 16` must succeed.
    let stream0_opts = CopyOptions {
        streams: vec![0],
        max_packet_bytes: 16,
        ..Default::default()
    };
    let dual_plan = media::plan_remux(&dual_mixed, &stream0_opts)
        .expect("media::plan_remux should ignore unselected track 1 packet sizes");
    assert_eq!(dual_plan.streams.len(), 1);
    assert_eq!(dual_plan.streams[0].index, 0);

    let out_dual_mka = tmp.path().join("dual-stream0.mka");
    let dual_stats = media::remux(&dual_mixed, &out_dual_mka, &stream0_opts)
        .expect("media::remux should ignore unselected track 1 packet sizes");
    assert_eq!(dual_stats.packets, 2);
    assert_eq!(dual_stats.payload_bytes, 16);
    let dual_loudness = media::measure_loudness(&out_dual_mka, &CopyOptions::default()).unwrap();
    assert_eq!(dual_loudness.sample_frames, 2048);

    // Selecting stream 1 (`streams: vec![1]`) with `max_packet_bytes: 16` must fail before creating output:
    let stream1_opts = CopyOptions {
        streams: vec![1],
        max_packet_bytes: 16,
        ..Default::default()
    };
    let err_dual_plan = media::plan_remux(&dual_mixed, &stream1_opts).unwrap_err();
    assert!(
        err_dual_plan.contains("MP4 packet exceeds budget"),
        "unexpected dual stream1 plan error: {err_dual_plan}"
    );
    let out_dual_refused = tmp.path().join("dual-stream1-refused.mka");
    let err_dual_remux = media::remux(&dual_mixed, &out_dual_refused, &stream1_opts).unwrap_err();
    assert!(
        err_dual_remux.contains("MP4 packet exceeds budget"),
        "unexpected dual stream1 remux error: {err_dual_remux}"
    );
    assert!(!out_dual_refused.exists());
}

#[test]
fn mp4_compressed_concat_scopes_packet_budget_to_selected_prefix_and_tracks() {
    let dir = fixture_dir();
    let dual_mixed = dir.join("dual-track-mixed.m4a");
    let single_mixed = dir.join("single-mixed.m4a");
    let seg_a_small = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-concat-packet-prefix-budget/seg-a-small.m4a");
    let tmp = TempDir::new("concat");

    // 1. Compressed MP4 concat of [seg_a_small, single_mixed] where seg_a_small has two 8-byte packets
    // and single_mixed has an 8-byte packet 0 and a 48-byte packet 1:
    // With max_packets = Some(3) and max_packet_bytes = 16, compressed MP4-to-Matroska concat
    // copies 2 packets from seg_a_small and 1 packet from single_mixed (all 8 bytes) and succeeds!
    let concat_prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(3),
        ..Default::default()
    };
    let sources = vec![seg_a_small, single_mixed];
    let concat_plan = media::plan_concat(&sources, &concat_prefix_opts)
        .expect("plan_concat should admit 3-packet compressed MP4 concat prefix within 16-byte budget");
    assert_eq!(concat_plan.streams[0].disposition, "copy");

    let out_concat_mka = tmp.path().join("concat-prefix.mka");
    let concat_stats = media::concat(&sources, &out_concat_mka, &concat_prefix_opts)
        .expect("concat should succeed for 3-packet compressed MP4 concat prefix within 16-byte budget");
    assert_eq!(concat_stats.packets, 3);
    assert_eq!(concat_stats.payload_bytes, 24);

    // With max_packets = Some(4), packet #4 (48 bytes) is visited and must fail with MP4 packet exceeds budget:
    let concat_exceed_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(4),
        ..Default::default()
    };
    let err_concat_plan = media::plan_concat(&sources, &concat_exceed_opts).unwrap_err();
    assert!(
        err_concat_plan.contains("MP4 packet exceeds budget"),
        "unexpected concat plan error: {err_concat_plan}"
    );
    let out_concat_refused = tmp.path().join("concat-refused.mka");
    let err_concat = media::concat(&sources, &out_concat_refused, &concat_exceed_opts).unwrap_err();
    assert!(
        err_concat.contains("MP4 packet exceeds budget"),
        "unexpected concat error: {err_concat}"
    );
    assert!(!out_concat_refused.exists());

    // 2. Compressed MP4 concat of [dual_mixed, dual_mixed] with `streams: vec![0]` and `max_packet_bytes: 16`:
    let dual_sources = vec![dual_mixed.clone(), dual_mixed];
    let dual_stream0_opts = CopyOptions {
        streams: vec![0],
        max_packet_bytes: 16,
        ..Default::default()
    };
    let dual_concat_plan = media::plan_concat(&dual_sources, &dual_stream0_opts)
        .expect("plan_concat should ignore unselected track 1 packet sizes");
    assert_eq!(dual_concat_plan.streams.len(), 1);
    assert_eq!(dual_concat_plan.streams[0].disposition, "copy");

    let out_dual_concat = tmp.path().join("dual-concat-stream0.mka");
    let dual_concat_stats = media::concat(&dual_sources, &out_dual_concat, &dual_stream0_opts)
        .expect("concat should ignore unselected track 1 packet sizes");
    assert_eq!(dual_concat_stats.packets, 4);
    assert_eq!(dual_concat_stats.payload_bytes, 32);
}

#[test]
fn native_plan_decode_audio_with_options_forwards_budgets_for_dynamic_stsd_mp4() {
    let dir = fixture_dir();
    let dynamic_mp4 = dir.join("dynamic-stsd-mixed.m4a");

    // 1. With max_packets = Some(1) and max_packet_bytes = 16, sample #0 (8 bytes) fits:
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(1),
        ..Default::default()
    };
    let plan = fvid::native_plan::decode_audio_with_options(
        &dynamic_mp4,
        &AudioDecodeTransform::default(),
        &prefix_opts,
    )
    .expect("decode_audio_with_options should admit 1-packet prefix on dynamic-stsd MP4");
    assert_eq!(plan.command, "decode-audio");

    // 2. Without max_packets (or max_packets = Some(2)), sample #1 (48 bytes) exceeds max_packet_bytes = 16
    // and must be refused by decode_audio_with_options instead of being silently ignored by CopyOptions::default():
    let tight_opts = CopyOptions {
        max_packet_bytes: 16,
        ..Default::default()
    };
    let err = fvid::native_plan::decode_audio_with_options(
        &dynamic_mp4,
        &AudioDecodeTransform::default(),
        &tight_opts,
    )
    .unwrap_err();
    assert!(
        err.contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget, got: {err}"
    );

    // Also via media::plan_loudness (which delegates to native_plan::loudness_with_options -> decode_audio_with_options):
    let err_loudness = media::plan_loudness(&dynamic_mp4, &tight_opts).unwrap_err();
    assert!(
        err_loudness.contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget from plan_loudness, got: {err_loudness}"
    );
    let ok_loudness = media::plan_loudness(&dynamic_mp4, &prefix_opts)
        .expect("plan_loudness should admit 1-packet prefix on dynamic-stsd MP4");
    assert_eq!(ok_loudness.command, "loudness");
    let measured = media::measure_loudness(&dynamic_mp4, &prefix_opts)
        .expect("measure_loudness should decode 1-packet prefix on dynamic-stsd MP4");
    assert_eq!(measured.sample_frames, 1024);

    let tmp = TempDir::new("dynamic-decode");
    let out_wav = tmp.path().join("dynamic-prefix.wav");
    let decoded = media::decode_audio_transformed(
        &dynamic_mp4,
        &out_wav,
        AudioDecodeTransform::default(),
        &prefix_opts,
    )
    .expect("decode_audio_transformed should decode 1-packet prefix on dynamic-stsd MP4");
    assert_eq!(decoded.sample_frames, 1024);
    assert_eq!(decoded.decoded_frames, 1);

    let out_refused = tmp.path().join("dynamic-refused.wav");
    let err_decode = media::decode_audio_transformed(
        &dynamic_mp4,
        &out_refused,
        AudioDecodeTransform::default(),
        &tight_opts,
    )
    .unwrap_err();
    assert!(
        err_decode.contains("MP4 packet exceeds budget"),
        "expected MP4 packet exceeds budget from decode_audio_transformed, got: {err_decode}"
    );
    assert!(!out_refused.exists());
}
