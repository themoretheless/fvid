#![cfg(feature = "player")]
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-subtitle-edits")
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
fn wanted(c: &serde_json::Value) -> Vec<(String, u128, u128)> {
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
#[test]
fn parsers_preserve_subtitle_edit_programs_raw_packets_and_actual_video() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(
            a.movie_timescale(),
            c["movie_scale"].as_u64().unwrap() as u32
        );
        let video_edits: Vec<_> = a.tracks()[0]
            .edits
            .iter()
            .map(|e| (e.duration, e.media_time))
            .collect();
        let expected_video: Vec<_> = c["video_edits"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| (r[0].as_u64().unwrap(), r[1].as_i64().unwrap()))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(video_edits, expected_video);
        assert_eq!(a.tracks()[1].timescale, c["scale"].as_u64().unwrap() as u32);
        let actual: Vec<_> = a.tracks()[1]
            .edits
            .iter()
            .map(|e| (e.duration, e.media_time))
            .collect();
        let other: Vec<_> = b.tracks()[1]
            .edits
            .iter()
            .map(|e| (e.duration, e.media_time))
            .collect();
        let expected: Vec<_> = c["edits"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| (r[0].as_u64().unwrap(), r[1].as_i64().unwrap()))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(actual, expected, "{n}");
        assert_eq!(other, expected);
        assert_eq!(a.tracks()[1].samples.len(), 3);
        for i in 0..3 {
            let x = a.tracks()[1].samples.get(i).unwrap();
            let y = b.tracks()[1].samples.get(i).unwrap();
            let pts = c["pts"]
                .as_array()
                .map_or(i as i64 * 40, |p| p[i].as_i64().unwrap());
            let duration = c["durations"]
                .as_array()
                .map_or(40, |d| d[i].as_u64().unwrap() as u32);
            assert_eq!((x.pts, x.duration), (pts, duration));
            assert_eq!((y.pts, y.duration), (x.pts, x.duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("text-{i}.packet")));
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
            let pts = c["video_pts_ticks"]
                .as_array()
                .map_or(i as i64 * 40, |p| p[i].as_i64().unwrap());
            let duration = c["video_durations_ticks"]
                .as_array()
                .map_or(40, |d| d[i].as_i64().unwrap());
            assert_eq!(
                (f.presentation_time.ticks, f.duration.ticks),
                (pts, duration)
            );
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
fn subtitle_edits_map_clip_repeat_and_hide_cues_on_the_movie_clock() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
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
        let expected = wanted(&c);
        if actual != expected {
            failures.push(format!("{n}: {actual:?} != {expected:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "MP4 subtitle movie edits: {failures:?}"
    );
}
#[test]
fn actual_video_seeks_select_the_edited_caption_at_every_boundary() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let expected = wanted(&c);
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let video_intervals: Vec<(u128, u128)> = c["video_intervals_ns"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| {
                        (
                            u128::from(v[0].as_u64().unwrap()),
                            u128::from(v[1].as_u64().unwrap()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                (0..3)
                    .map(|i| (i * 40_000_000, (i + 1) * 40_000_000))
                    .collect()
            });
        let mut times = vec![
            0,
            10_000_000,
            30_000_000,
            40_000_000,
            60_000_000,
            80_000_000,
            100_000_000,
            119_999_999,
        ];
        for (s, e) in &video_intervals {
            times.extend([*s as u64, (*e as u64).saturating_sub(1), *e as u64]);
        }
        for (_, s, e) in &expected {
            times.extend([*s as u64, (*e as u64).saturating_sub(1), *e as u64]);
        }
        for ns in times
            .into_iter()
            .filter(|n| *n < c["video_duration_ns"].as_u64().unwrap_or(120_000_000))
        {
            let at = Duration::from_nanos(ns);
            assert!(v.seek_raw(at).unwrap().is_some());
            let (s, e, scale) = v.frame_interval().unwrap();
            let expected_frame = *video_intervals
                .iter()
                .find(|(s, e)| *s <= u128::from(ns) && u128::from(ns) < *e)
                .unwrap();
            assert_eq!(
                (
                    s * 1_000_000_000 / u128::from(scale),
                    e * 1_000_000_000 / u128::from(scale)
                ),
                expected_frame,
                "{n} video seek"
            );
            let actual =
                fvid::subtitles::active(&cues, at, Duration::ZERO).map(|v| v.text.as_str());
            let desired = expected
                .iter()
                .find(|(_, s, e)| *s <= u128::from(ns) && u128::from(ns) < *e)
                .map(|(t, _, _)| t.as_str());
            if actual != desired {
                failures.push(format!("{n} at{ns}: {actual:?} != {desired:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "MP4 edited caption seek: {failures:?}");
}
#[test]
fn malformed_edit_count_is_refused_before_any_caption_is_published() {
    let n = "bad-edit-count.mp4";
    let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .err()
        .expect("bad edit accepted");
    let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .err()
        .expect("bad edit accepted");
    assert_eq!(a.to_string(), b.to_string());
    assert!(a.to_string().contains("table"), "{a}");
}
#[test]
fn repeated_edits_obey_the_subtitle_output_sample_budget() {
    for c in cases().into_iter().filter(|c| c["resource_bound"] == true) {
        let n = c["file"].as_str().unwrap();
        let limits = fvid::container::mp4::Limits {
            samples: 6,
            ..Default::default()
        };
        let mut parser =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), limits).unwrap();
        let mut packet = Vec::new();
        parser.read_packet(1, 2, &mut packet).unwrap();
        assert_eq!(packet, bytes("text-2.packet"));
        let mut r =
            fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(Cursor::new(bytes(n)), limits)
                .unwrap();
        let e = r
            .cues(0)
            .err()
            .expect("repeated cues must not bypass the sample budget");
        assert!(
            e.to_string().contains("subtitle cue count exceeds limit"),
            "{e}"
        );
    }
}

#[test]
fn coincident_pts_identity_edits_preserve_source_sample_precedence() {
    for c in cases().into_iter().filter(|c| c["coincident"] == true) {
        let n = c["file"].as_str().unwrap();
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let actual: Vec<_> = cues
            .iter()
            .map(|v| (v.text.clone(), v.start.as_nanos(), v.end.as_nanos()))
            .collect();
        assert_eq!(actual, wanted(&c), "{n}");
        assert_eq!(
            fvid::subtitles::active(&cues, Duration::ZERO, Duration::ZERO)
                .unwrap()
                .text,
            "First caption"
        );
    }
}
