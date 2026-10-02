use fvid_control::{CopyOptions, ProgressHook};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
#[test]
fn container_normalization_matches_owned_wave_for_both_pass_modes() {
    let folder =
        std::env::temp_dir().join(format!("fvid-container-normalize-{}", std::process::id()));
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
        let quiet = CopyOptions {
            streams,
            max_packets: Some(2),
            ..Default::default()
        };
        let reference = folder.join("reference.wav");
        fvid_media::owned_audio_export::decode_audio_transformed(
            &source,
            &reference,
            Default::default(),
            &quiet,
        )
        .unwrap();
        for dual in [false, true] {
            let args = Some("I=-16:TP=-1.5:LRA=11");
            let baseline = folder.join("baseline.wav");
            let destination = folder.join("normalized.wav");
            let expected = if dual {
                fvid_media::owned_loudnorm::apply_loudnorm_dual(
                    &reference,
                    &baseline,
                    args,
                    &Default::default(),
                )
            } else {
                fvid_media::owned_loudnorm::apply_loudnorm(
                    &reference,
                    &baseline,
                    args,
                    &Default::default(),
                )
            }
            .unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let captured = events.clone();
            let options = CopyOptions {
                progress: Some(ProgressHook::new(move |event| {
                    captured.lock().unwrap().push(event)
                })),
                ..quiet.clone()
            };
            let actual = if dual {
                fvid_media::apply_loudnorm_dual(&source, &destination, args, &options)
            } else {
                fvid_media::apply_loudnorm(&source, &destination, args, &options)
            }
            .unwrap();
            assert_eq!(
                std::fs::read(&destination).unwrap(),
                std::fs::read(&baseline).unwrap(),
                "{name}, dual={dual}"
            );
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            let events = events.lock().unwrap();
            assert_eq!(events.iter().filter(|e| e.done).count(), 1);
            assert!(events.last().unwrap().done);
            assert!(events
                .windows(2)
                .all(|w| w[0].packets <= w[1].packets && w[0].payload_bytes <= w[1].payload_bytes));
            let saved = std::fs::read(&destination).unwrap();
            assert!(fvid_media::owned_container_loudnorm::apply(
                &source,
                &destination,
                args,
                dual,
                &quiet
            )
            .is_err());
            assert_eq!(saved, std::fs::read(&destination).unwrap());
            let plan_options = CopyOptions {
                progress: Some(ProgressHook::new(|_| panic!("plan emitted progress"))),
                ..quiet.clone()
            };
            let plan = fvid_media::plan_loudnorm(&source, args, dual, &plan_options).unwrap();
            assert_eq!(
                plan.streams[0].index,
                quiet.streams.first().copied().unwrap_or(0)
            );
            assert!(plan
                .notes
                .iter()
                .any(|n| n.contains("owned container audio decode")));
            std::fs::remove_file(baseline).unwrap();
            std::fs::remove_file(destination).unwrap();
        }
        std::fs::remove_file(reference).unwrap();
    }
}

#[test]
fn container_normalization_refusal_and_cancellation_do_not_publish() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let output = std::env::temp_dir().join(format!(
        "fvid-container-norm-cancel-{}.wav",
        std::process::id()
    ));
    let flag = fvid_control::CancelFlag::default();
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
    let error =
        fvid_media::owned_container_loudnorm::apply(&source, &output, None, false, &options)
            .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(!output.exists());
    for options in [
        CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        },
        CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    ] {
        assert!(fvid_media::owned_container_loudnorm::apply(
            &source, &output, None, true, &options
        )
        .is_err());
        assert!(!output.exists());
    }
}
