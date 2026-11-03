use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-admission-catalog")
}
#[test]
fn prefix_queries_keep_precise_pcm_and_strict_packet_memory_cancellation_controls() {
    for name in [
        "catalog1-selected1.mp4",
        "catalog32-selected1.mp4",
        "catalog32-selected32.mp4",
    ] {
        let source = root().join(name);
        for n in [1, 257, 8192] {
            let opts = fvid_media::CopyOptions {
                max_packets: Some(n),
                max_packet_bytes: 2,
                max_controlled_bytes: Some(64 << 20),
                ..Default::default()
            };
            fvid_media::plan_decode_audio(&source, &Default::default(), &opts).unwrap();
            let mut out = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::fs::File::open(&source).unwrap(),
                &mut out,
                None,
                &opts,
            )
            .unwrap();
            assert_eq!(stats.sample_frames, n);
            assert_eq!(out, 0.125f32.to_le_bytes().repeat(n as usize));
            out.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                std::fs::File::open(&source).unwrap(),
                &mut out,
                None,
                &opts,
            )
            .unwrap();
            assert_eq!(out, 0.125f64.to_le_bytes().repeat(n as usize));
            let tight = fvid_media::CopyOptions {
                max_packet_bytes: 1,
                ..opts
            };
            let mut out = Vec::new();
            let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::fs::File::open(&source).unwrap(),
                &mut out,
                None,
                &tight,
            )
            .unwrap_err()
            .to_string();
            assert!(e.contains("packet"), "{e}");
            assert!(out.is_empty());
        }
        let tiny = fvid_media::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(fvid_media::plan_decode_audio(&source, &Default::default(), &tiny).is_err());
        let cancel = fvid::media_control::CancelFlag::new();
        cancel.cancel();
        let opts = fvid_media::CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        };
        assert!(
            fvid_media::plan_decode_audio(&source, &Default::default(), &opts)
                .unwrap_err()
                .contains("cancel")
        );
    }
}
#[cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn actual_compact_pcm_playback_workers_seek_and_software_video_are_exact() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    let name = "catalog32-selected32.mp4";
    let input = std::fs::read(root().join(name)).unwrap();
    let open = || {
        fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(input.clone()),
            Default::default(),
        )
        .unwrap()
    };
    let mut reader = open();
    let full = rational::collect(&mut reader);
    assert_eq!(
        full.iter()
            .flat_map(|p| p.data.iter().copied())
            .collect::<Vec<_>>(),
        std::fs::read(root().join("expected.f32le")).unwrap()
    );
    for at in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_millis(100),
        Duration::MAX,
    ] {
        reader.seek_time(at);
        rational::exact(
            &rational::collect(&mut reader),
            &rational::suffix(&full, at, (16384, 48000)),
            name,
        );
    }
    reader.rewind();
    rational::exact(&rational::collect(&mut reader), &full, name);
    reader.validate_eof().unwrap();
    for at in [Duration::ZERO, Duration::from_millis(100), Duration::MAX] {
        let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
        let copy = raw.clone();
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let mut worker = fvid::audio_thread::AudioPlayback::start(Box::new(open()), move || {
            Box::new(rational::RawCapture {
                inner: transitions::Capture::new(trace),
                raw: copy,
            })
        });
        worker.seek(at);
        worker.play();
        transitions::ended(&worker, worker.generation());
        let wanted = rational::suffix(&full, at, (16384, 48000));
        let got = raw.lock().unwrap();
        rational::exact(&got.packets, &wanted, name);
        assert_eq!(got.anchors, vec![rational::anchor(&wanted, (16384, 48000))]);
    }
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(input), 32 << 20).unwrap();
    for _ in 0..3 {
        assert_eq!(
            video
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            vec![130; 16 * 16 * 3]
        );
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    video.rewind().unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.seek_raw(Duration::from_millis(25)).unwrap().is_some());
}
