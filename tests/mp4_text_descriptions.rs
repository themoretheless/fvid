#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-text-descriptions")
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
fn expected(c: &serde_json::Value) -> Vec<(String, u128, u128)> {
    c["texts"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            (
                t.as_str().unwrap().to_owned(),
                i as u128 * 20_000_000,
                (i + 1) as u128 * 20_000_000,
            )
        })
        .collect()
}
#[test]
fn video_and_plain_text_controls_decode_with_original_headers_and_pixels() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let vi = a.tracks().iter().position(|t| t.id == 1).unwrap();
        assert_eq!(a.track_header_duration(vi), Some(3));
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(n)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        assert_eq!(video.track().id, 1, "{n} enabled video preference");
        for i in 0..3 {
            let f = video.read_frame().unwrap().unwrap();
            assert_eq!((f.presentation_time.ticks, f.duration.ticks), (i, 1));
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
        if c["variant"] == "plain" {
            let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            )
            .unwrap();
            let actual: Vec<_> = r
                .cues(0)
                .unwrap()
                .into_iter()
                .map(|v| (v.text, v.start.as_nanos(), v.end.as_nanos()))
                .collect();
            assert_eq!(actual, expected(&c));
        }
    }
}
#[test]
fn dormant_unknown_entries_preserve_referenced_tx3g_descriptions_and_bytes() {
    for c in cases()
        .into_iter()
        .filter(|c| c["available"] == true && c["variant"] != "plain")
    {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let ai = a
            .tracks()
            .iter()
            .position(|t| t.id == 2)
            .expect("unused unknown description must not hide the text track");
        let bi = b
            .tracks()
            .iter()
            .position(|t| t.id == 2)
            .expect("owned parser must retain text");
        assert_eq!(a.tracks()[ai].codec, *b"tx3g");
        assert_eq!(a.tracks()[ai].sample_descriptions.len(), 2);
        assert_eq!(b.tracks()[bi].sample_descriptions.len(), 2);
        assert!(a.refused().is_empty());
        assert!(b.refused().is_empty());
        for i in 0..3 {
            let x = a.tracks()[ai].samples.get(i).unwrap();
            let y = b.tracks()[bi].samples.get(i).unwrap();
            assert_eq!(
                (x.pts, x.duration, x.description_index),
                (i as i64, 1, c["indices"][i].as_u64().unwrap() as u32)
            );
            assert_eq!(
                (y.pts, y.duration, y.description_index),
                (x.pts, x.duration, x.description_index)
            );
            assert_eq!(
                a.tracks()[ai].description_for_sample(i).unwrap().codec,
                *b"tx3g"
            );
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(ai, i, &mut x).unwrap();
            b.read_packet(bi, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("tx3g-{i}.packet")));
            assert_eq!(x, y);
        }
    }
}
#[test]
fn actual_video_seeks_keep_captions_from_the_active_description() {
    for c in cases().into_iter().filter(|c| c["available"] == true) {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(r.tracks().len(), 1, "{n}");
        assert_eq!(r.tracks()[0].codec, *b"tx3g");
        let cues = r.cues(0).unwrap();
        let actual: Vec<_> = cues
            .iter()
            .map(|v| (v.text.clone(), v.start.as_nanos(), v.end.as_nanos()))
            .collect();
        assert_eq!(actual, expected(&c));
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for ns in [
            0, 19_999_999, 20_000_000, 39_999_999, 40_000_000, 59_999_999,
        ] {
            let at = Duration::from_nanos(ns);
            assert!(v.seek_raw(at).unwrap().is_some());
            let (s, e, scale) = v.frame_interval().unwrap();
            let frame = u128::from(ns / 20_000_000);
            assert_eq!(
                (
                    s * 1_000_000_000 / u128::from(scale),
                    e * 1_000_000_000 / u128::from(scale)
                ),
                (frame * 20_000_000, (frame + 1) * 20_000_000)
            );
            assert_eq!(
                fvid::subtitles::active(&cues, at, Duration::ZERO)
                    .unwrap()
                    .text,
                c["texts"][(ns / 20_000_000) as usize].as_str().unwrap()
            );
        }
    }
}
#[test]
fn referenced_unknown_descriptions_are_explicitly_refused_without_partial_text() {
    for c in cases().into_iter().filter(|c| c["available"] == false) {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let handler: [u8; 4] = c["handler"]
            .as_str()
            .unwrap()
            .as_bytes()
            .try_into()
            .unwrap();
        assert!(!a.tracks().iter().any(|t| t.id == 2), "{n}");
        assert!(!b.tracks().iter().any(|t| t.id == 2));
        assert!(
            a.refused()
                .iter()
                .any(|r| r.handler == handler && r.codec == *b"wvtt")
        );
        assert!(
            b.refused()
                .iter()
                .any(|r| r.handler == handler && r.codec == *b"wvtt")
        );
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        assert!(r.tracks().is_empty());
        assert!(r.cues(0).is_err());
    }
}
#[test]
fn an_unused_description_still_requires_a_valid_common_data_reference() {
    let n = "bad-unused-reference.mp4";
    let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .err()
        .expect("bad reference accepted");
    let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .err()
        .expect("bad reference accepted");
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(a.to_string(), "invalid sample data reference");
}
