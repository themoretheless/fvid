#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-subtitle-equal-pts")
            .join(name),
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
fn authored_zero_duration_packets_and_actual_video_remain_valid() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(data.clone()), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(data.clone()), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[1].timescale, c["scale"].as_u64().unwrap() as u32);
        assert_eq!(a.tracks()[1].samples.len(), 3);
        for i in 0..3 {
            let p = a.tracks()[1].samples.get(i).unwrap();
            let q = b.tracks()[1].samples.get(i).unwrap();
            assert_eq!(
                (p.pts, p.duration),
                (
                    c["pts"][i].as_i64().unwrap(),
                    c["durations"][i].as_u64().unwrap() as u32
                )
            );
            assert_eq!((p.pts, p.duration), (q.pts, q.duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, y);
            assert_eq!(&x[2..], c["texts"][i].as_str().unwrap().as_bytes());
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(data),
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
        assert_eq!(video.seek_to_sync(0), 0);
        assert!(video.read_frame().unwrap().is_some());
    }
}
#[test]
fn co_timed_unspecified_endpoints_share_the_next_strictly_later_point() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let actual: Vec<_> = cues
            .iter()
            .map(|c| (c.text.clone(), c.start.as_nanos(), c.end.as_nanos()))
            .collect();
        let wanted: Vec<_> = c["cues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["text"].as_str().unwrap().to_owned(),
                    u128::from(c["start_ns"].as_u64().unwrap()),
                    u128::from(c["end_ns"].as_u64().unwrap()),
                )
            })
            .collect();
        if actual != wanted {
            failures.push(format!("{name}: {actual:?} != {wanted:?}"));
            continue;
        }
        for cue in &cues {
            assert!(cue.end > cue.start);
            for at in [cue.start, cue.end - Duration::from_nanos(1)] {
                assert!(fvid::subtitles::active(&cues, at, Duration::ZERO).is_some());
            }
        }
        let again = r.cues(0).unwrap();
        assert_eq!(
            again
                .iter()
                .map(|c| (&c.text, c.start, c.end))
                .collect::<Vec<_>>(),
            cues.iter()
                .map(|c| (&c.text, c.start, c.end))
                .collect::<Vec<_>>()
        );
    }
    assert!(failures.is_empty(), "inferred spans: {failures:?}");
}
#[test]
fn subtitle_lookup_matches_the_authored_clock_after_real_video_seek() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut subs = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let cues = subs.cues(0).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for ns in [
            0,
            1,
            999_000,
            1_000_000,
            19_999_999,
            20_000_000,
            39_999_999,
            40_000_000,
            59_999_999,
            1_999_999_999,
            2_000_000_000,
            2_500_000_000,
            2_999_999_999,
            3_000_000_000,
            3_500_000_000,
            3_999_999_999,
            4_000_000_000,
            4_999_999_999,
            5_000_000_000,
            5_019_999_999,
        ] {
            let at = Duration::from_nanos(ns);
            let frame = video.seek_raw(at).unwrap().unwrap();
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
                failures.push(format!("{name} at{ns}: {actual:?} != {expected:?}"));
            }
            assert!(!frame.into_rgb(32 << 20).unwrap().is_empty());
        }
    }
    assert!(
        failures.is_empty(),
        "actual seek subtitle lookup: {failures:?}"
    );
}
#[test]
fn packet_budget_and_missing_track_refusals_remain_strict() {
    for c in cases() {
        let limits = fvid::container::mp4::Limits {
            packet_bytes: 1,
            ..Default::default()
        };
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            limits,
        )
        .unwrap();
        assert!(
            r.cues(0)
                .err()
                .unwrap()
                .to_string()
                .contains("packet exceeds budget")
        );
        assert!(
            r.cues(1)
                .err()
                .unwrap()
                .to_string()
                .contains("no such subtitle track")
        );
    }
}

#[test]
fn explicit_duration_controls_preserve_every_authored_endpoint() {
    for c in cases() {
        let data = bytes(c["control"].as_str().unwrap());
        let demux =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(data.clone()), Default::default())
                .unwrap();
        for i in 0..3 {
            let p = demux.tracks()[1].samples.get(i).unwrap();
            assert_eq!(p.pts, c["pts"][i].as_i64().unwrap());
            assert_eq!(
                u64::from(p.duration),
                c["control_durations"][i].as_u64().unwrap()
            );
        }
        let mut reader = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(data),
            Default::default(),
        )
        .unwrap();
        let actual: Vec<_> = reader
            .cues(0)
            .unwrap()
            .into_iter()
            .map(|v| (v.text, v.start.as_nanos(), v.end.as_nanos()))
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
        assert_eq!(actual, expected, "{}", c["control"]);
    }
}
