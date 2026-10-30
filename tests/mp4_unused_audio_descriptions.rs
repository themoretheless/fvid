use std::io::Cursor;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unused-audio-descriptions")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn unused_opaque_audio_entries_preserve_tracks_packet_selection_and_clocks() {
    let mut missing = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let mut b = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let mut control = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        if a.tracks().len() != 2 || b.tracks().len() != 2 {
            missing.push(n.to_owned());
            continue;
        }
        if n.contains("-compact-") {
            assert!(matches!(
                &a.tracks()[1].samples,
                fvid::container::mp4::SampleIndex::Uniform(_)
            ));
            assert!(matches!(
                &b.tracks()[1].samples,
                fvid_media::owned_mp4::SampleIndex::Uniform(_)
            ));
        }
        assert_eq!(
            a.tracks()[1].sample_descriptions[c["insert"].as_u64().unwrap() as usize].codec,
            *b"zzzz"
        );
        for i in 0..6 {
            let s = a.tracks()[1].samples.get(i).unwrap();
            let t = b.tracks()[1].samples.get(i).unwrap();
            let u = control.tracks()[1].samples.get(i).unwrap();
            assert_eq!((s.dts, s.pts, s.duration), (u.dts, u.pts, u.duration));
            assert_eq!((t.dts, t.pts, t.duration), (u.dts, u.pts, u.duration));
            assert_eq!(
                a.tracks()[1].description_for_sample(i),
                control.tracks()[1].description_for_sample(i)
            );
            let (mut x, mut y, mut z) = (Vec::new(), Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            control.read_packet(1, i, &mut z).unwrap();
            assert_eq!(x, z);
            assert_eq!(y, z);
        }
    }
    assert!(
        missing.is_empty(),
        "unused audio entry silently dropped the audio track: {missing:?}"
    );
}
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn unused_audio_entries_play_exact_pcm_transitions_rewind_and_seek() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    use std::time::Duration;
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut stream = match Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{n}: {e}"));
                continue;
            }
        };
        let wanted: transitions::Frames = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["rate"].as_u64().unwrap() as u32,
                    f["channels"].as_u64().unwrap() as u16,
                    Duration::from_micros(f["pts"].as_u64().unwrap()),
                    bytes(f["oracle"].as_str().unwrap()),
                )
            })
            .collect();
        transitions::compare(&transitions::collect(&mut stream), &wanted);
        stream.rewind();
        transitions::compare(&transitions::collect(&mut stream), &wanted);
        // Constant legacy PCM seeks at packet boundaries; changing geometry
        // supports exact within-packet windows. Exercise each public contract.
        let target = if n.starts_with("constant-") {
            40000
        } else {
            50000
        };
        assert_eq!(stream.seek_to(target), target);
        let mut expected = wanted.clone();
        expected.drain(..2);
        expected[0].2 = Duration::from_micros(target as u64);
        let skip = ((target - 40000) as u64 * u64::from(expected[0].0) / 1_000_000) as usize
            * expected[0].1 as usize
            * 4;
        expected[0].3 = expected[0].3[skip..].to_vec();
        transitions::compare(&transitions::collect(&mut stream), &expected);
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut count = 0;
        while v.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 6);
    }
    assert!(
        failures.is_empty(),
        "unused metadata prevented audio playback: {failures:?}"
    );
}
#[cfg(feature = "player")]
#[test]
fn an_actually_selected_unknown_entry_is_refused_as_a_capability() {
    match fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(bytes("referenced-unknown.mp4")),
        Default::default(),
    ) {
        Err(fvid::Error::Unsupported(_)) => {}
        Err(e) => panic!("wrong refusal: {e}"),
        Ok(_) => panic!("unknown audio was decoded as PCM"),
    }
}
#[test]
fn owned_audio_decode_keeps_the_control_waveform_with_unused_metadata() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut expected = Vec::new();
        let control = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(
            expected
                .chunks_exact(4)
                .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 1e-4)
        );
        let mut actual = Vec::new();
        match fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut actual,
            None,
            &Default::default(),
        ) {
            Ok(stats) => {
                assert_eq!(
                    (stats.sample_rate, stats.channels, stats.sample_frames),
                    (control.sample_rate, control.channels, control.sample_frames)
                );
                assert_eq!(actual, expected, "{n}");
            }
            Err(e) => failures.push(format!("{n}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "unused audio metadata prevented owned decode: {failures:?}"
    );
}

#[test]
fn unselected_unknown_entries_still_validate_their_common_data_reference() {
    let a = fvid::container::mp4::Mp4Reader::open_indexed(
        Cursor::new(bytes("unreferenced-external.mp4")),
        Default::default(),
    )
    .err()
    .expect("external reference ignored");
    let b = fvid_media::owned_mp4::Mp4Reader::open_indexed(
        Cursor::new(bytes("unreferenced-external.mp4")),
        Default::default(),
    )
    .err()
    .expect("external reference ignored");
    assert!(a.to_string().contains("invalid sample data reference"));
    assert!(b.to_string().contains("invalid sample data reference"));
}
#[cfg(feature = "player")]
#[test]
fn the_audio_worker_plays_and_rewinds_with_unused_first_entries() {
    use fvid::{audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
    use std::{
        sync::{Arc, Mutex, atomic::AtomicUsize},
        time::Duration,
    };
    for name in [
        "changing-fragmented-insert-0.mp4",
        "constant-compact-insert-0.mp4",
    ] {
        let c = cases()
            .into_iter()
            .find(|c| c["file"].as_str() == Some(name))
            .unwrap();
        let wanted: transitions::Frames = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["rate"].as_u64().unwrap() as u32,
                    f["channels"].as_u64().unwrap() as u16,
                    Duration::from_micros(f["pts"].as_u64().unwrap()),
                    bytes(f["oracle"].as_str().unwrap()),
                )
            })
            .collect();
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let shared = trace.clone();
        let stream = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(stream), move || {
            Box::new(transitions::Capture {
                trace: shared,
                spec: None,
                at: Duration::ZERO,
                remaining: AtomicUsize::new(0),
                draining: false,
                fail_next: None,
                gate: None,
                paused: None,
            })
        });
        audio.play();
        transitions::ended(&audio, 0);
        transitions::compare(&trace.lock().unwrap().packets, &wanted);
        audio.rewind();
        transitions::ended(&audio, audio.generation());
        transitions::compare(&trace.lock().unwrap().packets, &wanted);
    }
}
