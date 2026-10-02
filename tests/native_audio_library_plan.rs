use fvid_media::{AudioDecodeTransform, CopyOptions, ProgressHook};
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn public_audio_plan_uses_owned_stages_without_decoding_or_progress() {
    for name in ["wave-probe-info.wav", "aac-packet-prefix.aac"] {
        let source = fixture(name);
        let before = std::fs::read(&source).unwrap();
        let options = CopyOptions {
            max_packets: Some(3),
            metadata_set: vec![("title".into(), "Synthetic plan".into())],
            progress: Some(ProgressHook::new(|_| panic!("plan emitted progress"))),
            ..Default::default()
        };
        let transform = AudioDecodeTransform {
            sample_rate: Some(16000),
            volume: Some(0.5),
            interval: Some((10000, 50000)),
            ..Default::default()
        };
        let owned =
            fvid_media::owned_audio_plan::plan_decode_audio(&source, &transform, &options).unwrap();
        let public = fvid_media::plan_decode_audio(&source, &transform, &options).unwrap();
        assert_eq!(
            serde_json::to_value(&owned).unwrap(),
            serde_json::to_value(&public).unwrap()
        );
        assert_eq!(
            public
                .steps
                .iter()
                .map(|s| s.action.as_str())
                .collect::<Vec<_>>(),
            ["decode", "volume", "resample", "trim", "metadata", "write"]
        );
        assert!(public.graph.is_none());
        assert_eq!(std::fs::read(source).unwrap(), before);
        assert!(public.notes.iter().any(|s| s.contains("during execution")));
        // The truncated fourth AAC header is intentionally not decoded by this
        // preflight. Execution with max_packets=3 is covered by file-export tests.
    }
}
#[test]
fn plan_refuses_requests_the_owned_writer_cannot_execute() {
    let source = fixture("wave-probe-info.wav");
    for transform in [
        AudioDecodeTransform {
            volume: Some(f64::NAN),
            ..Default::default()
        },
        AudioDecodeTransform {
            sample_rate: Some(1),
            ..Default::default()
        },
        AudioDecodeTransform {
            channels: Some(0),
            ..Default::default()
        },
        AudioDecodeTransform {
            interval: Some((1, 1)),
            ..Default::default()
        },
    ] {
        assert!(
            fvid_media::owned_audio_plan::plan_decode_audio(
                &source,
                &transform,
                &Default::default()
            )
            .is_err()
        );
    }
    let options = CopyOptions {
        streams: vec![1],
        ..Default::default()
    };
    assert!(
        fvid_media::owned_audio_plan::plan_decode_audio(&source, &Default::default(), &options)
            .is_err()
    );
    let adts = fixture("aac-packet-prefix.aac");
    let options = CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_audio_plan::plan_decode_audio(&adts, &Default::default(), &options)
            .unwrap_err()
            .contains("allocation admission")
    );
    let options = CopyOptions {
        metadata_set: vec![("title".into(), "bad\0value".into())],
        ..Default::default()
    };
    assert!(
        fvid_media::owned_audio_plan::plan_decode_audio(&source, &Default::default(), &options)
            .unwrap_err()
            .contains("NUL")
    );
}
