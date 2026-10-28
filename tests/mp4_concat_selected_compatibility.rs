use fvid::{container::mp4_concat, media::CopyOptions};
use std::path::PathBuf;
fn sources() -> [PathBuf; 2] {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-selected-compatibility");
    [
        dir.join("video-audio-48000.mp4"),
        dir.join("video-audio-44100.mp4"),
    ]
}
#[test]
fn discarded_aac_configuration_does_not_reject_identical_video() {
    let options = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    assert!(
        mp4_concat::open_with_options(&sources(), &options)
            .unwrap()
            .is_some(),
        "discarded AAC clocks must not participate in selected video compatibility"
    );
}
#[test]
fn incompatible_audio_is_still_refused_when_selected() {
    for streams in [vec![], vec![1], vec![0, 1]] {
        let options = CopyOptions {
            streams,
            ..Default::default()
        };
        assert!(
            mp4_concat::open_with_options(&sources(), &options)
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn each_audio_clock_is_supported_with_matching_segments() {
    for source in sources() {
        let options = CopyOptions {
            streams: vec![0],
            ..Default::default()
        };
        assert!(
            mp4_concat::open_with_options(&[source.clone(), source], &options)
                .unwrap()
                .is_some()
        );
    }
}
#[test]
fn discarded_track_presence_does_not_reject_identical_video() {
    let source = sources()[0].clone();
    let video_only = source.parent().unwrap().join("video-only.mp4");
    for inputs in [[source.clone(), video_only.clone()], [video_only, source]] {
        let options = CopyOptions {
            streams: vec![0],
            ..Default::default()
        };
        assert!(
            mp4_concat::open_with_options(&inputs, &options)
                .unwrap()
                .is_some()
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn selected_video_with_differing_discarded_audio_plays_and_seeks() {
    use fvid::playback_native::NativeReader;
    use std::{
        fs::File,
        io::{BufReader, Cursor},
        time::Duration,
    };
    let sources = sources();
    let mut reference =
        NativeReader::software(BufReader::new(File::open(&sources[0]).unwrap()), 32 << 20).unwrap();
    let expected = reference
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    let options = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    for second in [
        sources[1].clone(),
        sources[0].parent().unwrap().join("video-only.mp4"),
    ] {
        for inputs in [
            [sources[0].clone(), second.clone()],
            [second, sources[0].clone()],
        ] {
            let mut segments = mp4_concat::open_with_options(&inputs, &options)
                .unwrap()
                .unwrap();
            let mut output = Cursor::new(Vec::new());
            assert_eq!(
                mp4_concat::write_with_options(&mut segments, &mut output, &options)
                    .unwrap()
                    .packets,
                2
            );
            let mut video =
                NativeReader::software(BufReader::new(Cursor::new(output.into_inner())), 32 << 20)
                    .unwrap();
            assert_eq!(video.duration(), Some(Duration::from_millis(80)));
            for _ in 0..2 {
                assert_eq!(
                    video
                        .read_frame_raw()
                        .unwrap()
                        .unwrap()
                        .into_rgb(32 << 20)
                        .unwrap(),
                    expected
                );
            }
            assert!(video.read_frame_raw().unwrap().is_none());
            assert!(video.read_frame_raw().unwrap().is_none());
            video.rewind().unwrap();
            assert_eq!(
                video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                expected
            );
            for ms in [0, 39, 40, 79, 80, 100] {
                assert_eq!(
                    video
                        .seek_raw(Duration::from_millis(ms))
                        .unwrap()
                        .unwrap()
                        .into_rgb(32 << 20)
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
#[test]
fn public_plan_and_concat_keep_only_the_selected_video() {
    use fvid::container::webm::WebmReader;
    use std::{fs::File, io::BufReader};
    let options = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    let inputs = sources();
    let plan = fvid::media::plan_concat(&inputs, &options).unwrap();
    assert_eq!(plan.command, "concat");
    assert_eq!(plan.streams.len(), 1);
    let dir = std::env::temp_dir().join(format!(
        "fvid-selected-compatibility-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let output = dir.join("video.mkv");
    let stats = fvid::media::concat(&inputs, &output, &options).unwrap();
    assert_eq!(stats.packets, 2);
    assert_eq!(stats.segments, 2);
    let mut reader = WebmReader::open(
        BufReader::new(File::open(output).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 1);
    assert_eq!(reader.packets.len(), 2);
    assert_eq!(reader.packets[1].pts_ns, 40_000_000);
}
