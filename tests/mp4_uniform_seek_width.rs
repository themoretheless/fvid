use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-uniform-seek-width")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn targets(delta: i64) -> [i64; 11] {
    [
        -1,
        0,
        delta - 1,
        delta,
        2 * delta,
        3 * delta,
        1i64 << 32,
        (1i64 << 32) + delta,
        1i64 << 40,
        (1i64 << 40) + delta,
        i64::MAX,
    ]
}
#[test]
fn parsed_uniform_and_expanded_indices_seek_on_full_width_media_ticks() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let delta = c["delta"].as_i64().unwrap();
        let root_reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let owned = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(root_reader.tracks()[1].samples.len(), 3);
        assert_eq!(owned.tracks()[1].samples.len(), 3);
        assert_eq!(
            matches!(
                owned.tracks()[1].samples,
                fvid_media::owned_mp4::SampleIndex::Uniform(_)
            ),
            c["uniform"].as_bool().unwrap()
        );
        for t in targets(delta) {
            let expected = (i128::from(t).max(0) / i128::from(delta)).min(2) as usize;
            assert_eq!(
                root_reader.tracks()[1].samples.at_or_before(t),
                expected,
                "root:{n}:{t}"
            );
            assert_eq!(
                owned.tracks()[1].samples.at_or_before(t),
                expected,
                "owned:{n}:{t}"
            );
        }
        let mut pcm = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(pcm, bytes("expected.f32le"));
        let video = fvid_media::decode_video(&root().join(n)).unwrap();
        assert_eq!(video.decode_errors, 0);
        assert_eq!(video.video_frames, 3);
    }
}
#[test]
fn uniform_seek_boundaries_match_wide_integer_reference() {
    use fvid_media::owned_mp4::{SampleIndex, UniformIndex};
    for count in [0, 1, 3, 100] {
        for duration in [0, 1, 20, u32::MAX] {
            let index = SampleIndex::Uniform(UniformIndex {
                bytes_per_frame: 2,
                duration,
                count,
                runs: vec![],
            });
            for t in [
                -1,
                0,
                1,
                19,
                20,
                21,
                u32::MAX as i64,
                1i64 << 32,
                (1i64 << 32) + 20,
                1i64 << 40,
                i64::MAX,
            ] {
                let expected = if duration == 0 {
                    0
                } else {
                    (i128::from(t).max(0) / i128::from(duration))
                        .min(count.saturating_sub(1) as i128) as usize
                };
                assert_eq!(
                    index.at_or_before(t),
                    expected,
                    "count{count}:delta{duration}:target{t}"
                );
            }
        }
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
fn full_width_raw_seek_preserves_native_playback_and_precise_duration_seek() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let open = |n: &str| {
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            )
            .unwrap()
        };
        let mut control = open(name);
        let full = rational::collect(&mut control);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            bytes("expected.f32le")
        );
        let end = (3, 50);
        let mut reader = open(name);
        let last_pts = reader
            .track()
            .samples
            .get(reader.track().samples.len() - 1)
            .unwrap()
            .pts;
        for raw in [1i64 << 32, 1i64 << 40, i64::MAX] {
            assert_eq!(reader.seek(raw), last_pts, "{name}:{raw}");
        }
        reader.rewind();
        assert_eq!(
            reader.duration(),
            Some(rational::duration(end.0, end.1)),
            "{name}"
        );
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(20),
            rational::duration(end.0, end.1),
            Duration::MAX,
        ] {
            reader.seek_time(at);
            rational::exact(
                &rational::collect(&mut reader),
                &rational::suffix(&full, at, end),
                name,
            );
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
        reader.validate_eof().unwrap();
        for at in [Duration::ZERO, Duration::from_millis(20), Duration::MAX] {
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let wanted = rational::suffix(&full, at, end);
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, name);
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
        let video = fvid_media::decode_video(&root().join(name)).unwrap();
        assert_eq!(video.decode_errors, 0);
        assert_eq!(video.video_frames, 3);
    }
}
