#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-subtitle-unicode")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    manifest()[key].as_array().unwrap().clone()
}
fn text(c: &serde_json::Value) -> Vec<String> {
    let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
        Cursor::new(bytes(c["file"].as_str().unwrap())),
        Default::default(),
    )
    .unwrap();
    r.cues(0).unwrap().into_iter().map(|v| v.text).collect()
}
fn expected(c: &serde_json::Value) -> Vec<String> {
    c["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}
#[test]
fn unicode_witnesses_keep_exact_packets_and_decodable_video() {
    let mut all = cases("cases");
    all.extend(cases("malformed_cue_refusals"));
    for c in all {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[1].codec, *b"tx3g");
        assert_eq!(a.tracks()[1].samples.len(), 3);
        for i in 0..3 {
            let x = a.tracks()[1].samples.get(i).unwrap();
            let y = b.tracks()[1].samples.get(i).unwrap();
            assert_eq!((x.pts, x.duration), ([0, 2, 3][i], [2, 1, 2][i]));
            assert_eq!((y.pts, y.duration), (x.pts, x.duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("{n}-{i}.packet")));
            assert_eq!(x, y);
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let f = video.read_frame().unwrap().unwrap();
            let pixels: Vec<_> = f
                .picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
#[test]
fn utf16_cues_preserve_unicode_markup_newlines_and_empty_text() {
    let mut failures = Vec::new();
    for c in cases("cases") {
        let actual = text(&c);
        let wanted = expected(&c);
        if actual != wanted {
            failures.push(format!("{}: {actual:?} != {wanted:?}", c["file"]));
        }
    }
    assert!(
        failures.is_empty(),
        "Unicode subtitle acceptance: {failures:?}"
    );
}
#[test]
fn malformed_utf16_cues_are_refused_without_losing_healthy_following_text() {
    let mut failures = Vec::new();
    for c in cases("malformed_cue_refusals") {
        let actual = text(&c);
        let wanted = expected(&c);
        if actual != wanted {
            failures.push(format!("{}: {actual:?} != {wanted:?}", c["file"]));
        }
    }
    assert!(failures.is_empty(), "Malformed cue refusal: {failures:?}");
}
#[test]
fn unicode_cue_content_and_timestamps_survive_real_video_seeks() {
    let mut failures = Vec::new();
    for c in cases("cases") {
        let n = c["file"].as_str().unwrap();
        let mut subs = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = subs.cues(0).unwrap();
        let wanted = expected(&c);
        let indices: Vec<usize> = if wanted.len() == 2 {
            vec![1, 2]
        } else {
            vec![0, 1, 2]
        };
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for (i, text) in indices.into_iter().zip(wanted) {
            let (start, end) = ([0, 40, 60][i], [40, 60, 100][i]);
            let at = Duration::from_millis(start);
            assert!(video.seek_raw(at).unwrap().is_some());
            let actual = fvid::subtitles::active(&cues, at, Duration::ZERO);
            if actual.map(|v| v.text.as_str()) != Some(text.as_str()) {
                failures.push(format!(
                    "{n} at{start}: {:?} != {text:?}",
                    actual.map(|v| v.text.as_str())
                ));
            } else {
                let cue = actual.unwrap();
                assert_eq!((cue.start, cue.end), (at, Duration::from_millis(end)));
            }
        }
    }
    assert!(failures.is_empty(), "Unicode seek playback: {failures:?}");
}
