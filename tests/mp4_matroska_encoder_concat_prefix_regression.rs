use fvid::media::{self, CopyOptions};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-matroska-encoder-concat-prefix")
}

#[test]
fn mp4_to_matroska_remux_and_concat_preserve_itunes_encoder_tag_and_allow_delete_or_override() {
    let seg_a = fixture_dir().join("segment-a.m4a");
    let temp = std::env::temp_dir().join(format!("fvid-batch265-remux-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. Default MP4-to-Matroska remux preserves ©too as Matroska ENCODER tag.
    let remux_default = temp.join("remux-default.mka");
    let stats = media::remux(&seg_a, &remux_default, &CopyOptions::default()).unwrap();
    assert_eq!(stats.packets, 2);
    let info = media::probe(&remux_default).unwrap();
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("MP4 Remux And Concat Title")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-remux-encoder 2.0")
    );

    // 2. MP4-to-Matroska remux with max_packets = Some(1) (routes through fvid_media::owned_mp4_remux)
    // also preserves ©too as ENCODER.
    let remux_prefix = temp.join("remux-prefix.mka");
    let prefix_opts = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let prefix_stats = media::remux(&seg_a, &remux_prefix, &prefix_opts).unwrap();
    assert_eq!(prefix_stats.packets, 1);
    let prefix_info = media::probe(&remux_prefix).unwrap();
    assert_eq!(
        prefix_info.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-remux-encoder 2.0")
    );

    // 3. Explicit metadata_set overrides the source ©too encoder tag without duplicating it.
    let remux_override = temp.join("remux-override.mka");
    let override_opts = CopyOptions {
        metadata_set: vec![("encoder".into(), "custom-remux-encoder 3.0".into())],
        ..Default::default()
    };
    media::remux(&seg_a, &remux_override, &override_opts).unwrap();
    let override_info = media::probe(&remux_override).unwrap();
    assert_eq!(
        override_info.metadata.get("encoder").map(String::as_str),
        Some("custom-remux-encoder 3.0")
    );

    // 4. Explicit metadata_delete removes the source ©too encoder tag (so probe falls back to muxer WritingApp "FVid").
    let remux_delete = temp.join("remux-delete.mka");
    let delete_opts = CopyOptions {
        metadata_delete: vec!["encoder".into()],
        ..Default::default()
    };
    media::remux(&seg_a, &remux_delete, &delete_opts).unwrap();
    let delete_info = media::probe(&remux_delete).unwrap();
    assert_eq!(
        delete_info.metadata.get("encoder").map(String::as_str),
        Some("FVid")
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn mp4_to_matroska_concat_scopes_packet_byte_budget_to_max_packets_prefix_and_preserves_encoder() {
    let seg_a = fixture_dir().join("segment-a.m4a");
    let seg_b = fixture_dir().join("segment-b.m4a");
    let inputs = vec![seg_a.clone(), seg_b.clone()];
    let temp = std::env::temp_dir().join(format!("fvid-batch265-concat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // segment-a has two 8-byte packets; segment-b has an 8-byte packet 0 and a 48-byte packet 1.
    // Without max_packets (or with max_packets = Some(4)), packet 1 of segment-b (48B > 16B) must be rejected.
    for max_packets in [None, Some(4)] {
        let opts = CopyOptions {
            max_packet_bytes: 16,
            max_packets,
            ..Default::default()
        };
        let err = media::plan_concat(&inputs, &opts).unwrap_err();
        assert!(
            err.to_string().contains("MP4 packet exceeds budget"),
            "expected MP4 packet budget refusal for max_packets={max_packets:?}, got: {err}"
        );
        let out = temp.join(format!("concat-fail-{max_packets:?}.mka"));
        let exec_err = media::concat(&inputs, &out, &opts).unwrap_err();
        assert!(
            exec_err.to_string().contains("MP4 packet exceeds budget"),
            "expected MP4 packet budget refusal in concat for max_packets={max_packets:?}, got: {exec_err}"
        );
    }

    // With max_packets = Some(3), the cumulative prefix visits both 8-byte packets of segment-a
    // and packet 0 (8 bytes) of segment-b, stopping before the 48-byte packet 1 of segment-b!
    let prefix_opts = CopyOptions {
        max_packet_bytes: 16,
        max_packets: Some(3),
        ..Default::default()
    };
    let plan = media::plan_concat(&inputs, &prefix_opts).unwrap();
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].disposition, "copy");

    let out_mka = temp.join("concat-prefix-3.mka");
    let stats = media::concat(&inputs, &out_mka, &prefix_opts).unwrap();
    assert_eq!(stats.packets, 3);
    assert_eq!(stats.payload_bytes, 24);

    let info = media::probe(&out_mka).unwrap();
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("MP4 Remux And Concat Title")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-mp4-remux-encoder 2.0")
    );

    let _ = std::fs::remove_dir_all(&temp);
}
