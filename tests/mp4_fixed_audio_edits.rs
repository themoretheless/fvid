#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, time::Duration};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-open-ended-edit")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest-audio.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["codec"] != "aac" && c["open"] == false)
        .cloned()
        .collect()
}
fn open(c: &serde_json::Value) -> fvid::playback_mp4_audio::Mp4AudioReader<Cursor<Vec<u8>>> {
    fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(bytes(c["file"].as_str().unwrap())),
        Default::default(),
    )
    .unwrap()
}
fn compare(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() <= 1e-6, "{a} != {b}");
    }
}
#[test]
fn finite_fixed_audio_edits_set_the_native_duration_and_trim_window() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = open(&c);
        let wanted = Duration::from_nanos(
            c["frames"].as_u64().unwrap() * 1_000_000_000 / c["rate"].as_u64().unwrap(),
        );
        if r.duration() != Some(wanted) {
            failures.push(format!("{n}: {:?} != {wanted:?}", r.duration()));
            continue;
        }
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        let expected = bytes(c["oracle"].as_str().unwrap());
        let at = c["start_frames"].as_u64().unwrap() as usize * 4;
        compare(&pcm, &expected[at..]);
        assert!(transitions::collect(&mut r).is_empty());
        AudioStream::rewind(&mut r);
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|f| f.3)
            .collect();
        compare(&pcm, &expected[at..]);
    }
    assert!(
        failures.is_empty(),
        "finite media edits ignored: {failures:?}"
    );
}
#[test]
fn finite_fixed_audio_seek_respects_the_media_trim() {
    for c in cases()
        .into_iter()
        .filter(|c| c["start_frames"].as_u64().unwrap() != 0)
    {
        let mut r = open(&c);
        r.seek_time(Duration::from_millis(10));
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_millis(10));
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        let rate = c["rate"].as_u64().unwrap();
        let at = (c["start_frames"].as_u64().unwrap() + rate / 100) as usize * 4;
        compare(&pcm, &bytes(c["oracle"].as_str().unwrap())[at..]);
    }
}

#[test]
fn finite_fractional_pcm_and_alac_edits_keep_exact_duration_and_sample_positions() {
    let doc: serde_json::Value =
        serde_json::from_slice(&bytes("manifest-fractional.json")).unwrap();
    for c in doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["open"] == false)
    {
        let mut r = open(c);
        let exact = Duration::from_nanos(2645u64 * 1_000_000_000 / 44100);
        assert_eq!(r.duration(), Some(exact), "{}", c["file"]);
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_nanos(102_324));
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        compare(&pcm, &bytes(c["oracle"].as_str().unwrap())[4..]);
        AudioStream::rewind(&mut r);
        r.seek_time(Duration::from_millis(10));
        let frames = transitions::collect(&mut r);
        assert_eq!(frames.first().unwrap().2, Duration::from_nanos(10_102_324));
        let pcm: Vec<_> = frames.into_iter().flat_map(|f| f.3).collect();
        compare(&pcm, &bytes(c["oracle"].as_str().unwrap())[81 * 4..]);
    }
}
