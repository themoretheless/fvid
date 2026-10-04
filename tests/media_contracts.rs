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
    {
        let alias: fvid::media::CopyOptions = options;
        assert!(alias.cancel.unwrap().is_cancelled());
    }
}

#[test]
fn shared_video_requests_execute_headlessly_and_keep_legacy_type_compatibility() {
    use media_info::{CropRect, DecodeTransform, ScaleSize, TransposeMode};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let transform = DecodeTransform {
        crop: Some(CropRect {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        }),
        horizontal_flip: true,
        transpose: Some(TransposeMode::Clock),
        scale: Some(ScaleSize {
            width: 4,
            height: 4,
        }),
        interval: Some((0, 200000)),
        ..Default::default()
    };
    let stats = fvid::native_media::decode_video_request(&path, &transform).unwrap();
    assert_eq!((stats.width, stats.height), (4, 4));
    assert_eq!(stats.backend, "fvid");
    assert!(stats.video_frames > 0);
    let blurred = DecodeTransform {
        gblur: Some("sigma=1".into()),
        ..Default::default()
    };
    let blurred_stats = fvid::native_media::decode_video_request(&path, &blurred).unwrap();
    assert_eq!(blurred_stats.backend, "fvid");
    assert_eq!(blurred_stats.video_frames, 25);
    let invalid = DecodeTransform {
        interval: Some((-1, 0)),
        ..Default::default()
    };
    assert!(fvid::native_media::decode_video_request(&path, &invalid).is_err());
    {
        let legacy: fvid::media::DecodeTransform = transform;
        assert_eq!(
            serde_json::to_value(fvid::media::decode_video_transformed(&path, legacy).unwrap())
                .unwrap(),
            serde_json::to_value(stats).unwrap()
        );
        let _: fvid::media::LosslessTransform = media_info::LosslessTransform::default();
        let _: fvid::media::OverlaySpec = media_info::OverlaySpec::default();
        let _: fvid::media::XfadeSpec = media_info::XfadeSpec::default();
        let _: fvid::media::RotateAngle = media_info::RotateAngle::parse("90").unwrap();
        let _: fvid::media::PadRect = media_info::PadRect {
            width: 4,
            height: 4,
            x: 0,
            y: 0,
        };
    }
}
