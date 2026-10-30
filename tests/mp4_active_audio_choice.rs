#![cfg(feature = "player")]
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-active-audio-choice")
            .join(n),
    )
    .unwrap()
}
fn pcm(s: &mut dyn AudioStream) -> Vec<u8> {
    transitions::collect(s)
        .iter()
        .flat_map(|f| f.3.iter().copied())
        .collect()
}
#[test]
fn automatic_selection_uses_the_initial_selected_setup_not_the_first_table_entry() {
    let expected = bytes("expected.f32le");
    let mut failures = Vec::new();
    for n in [
        "control.mp4",
        "unused-first.mp4",
        "unused-last.mp4",
        "unused-first-compact.mp4",
    ] {
        let mut s = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let id = s.track().id;
        let got = pcm(&mut s);
        if id != 3 || got != expected {
            failures.push(format!(
                "{n}: selected track {id}, {} bytes, nonzero={}",
                got.len(),
                got.chunks_exact(4)
                    .any(|v| f32::from_le_bytes(v.try_into().unwrap()) != 0.0)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "unselected PCM setup chose the quiet alternative: {failures:?}"
    );
}
#[test]
fn explicit_selection_and_owned_decode_keep_the_analytic_preferred_waveform() {
    let expected = bytes("expected.f32le");
    for n in [
        "control.mp4",
        "unused-first.mp4",
        "unused-last.mp4",
        "unused-first-compact.mp4",
    ] {
        let mut s = Mp4AudioReader::open_at(Cursor::new(bytes(n)), Default::default(), 1).unwrap();
        assert_eq!(s.track().id, 3);
        assert!(pcm(&mut s) == expected, "{n}: explicit PCM");
        s.rewind();
        assert!(pcm(&mut s) == expected);
        let mut owned = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (8000, 1, 480)
        );
        assert!(owned == expected, "{n}: owned PCM");
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut count = 0;
        while video.read_frame_raw().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
#[test]
fn an_actually_selected_unsupported_setup_still_falls_back_and_refuses_explicit_decode() {
    let mut s =
        Mp4AudioReader::open(Cursor::new(bytes("selected-bad.mp4")), Default::default()).unwrap();
    assert_eq!(s.track().id, 2);
    assert!(pcm(&mut s).iter().all(|b| *b == 0));
    let selected = Mp4AudioReader::open_at(
        Cursor::new(bytes("selected-bad.mp4")),
        Default::default(),
        1,
    )
    .unwrap();
    let error = selected
        .make_decoder()
        .err()
        .expect("272-bit PCM decoder accepted");
    assert!(error.to_string().contains("PCM"), "{error}");
}
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn automatic_worker_plays_and_navigates_the_preferred_nonzero_track() {
    use std::time::Duration;
    let expected = bytes("expected.f32le");
    let intervals: Vec<_> = expected
        .chunks_exact(640)
        .enumerate()
        .map(|(i, b)| (Duration::from_millis(i as u64 * 20), b.to_vec()))
        .collect();
    for n in ["unused-first.mp4", "unused-first-compact.mp4"] {
        let s = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        assert_eq!(s.track().id, 3);
        capture::worker_check(Box::new(s), &intervals);
    }
}
