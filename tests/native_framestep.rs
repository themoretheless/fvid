use fvid::media::{DecodeTransform, decode_video_transformed};
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn owned_framestep_selects_native_codec_outputs_after_interval() {
    for name in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm",
        "av1/ramp.webm",
        "playback-errors/shuffleplanes-444-8.mkv",
        "playback-errors/framestep-six-frames.y4m",
    ] {
        let source = fixture(name);
        let mut reader = fvid::playback_native::NativeReader::software(
            std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
            usize::MAX,
        )
        .unwrap();
        reader.read_frame_raw().unwrap().unwrap();
        let (start, _, scale) = reader.frame_interval().unwrap();
        let end = i64::try_from(start * 1_000_000 / u128::from(scale)).unwrap() + 1_000_000;
        for interval in [None, Some((0, end))] {
            let full = decode_video_transformed(
                &source,
                DecodeTransform {
                    interval,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|error| panic!("{name} {interval:?}: {error}"));
            for step in [1u64, 2, 3, 100] {
                let actual = decode_video_transformed(
                    &source,
                    DecodeTransform {
                        interval,
                        framestep: Some(format!("step={step}")),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(actual.backend, "fvid");
                assert_eq!(
                    actual.video_frames,
                    full.video_frames.div_ceil(step),
                    "{name}"
                );
                assert_eq!((actual.width, actual.height), (full.width, full.height));
                assert_eq!(actual.pixel_format, full.pixel_format);
            }
        }
    }
    let source = fixture("playback-errors/framestep-six-frames.y4m");
    let actual = decode_video_transformed(
        &source,
        DecodeTransform {
            interval: Some((250_000, 1_000_000)),
            framestep: Some("2".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(actual.video_frames, 2);
}

#[test]
fn dropped_frames_still_validate_storage_and_invalid_options_are_refused() {
    let source = fixture("playback-errors/framestep-discarded-truncated.y4m");
    let error = decode_video_transformed(
        &source,
        DecodeTransform {
            framestep: Some("2".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error.to_lowercase().contains("truncated") || error.contains("failed to fill whole buffer"),
        "{error}"
    );
    for invalid in ["0", "-1", "2147483648", "step=n"] {
        assert!(
            decode_video_transformed(
                &fixture("playback-errors/ffv1-level-one-source.mp4"),
                DecodeTransform {
                    framestep: Some(invalid.into()),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn framestep_cli_decodes_avc_without_legacy_feature() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"])
        .arg(fixture("playback-errors/ffv1-level-one-source.mp4"))
        .args(["--framestep", "3"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["backend"], "fvid");
    assert_eq!(result["video_frames"], 4);
}

#[test]
fn mp4_framestep_exports_selected_times_and_every_audio_packet() {
    use fvid::{
        container::webm::WebmReader,
        media::{CopyOptions, LosslessTransform},
    };
    use std::io::Cursor;
    let dir = std::env::temp_dir().join(format!("fvid-step-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (index, name) in [
        "playback-errors/ffv1-level-one-source.mp4",
        "hevc/main10-ipb.mp4",
        "audio/two-audio.mp4",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let baseline = dir.join(format!("base-{index}.mkv"));
        let selected = dir.join(format!("step-{index}.mkv"));
        let options = CopyOptions::default();
        let base_stats = fvid::media::transcode_lossless(
            &source,
            &baseline,
            LosslessTransform::default(),
            &options,
        )
        .unwrap();
        let transform = LosslessTransform {
            framestep: Some("3".into()),
            ..Default::default()
        };
        let plan =
            fvid::media::plan_transcode_lossless(&source, &transform, &options, None).unwrap();
        assert!(plan.notes.iter().any(|n| n.contains("framestep")));
        let settings = fvid::media::EncoderSettings {
            name: "ffv1".into(),
            options: vec![("level".into(), "1".into())],
        };
        let stats =
            fvid::media::transcode(&source, &selected, transform, &options, &settings).unwrap();
        assert_eq!(stats.decoded_frames, base_stats.decoded_frames);
        assert_eq!(stats.video_frames, base_stats.video_frames.div_ceil(3));
        let mut base = WebmReader::open(
            Cursor::new(std::fs::read(&baseline).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut step = WebmReader::open(
            Cursor::new(std::fs::read(&selected).unwrap()),
            Default::default(),
        )
        .unwrap();
        base.scan_all().unwrap();
        step.scan_all().unwrap();
        for track in 0..base.tracks.len() {
            let is_video = base.tracks[track].codec == "V_FFV1";
            let expected: Vec<_> = base
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == track as u64 + 1)
                .enumerate()
                .filter(|(i, _)| !is_video || i % 3 == 0)
                .map(|(_, (i, _))| i)
                .collect();
            let actual: Vec<_> = step
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == track as u64 + 1)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.into_iter().zip(expected) {
                assert_eq!(step.packets[a].pts_ns, base.packets[b].pts_ns);
                assert_eq!(step.packets[a].duration_ns, base.packets[b].duration_ns);
                assert_eq!(step.read_packet(a).unwrap(), base.read_packet(b).unwrap());
            }
        }
    }
    let destination = dir.join("cli.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(fixture("playback-errors/ffv1-level-one-source.mp4"))
        .arg(destination)
        .args(["--framestep", "3"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["video_frames"],
        4
    );
    std::fs::remove_dir_all(dir).unwrap();
}
