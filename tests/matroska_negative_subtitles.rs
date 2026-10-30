#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/matroska-negative-subtitles")
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
    c["cues"]
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
        .collect()
}
fn rgb(n: &str) -> Vec<Vec<u8>> {
    let mut r =
        fvid::playback_webm::WebmVideoReader::open_software(Cursor::new(bytes(n)), 32 << 20)
            .unwrap();
    let mut out = Vec::new();
    for i in 0..3 {
        assert!(r.read_frame().unwrap());
        assert_eq!(r.dimensions(), [16, 16]);
        assert_eq!(
            r.frame_interval().map(|v| (v.0, v.1)),
            Some((i * 40_000_000, (i + 1) * 40_000_000))
        );
        out.push(r.rgb().to_vec());
    }
    assert!(!r.read_frame().unwrap());
    out
}
#[test]
fn both_parsers_keep_exact_packet_spans_and_the_actual_video() {
    let pictures = rgb("utf8-positive.mkv");
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        a.scan_all().unwrap();
        b.scan_all().unwrap();
        let indices: Vec<_> = a
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        let other: Vec<_> = b
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == 2)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(indices.len(), 3);
        assert_eq!(a.tracks[1].default_duration_ns, 0);
        assert_eq!(other.len(), 3);
        for (i, (&x, &y)) in indices.iter().zip(&other).enumerate() {
            let wanted = (
                c["pts_ns"][i].as_i64().unwrap(),
                c["duration_ns"][i].as_u64(),
            );
            assert_eq!(
                (a.packets[x].pts_ns, a.packets[x].duration_ns),
                wanted,
                "{n}"
            );
            assert_eq!((b.packets[y].pts_ns, b.packets[y].duration_ns), wanted);
            let p = a.read_packet(x).unwrap();
            assert_eq!(p, b.read_packet(y).unwrap());
            assert_eq!(
                p,
                bytes(&format!("{}-{i}.packet", c["codec"].as_str().unwrap()))
            );
        }
        assert_eq!(rgb(n), pictures, "{n}");
    }
}
#[test]
fn negative_subtitle_frames_are_decoded_but_never_published() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
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
        let wanted = expected(&c);
        if actual != wanted {
            failures.push(format!("{n}: {actual:?} != {wanted:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "Negative Matroska subtitle presentation: {failures:?}"
    );
}
#[test]
fn cue_gaps_and_endpoints_remain_correct_after_actual_video_seeks() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let wanted = expected(&c);
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for ns in [
            0,
            10_000_000,
            20_000_000,
            29_999_999,
            30_000_000,
            60_000_000,
            69_999_999,
            70_000_000,
            90_000_000,
            99_999_999,
            100_000_000,
            110_000_000,
        ] {
            let at = Duration::from_nanos(ns);
            assert!(video.seek_raw(at).unwrap().is_some());
            let picture = u128::from(ns / 40_000_000);
            assert_eq!(
                video.frame_interval(),
                Some((
                    picture * 40_000_000,
                    (picture + 1) * 40_000_000,
                    1_000_000_000
                )),
                "{n} sought picture at{ns}"
            );
            let expected = wanted
                .iter()
                .find(|v| v.1 <= u128::from(ns) && u128::from(ns) < v.2)
                .map(|v| v.0.as_str());
            let actual =
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|v| v.text.as_str());
            if actual != expected {
                failures.push(format!("{n} at{ns}: {actual:?} != {expected:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "Matroska subtitle seek: {failures:?}");
}
