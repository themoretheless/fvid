#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-mace-composition-runs")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn audio(n: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default())
        .unwrap_or_else(|e| panic!("audio {n}: {e}"))
}
fn video(n: &str) {
    let mut v =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
    for i in 0..6 {
        match v.read_frame_raw().unwrap().unwrap() {
            fvid::playback_native::RawFrame::Planar(p) => assert_eq!(p.frame.data, vec![128; 384]),
            _ => panic!("expected exact owned planes"),
        }
        assert_eq!(v.frame_interval(), Some((i, i + 1, 50)));
    }
    assert!(v.read_frame_raw().unwrap().is_none());
    v.rewind().unwrap();
    assert!(v.seek_raw(Duration::from_millis(45)).unwrap().is_some());
    assert_eq!(v.frame_interval(), Some((2, 3, 50)));
}
#[test]
fn whole_block_composition_controls_decode_and_play() {
    for pair in manifest()["pairs"].as_array().unwrap() {
        let n = pair["control"].as_str().unwrap();
        video(n);
        assert!(!transitions::collect(&mut audio(n)).is_empty(), "{n}");
    }
}
#[test]
fn equivalent_split_runs_preserve_indexes_pcm_and_seek() {
    for pair in manifest()["pairs"].as_array().unwrap() {
        let control = pair["control"].as_str().unwrap();
        let split = pair["split"].as_str().unwrap();
        let mut a = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(control)),
            Default::default(),
        )
        .unwrap();
        let mut b = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(split)),
            Default::default(),
        )
        .unwrap_or_else(|e| panic!("split {split}: {e}"));
        let mut c = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(split)),
            Default::default(),
        )
        .unwrap();
        assert_eq!(a.tracks().len(), b.tracks().len());
        for track in 0..a.tracks().len() {
            assert_eq!(
                a.tracks()[track].samples.len(),
                b.tracks()[track].samples.len()
            );
            for i in 0..a.tracks()[track].samples.len() {
                let x = a.tracks()[track].samples.get(i).unwrap();
                let y = b.tracks()[track].samples.get(i).unwrap();
                let z = c.tracks()[track].samples.get(i).unwrap();
                let clock = |s: &fvid::container::mp4::Sample| {
                    (s.dts, s.pts, s.duration, s.size, s.description_index)
                };
                assert_eq!(clock(&x), clock(&y));
                assert_eq!(
                    (z.dts, z.pts, z.duration, z.size, z.description_index),
                    clock(&x)
                );
                let mut px = Vec::new();
                let mut py = Vec::new();
                let mut pz = Vec::new();
                a.read_packet(track, i, &mut px).unwrap();
                b.read_packet(track, i, &mut py).unwrap();
                c.read_packet(track, i, &mut pz).unwrap();
                assert_eq!(px, py);
                assert_eq!(px, pz);
            }
        }
        let wanted = transitions::collect(&mut audio(control));
        let mut stream = audio(split);
        transitions::compare(&transitions::collect(&mut stream), &wanted);
        stream.rewind();
        transitions::compare(&transitions::collect(&mut stream), &wanted);
        for ms in [0, 1, 3, 10, 29, 30, 100] {
            let mut a = audio(control);
            let mut b = audio(split);
            assert_eq!(
                a.seek_time(Duration::from_millis(ms)),
                b.seek_time(Duration::from_millis(ms))
            );
            transitions::compare(&transitions::collect(&mut b), &transitions::collect(&mut a));
        }
        video(split);
    }
}
#[test]
fn malformed_or_nonuniform_block_composition_is_refused() {
    for case in manifest()["damaged"].as_array().unwrap() {
        let n = case["file"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .err()
        .unwrap()
        .to_string();
        let b = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(a.contains(expected), "{n}: {a}");
        assert_eq!(a, b);
    }
}
