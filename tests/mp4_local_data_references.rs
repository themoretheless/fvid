#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-local-data-references")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn decoded_audio(n: &str) -> Vec<u8> {
    let mut r =
        fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
    let mut d = r.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = r.read_packet().unwrap() {
        pcm.extend(
            d.decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                .unwrap()
                .unwrap()
                .data,
        );
    }
    assert!(r.read_packet().unwrap().is_none());
    pcm
}
fn accepted(c: &serde_json::Value) {
    let n = c["file"].as_str().unwrap();
    let kind = c["kind"].as_str().unwrap();
    let mut a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .unwrap_or_else(|e| panic!("root {n}: {e}"));
    let mut b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
        .unwrap_or_else(|e| panic!("owned {n}: {e}"));
    assert!(a.refused().is_empty());
    assert!(b.refused().is_empty());
    let step = if kind == "fragment" { 40 } else { 1 };
    let scale = if kind == "fragment" { 1000 } else { 50 };
    let frame_ns = if kind == "fragment" {
        40_000_000
    } else {
        20_000_000
    };
    for i in [2, 0, 1] {
        let s = a.tracks()[0].samples.get(i).unwrap();
        let t = b.tracks()[0].samples.get(i).unwrap();
        assert_eq!((s.pts, s.duration), (i as i64 * step, step as u32));
        assert_eq!((t.pts, t.duration), (s.pts, s.duration));
        if c["alternating"] == true {
            assert_eq!(s.description_index, i as u32 + 1);
        }
        let mut x = Vec::new();
        let mut y = Vec::new();
        a.read_packet(0, i, &mut x).unwrap();
        b.read_packet(0, i, &mut y).unwrap();
        assert_eq!(x, bytes(&format!("frame-{i}.packet")));
        assert_eq!(y, x);
    }
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(n)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    let rgb_reference;
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        assert_eq!(
            (f.presentation_time.ticks, f.duration.ticks),
            (i * step, step)
        );
        let p: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(p, bytes(&format!("frame-{i}.yuv16")), "{n}");
    }
    assert!(v.read_frame().unwrap().is_none());
    let mut native =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
    rgb_reference = (0..3)
        .map(|_| {
            native
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(native.read_frame_raw().unwrap().is_none());
    native.rewind().unwrap();
    for ns in [
        0,
        frame_ns - 1,
        frame_ns,
        2 * frame_ns - 1,
        2 * frame_ns,
        3 * frame_ns - 1,
        3 * frame_ns,
    ] {
        let i = (ns / frame_ns).min(2) as usize;
        let f = native.seek_raw(Duration::from_nanos(ns)).unwrap().unwrap();
        assert_eq!(f.into_rgb(32 << 20).unwrap(), rgb_reference[i]);
        let (s, e, ts) = native.frame_interval().unwrap();
        assert_eq!(
            (
                s * 1_000_000_000 / u128::from(ts),
                e * 1_000_000_000 / u128::from(ts)
            ),
            (
                i as u128 * u128::from(frame_ns),
                (i as u128 + 1) * u128::from(frame_ns)
            ),
            "{n}"
        );
    }
    assert_eq!(a.tracks()[0].timescale, scale);
    if kind == "text" || kind == "fragment" {
        let prefix = if kind == "text" {
            "text-4-3"
        } else {
            "fragment-text"
        };
        for i in 0..3 {
            let mut x = Vec::new();
            let mut y = Vec::new();
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(x, bytes(&format!("{prefix}-{i}.packet")));
            assert_eq!(y, x);
        }
        let mut r = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        let cues = r.cues(0).unwrap();
        let actual: Vec<_> = cues
            .iter()
            .map(|c| (c.text.as_str(), c.start.as_nanos(), c.end.as_nanos()))
            .collect();
        if kind == "text" {
            assert_eq!(
                actual,
                [
                    ("A", 0, 20_000_000),
                    ("BBB", 20_000_000, 40_000_000),
                    ("CC", 40_000_000, 60_000_000)
                ]
            );
        } else {
            assert_eq!(
                actual,
                [
                    ("Middle caption", 0, 40_000_000),
                    ("Last caption", 40_000_000, 80_000_000)
                ]
            );
        }
    }
    if kind == "pcm" {
        assert_eq!(decoded_audio(n), bytes("pcm.f32le"));
    }
    if kind == "mace" {
        assert_eq!(decoded_audio(n).len(), 2880 * 4);
        assert_eq!(decoded_audio(n), decoded_audio("mace-single-control.mp4"));
    }
    a.restrict_packet_bytes(1);
    assert!(
        a.read_packet(0, 0, &mut Vec::new())
            .unwrap_err()
            .to_string()
            .contains("packet exceeds budget")
    );
    let mut limit = fvid::container::mp4::Limits::default();
    limit.metadata_bytes = 128;
    assert!(fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), limit).is_err());
}
#[test]
fn single_local_reference_controls_decode_and_seek() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["single"] == true)
    {
        accepted(c);
    }
}
#[test]
fn multiple_local_references_and_largest_index_decode_and_seek() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["single"] == false)
    {
        accepted(c);
    }
}
fn refused(new_specific: bool) {
    for c in manifest()["damaged"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["new_specific_index_check"] == new_specific)
    {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("root accepted invalid reference");
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .err()
            .expect("owned accepted invalid reference");
        assert!(matches!(a, fvid::Error::Invalid(_)));
        assert!(!b.is_unsupported());
        assert_eq!(a.to_string(), c["expected"].as_str().unwrap(), "{n}");
        assert_eq!(b.to_string(), a.to_string());
    }
}
#[test]
fn existing_missing_external_and_invalid_single_indices_still_refuse() {
    refused(false);
}
#[test]
fn multiple_reference_indices_including_unused_opaque_entries_are_bounded() {
    refused(true);
}
