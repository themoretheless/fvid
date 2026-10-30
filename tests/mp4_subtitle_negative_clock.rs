#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-subtitle-negative-clock")
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
fn subtitle_clock_witnesses_preserve_packets_signed_pts_and_actual_video() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[1].codec, *b"tx3g");
        assert!(a.tracks()[1].edits.is_empty());
        assert_eq!(a.tracks()[1].samples.len(), 3);
        for i in 0..3 {
            let x = a.tracks()[1].samples.get(i).unwrap();
            let y = b.tracks()[1].samples.get(i).unwrap();
            let pts = c["pts"][i].as_i64().unwrap();
            let duration = c["durations"][i].as_u64().unwrap() as u32;
            assert_eq!((x.pts, x.duration), (pts, duration));
            assert_eq!((y.pts, y.duration), (pts, duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, y);
            let text = c["texts"][i].as_str().unwrap();
            assert_eq!(&x[2..], text.as_bytes());
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let frame = video.read_frame().unwrap().unwrap();
            assert_eq!(
                (frame.presentation_time.ticks, frame.duration.ticks),
                (i * 2, 2)
            );
            let pixels: Vec<_> = frame
                .picture
                .y
                .iter()
                .chain(&frame.picture.cb)
                .chain(&frame.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
#[test]
fn subtitle_cues_clip_at_zero_and_keep_the_absolute_composition_endpoint() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut reader = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let actual: Vec<_> = reader
            .cues(0)
            .unwrap()
            .iter()
            .map(|v| (v.text.clone(), v.start.as_nanos(), v.end.as_nanos()))
            .collect();
        let expected: Vec<_> = c["cues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                (
                    v["text"].as_str().unwrap().to_owned(),
                    u128::from(v["start_ns"].as_u64().unwrap()),
                    u128::from(v["end_ns"].as_u64().unwrap()),
                )
            })
            .collect();
        if actual != expected {
            failures.push(format!("{n}: {actual:?} != {expected:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "subtitle composition spans: {failures:?}"
    );
}
#[test]
fn subtitle_lookup_follows_the_video_clock_before_and_after_seek() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut subs = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = subs.cues(0).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let mut times = vec![
            0,
            20_000_000,
            40_000_000,
            60_000_000,
            80_000_000,
            100_000_000,
        ];
        for cue in c["cues"].as_array().unwrap() {
            let end = cue["end_ns"].as_u64().unwrap();
            times.extend([end.saturating_sub(1), end]);
        }
        for ns in times.into_iter().filter(|n| *n < 120_000_000) {
            let at = Duration::from_nanos(ns);
            assert!(video.seek_raw(at).unwrap().is_some());
            let expected = c["cues"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| {
                    v["start_ns"].as_u64().unwrap() <= ns && ns < v["end_ns"].as_u64().unwrap()
                })
                .map(|v| v["text"].as_str().unwrap());
            let actual =
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|v| v.text.as_str());
            if actual != expected {
                failures.push(format!("{n} at{ns}: {actual:?} != {expected:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "subtitle lookup: {failures:?}");
}
