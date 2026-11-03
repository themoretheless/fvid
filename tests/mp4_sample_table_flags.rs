use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-sample-table-flags")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn zero_flag_controls_keep_complete_valid_video_audio_and_clocks() {
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(reader.tracks().len(), 2);
        for t in reader.tracks() {
            assert_eq!(t.samples.len(), 3);
            for i in 0..3 {
                let s = t.samples.get(i).unwrap();
                assert_eq!(s.pts, (i as i64) * i64::from(s.duration));
                assert!(s.sync);
            }
        }
        let mut pcm = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(name)),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(pcm, bytes("expected.f32le"), "{name}");
        let video = fvid_media::decode_video(&root().join(name)).unwrap();
        assert_eq!(video.video_frames, 3);
        assert_eq!(video.decode_errors, 0);
    }
}
#[test]
fn reserved_sample_table_flags_have_specific_invalid_refusals() {
    let mut failures = Vec::new();
    for c in manifest()["bad"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let needle = if c["tag"] == "stz2" {
            "reserved MP4 compact sample-size fields"
        } else {
            "reserved MP4 sample-table flags"
        };
        match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => failures.push(format!("root:{name}:admitted")),
            Err(e) => {
                assert!(matches!(e, fvid::Error::Invalid(_)), "{name}:{e}");
                if !e.to_string().contains(needle) {
                    failures.push(format!("root:{name}:unrelated:{e}"));
                }
            }
        }
        match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => failures.push(format!("owned:{name}:admitted")),
            Err(e) => {
                assert!(!e.is_unsupported(), "{name}:{e}");
                if !e.to_string().contains(needle) {
                    failures.push(format!("owned:{name}:unrelated:{e}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} incorrect admissions/refusals: {failures:?}",
        failures.len()
    );
}

#[cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn zero_flag_controls_keep_actual_playback_precise_seek_rewind_and_eof() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in manifest()["cases"].as_array().unwrap() {
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
