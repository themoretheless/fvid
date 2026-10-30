use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unedited-audio-tail")
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
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn owned_unedited_tail_is_real_silence_after_all_codec_output() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let track = &reader.tracks()[1];
        assert!(track.edits.is_empty());
        assert_eq!(track.samples.len(), 3);
        let step = c["step"].as_u64().unwrap();
        assert_eq!(track.duration, 4 * step);
        for i in 0..3 {
            let sample = track.samples.get(i).unwrap();
            assert_eq!(
                (sample.dts, sample.pts, sample.duration),
                (i as u64 * step, (i as u64 * step) as i64, step as u32)
            );
        }
        let video = fvid_media::decode_video(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/mp4-unedited-audio-tail")
                .join(n),
        )
        .unwrap();
        assert_eq!(video.video_frames, c["video_frames"].as_u64().unwrap());
        assert_eq!(video.decode_errors, 0);
        let mut out = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap(), "{n}");
        compare(
            &out,
            &bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap())),
            &c,
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn native_unedited_tail_matches_owned_pcm_and_declared_duration() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let packets = transitions::collect(&mut r);
        let out: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
        let expected = bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap()));
        if out.len() != expected.len() {
            failures.push(format!(
                "{n}: {} instead of{} bytes",
                out.len(),
                expected.len()
            ));
        } else {
            compare(&out, &expected, &c);
        }
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(
                c["frames"].as_u64().unwrap() * 1_000_000_000 / c["rate"].as_u64().unwrap()
            ))
        );
    }
    assert!(failures.is_empty(), "unedited tail lost: {failures:?}");
}

fn compare(actual: &[u8], expected: &[u8], c: &serde_json::Value) {
    assert_eq!(actual.len(), expected.len(), "{}: PCM length", c["file"]);
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            a.is_finite() && (a - b).abs() <= 1e-6,
            "{}: PCM {a} != {b}",
            c["file"]
        );
    }
    let begin =
        (3 * c["packet_frames"].as_u64().unwrap() * c["channels"].as_u64().unwrap() * 4) as usize;
    assert!(
        actual[begin..].iter().all(|b| *b == 0),
        "{}: tail must be literal silence",
        c["file"]
    );
}

#[cfg(feature = "player")]
#[test]
fn unedited_tail_seek_rewind_and_workers_keep_exact_silence() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    use std::sync::{Arc, Mutex};
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap();
        let frames = c["frames"].as_u64().unwrap();
        let offset = c["packet_frames"].as_u64().unwrap() * 7 / 2;
        let from = Duration::from_nanos(offset * 1_000_000_000 / rate);
        let end = Duration::from_nanos(frames * 1_000_000_000 / rate);
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        r.seek_time(from);
        let suffix = transitions::collect(&mut r);
        let out: Vec<_> = suffix.iter().flat_map(|p| p.3.iter().copied()).collect();
        assert_eq!(
            out,
            vec![0; ((frames - offset) * channels * 4) as usize],
            "{n}: tail seek"
        );
        assert_eq!(suffix[0].2, from);
        r.seek_time(end);
        assert!(transitions::collect(&mut r).is_empty());
        r.rewind();
        let out: Vec<_> = transitions::collect(&mut r)
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        compare(
            &out,
            &bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap())),
            &c,
        );
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let reader = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let mut audio = fvid::audio_thread::AudioPlayback::start(Box::new(reader), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        let out: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        compare(
            &out,
            &bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap())),
            &c,
        );
        trace.lock().unwrap().packets.clear();
        audio.seek(from);
        audio.play();
        transitions::ended(&audio, audio.generation());
        let packets = trace.lock().unwrap().packets.clone();
        let out: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
        assert_eq!(
            out,
            vec![0; ((frames - offset) * channels * 4) as usize],
            "{n}: worker seek"
        );
        assert_eq!(packets[0].2, from);
    }
}

#[cfg(feature = "player")]
#[test]
fn a_two_day_unedited_tail_seeks_without_materializing_its_pcm() {
    use fvid::{
        audio::{AudioStep, AudioStream},
        playback_mp4_audio::Mp4AudioReader,
    };
    let n = "pcm-two-day-tail.mp4";
    assert!(bytes(n).len() < 64 * 1024);
    let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    assert_eq!(
        r.duration(),
        Some(Duration::from_secs(2 * 24 * 3600) + Duration::from_millis(60))
    );
    let from = Duration::from_secs(24 * 3600);
    r.seek_time(from);
    let mut produced = false;
    for _ in 0..4 {
        match r.next_step().unwrap() {
            Some(AudioStep::ReconfigureDecoder) => r.advance_configuration().unwrap(),
            Some(AudioStep::Pcm(p)) => {
                assert_eq!(p.presentation_time(), from);
                assert!(!p.data.is_empty() && p.data.len() <= 4096);
                assert!(p.data.iter().all(|b| *b == 0));
                produced = true;
                break;
            }
            _ => panic!("tail seek must not replay encoded source"),
        }
    }
    assert!(produced);
    let mut out = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(n)),
        &mut out,
        Some((from, from + Duration::from_millis(10))),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 80);
    assert_eq!(out, vec![0; 320]);
    r.seek_time(r.duration().unwrap());
    assert!(transitions::collect(&mut r).is_empty());
}
