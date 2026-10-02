use fvid_control::{CancelFlag, CopyOptions, ProgressHook};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
#[test]
fn container_loudness_matches_owned_pcm_and_encoded_progress() {
    let folder =
        std::env::temp_dir().join(format!("fvid-container-loudness-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(folder.clone());
    for (name, streams) in [
        ("audio.mp4", vec![]),
        ("playback-errors/alac-select-second.m4a", vec![1]),
        ("audio/aac-stereo.mka", vec![]),
        ("playback-errors/alac-presentation-timeline.mka", vec![]),
        ("playback-errors/pcm64-resample-window.mka", vec![]),
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let options = CopyOptions {
            streams,
            max_packets: Some(2),
            progress: Some(ProgressHook::new(move |event| {
                captured.lock().unwrap().push(event)
            })),
            ..Default::default()
        };
        let reference = folder.join("reference.wav");
        let mut quiet = options.clone();
        quiet.progress = None;
        fvid_media::owned_audio_export::decode_audio_transformed(
            &source,
            &reference,
            Default::default(),
            &quiet,
        )
        .unwrap();
        let mut expected =
            fvid_media::owned_wave_loudness::measure_loudness(&reference, &Default::default())
                .unwrap();
        expected.backend = "owned container audio loudness";
        let actual = fvid_media::measure_loudness(&source, &options).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{name}"
        );
        std::fs::remove_file(&reference).unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e.done).count(), 1);
        assert_eq!(events.last().unwrap().packets, 2);
        assert!(events.last().unwrap().done);
        assert!(events
            .windows(2)
            .all(|w| w[0].packets <= w[1].packets && w[0].payload_bytes <= w[1].payload_bytes));
        let options = CopyOptions {
            progress: Some(ProgressHook::new(|_| panic!("plan emitted progress"))),
            ..quiet
        };
        let plan = fvid_media::plan_loudness(&source, &options).unwrap();
        assert_eq!(plan.command, "loudness");
        assert_eq!(plan.streams[0].disposition, "analyze");
        assert_eq!(
            plan.streams[0].index,
            options.streams.first().copied().unwrap_or(0)
        );
        assert!(!plan.steps.iter().any(|s| s.action == "write"));
        assert!(plan.notes.iter().any(|n| n.contains("backend: owned")));
    }
}
#[test]
fn container_loudness_cancellation_never_emits_completion() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let flag = CancelFlag::default();
    let captured = flag.clone();
    let options = CopyOptions {
        cancel: Some(flag),
        progress: Some(ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets > 0 {
                captured.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_container_loudness::measure_loudness(&source, &options)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(fvid_media::owned_container_loudness::measure_loudness(
        &source,
        &CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        }
    )
    .is_err());
}
