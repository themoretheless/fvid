use fvid_control::{CopyOptions, ProgressHook};
use std::path::Path;

#[test]
fn matroska_audio_plans_use_owned_metadata_and_preserve_precision() {
    for (name, codec, precision) in [
        ("audio/aac-stereo.mka", "aac", "float32"),
        (
            "playback-errors/alac-presentation-timeline.mka",
            "alac",
            "float32",
        ),
        (
            "playback-errors/pcm64-resample-window.mka",
            "A_PCM/FLOAT/IEEE",
            "float64",
        ),
        (
            "playback-errors/pcm32-precision-big.mka",
            "A_PCM/INT/BIG",
            "float64",
        ),
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let before = std::fs::read(&source).unwrap();
        let options = CopyOptions {
            max_packet_bytes: 1,
            progress: Some(ProgressHook::new(|_| {
                panic!("metadata planning emitted progress")
            })),
            ..Default::default()
        };
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((1000, 2000)),
            volume: Some(0.5),
            sample_rate: Some(16000),
            ..Default::default()
        };
        let owned =
            fvid_media::owned_audio_plan::plan_decode_audio(&source, &transform, &options).unwrap();
        let public = fvid_media::plan_decode_audio(&source, &transform, &options).unwrap();
        assert_eq!(
            serde_json::to_value(&owned).unwrap(),
            serde_json::to_value(&public).unwrap()
        );
        assert_eq!(public.streams[0].codec, codec);
        assert_eq!(public.streams[0].index, 0);
        assert!(public.steps[0].detail.contains("owned Matroska"));
        assert!(public.steps[0].detail.contains(precision));
        assert!(public.steps.last().unwrap().detail.contains(precision));
        assert!(public.steps[0].detail.contains("owned Matroska"));
        assert!(public.steps[0].detail.contains(precision));
        assert!(public.steps.last().unwrap().detail.contains(precision));
        assert!(public
            .notes
            .iter()
            .any(|n| n.contains("selected encoded audio")));
        assert_eq!(std::fs::read(&source).unwrap(), before);
        let error = fvid_media::owned_audio_plan::plan_decode_audio(
            &source,
            &transform,
            &CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("controlled memory budget exceeded"));
        fvid_media::owned_audio_plan::plan_decode_audio(
            &source,
            &transform,
            &CopyOptions {
                max_controlled_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(fvid_media::owned_audio_plan::plan_decode_audio(
            &source,
            &transform,
            &CopyOptions {
                streams: vec![99],
                ..Default::default()
            }
        )
        .is_err());
    }
}
