use fvid::{
    media_control::{CancelFlag, CopyOptions, ProgressHook},
    media_info,
};
#[test]
fn native_results_have_the_shared_public_schema_without_media_feature() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let stats: media_info::DecodeStats = fvid::native_media::decode_video(&path).unwrap();
    let json = serde_json::to_value(&stats).unwrap();
    assert_eq!(json["backend"], "fvid");
    assert_eq!(json["video_frames"], 25);
    assert_eq!(json["decode_errors"], 0);
    let keys: std::collections::BTreeSet<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "backend",
            "video_frames",
            "width",
            "height",
            "pixel_format",
            "decode_errors"
        ]
        .into_iter()
        .collect()
    );
    let pcm: fvid::native_pcm::PcmTrimStats = media_info::PcmTrimStats {
        packets: 2,
        sample_frames: 4,
        payload_bytes: 16,
        fvid_payload_copies: 0,
    };
    assert_eq!(
        serde_json::to_value(pcm).unwrap(),
        serde_json::json!({"packets":2,"sample_frames":4,"payload_bytes":16,"fvid_payload_copies":0})
    );
    #[cfg(feature = "media")]
    {
        let legacy_alias: fvid::media::DecodeStats = stats;
        let actual = fvid::media::decode_video(&path).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(legacy_alias).unwrap()
        );
        let _: fvid::media::PcmTrimStats = pcm;
        fn copy_alias(v: media_info::CopyStats) -> fvid::media::CopyStats {
            v
        }
        fn audio_alias(v: media_info::AudioDecodeStats) -> fvid::media::AudioDecodeStats {
            v
        }
        fn lossless_alias(v: media_info::LosslessStats) -> fvid::media::LosslessStats {
            v
        }
        let _ = (copy_alias, audio_alias, lossless_alias);
    }
}
#[test]
fn independent_options_preserve_shared_cancellation_and_progress() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let flag = CancelFlag::new();
    let captured = flag.clone();
    let events = Arc::new(AtomicUsize::new(0));
    let seen = events.clone();
    let options = CopyOptions {
        cancel: Some(flag),
        progress: Some(ProgressHook::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            captured.cancel();
        })),
        ..Default::default()
    };
    assert_eq!(options.max_packet_bytes, 64 * 1024 * 1024);
    assert!(options.streams.is_empty());
    let clone = options.clone();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-mono-44k.aac");
    let path =
        std::env::temp_dir().join(format!("fvid-contract-cancel-{}.m4a", std::process::id()));
    assert!(!path.exists());
    assert!(
        fvid::native_export::remux_adts_aac_controlled(
            &source,
            &path,
            clone.cancel.as_ref(),
            clone.progress.as_ref()
        )
        .is_err()
    );
    assert_eq!(events.load(Ordering::Relaxed), 1);
    assert!(options.cancel.as_ref().unwrap().is_cancelled());
    assert!(!path.exists());
    #[cfg(feature = "media")]
    {
        let alias: fvid::media::CopyOptions = options;
        assert!(alias.cancel.unwrap().is_cancelled());
    }
}
