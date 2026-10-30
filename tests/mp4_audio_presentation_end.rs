#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-audio-presentation-end")
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
fn audio_duration_matches_the_composition_endpoint_and_retains_empty_tails() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let expected = Some(Duration::from_nanos(c["end_ns"].as_u64().unwrap()));
        for phase in 0..3 {
            if s.duration() != expected {
                failures.push(format!(
                    "{n} phase{phase}: {:?} vs {expected:?}",
                    s.duration()
                ));
            }
            if phase == 0 {
                let _ = transitions::collect(&mut s);
            } else {
                s.rewind();
            }
        }
    }
    assert!(
        failures.is_empty(),
        "decode clock reported as presentation length: {failures:?}"
    );
}
#[test]
fn exact_pcm_timestamps_seek_rewind_and_video_remain_intact() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        for _ in 0..2 {
            let got = transitions::collect(&mut s);
            let actual: Vec<_> = got.iter().flat_map(|f| f.3.iter().copied()).collect();
            let expected_pcm = bytes(c["oracle"].as_str().unwrap());
            assert_eq!(
                actual.len(),
                expected_pcm.len(),
                "{n}: exact decoded PCM byte count"
            );
            assert_eq!(actual, expected_pcm, "{n}");
            for (i, f) in got.iter().enumerate() {
                assert_eq!((f.0, f.1), (8000, 1));
                let scale = i128::from(c["clock"].as_i64().unwrap());
                let numerator = i128::from(c["shift_ticks"].as_i64().unwrap()) * 1_000_000_000
                    + i as i128 * 20_000_000 * scale;
                let dropped = if numerator < 0 {
                    (-numerator * 8000 + scale * 1_000_000_000 - 1) / (scale * 1_000_000_000)
                } else {
                    0
                };
                let ns = (numerator * 8000 + dropped * scale * 1_000_000_000) / (scale * 8000);
                assert_eq!(f.2, Duration::from_nanos(ns as u64), "{n}");
            }
            s.rewind();
        }
        let target = c["shift_ticks"].as_i64().unwrap() + c["clock"].as_i64().unwrap() / 50;
        assert_eq!(s.seek_to(target), target);
        let got = transitions::collect(&mut s);
        let full = if c["silent_tail_frames"].as_u64().unwrap_or(0)>0 {
            bytes("full-tail.f32le")
        } else {bytes("full.f32le")};
        assert_eq!(
            got.iter()
                .flat_map(|f| f.3.iter().copied())
                .collect::<Vec<_>>(),
            full[160 * 4..],
            "{n}: seek suffix"
        );
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut count = 0;
        while v.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
#[test]
fn the_audio_worker_keeps_the_complete_shifted_tail() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex, atomic::AtomicUsize};
    for name in ["pcm-1001-shift10-known.mp4", "alac-1001-shift10-known.mp4"] {
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
        for pass in 0..2 {
            let frames = trace.lock().unwrap().packets.clone();
            assert!(frames.iter().all(|f| f.0 == 8000 && f.1 == 1));
            let actual: Vec<_> = frames.iter().flat_map(|f| f.3.iter().copied()).collect();
            let expected = bytes("full.f32le");
            assert_eq!(actual.len(), expected.len(), "{name}: worker pass {pass}");
            assert!(actual == expected, "{name}: exact worker PCM differs");
            if pass == 0 {
                audio.rewind();
                transitions::ended(&audio, audio.generation());
            }
        }
    }
}
