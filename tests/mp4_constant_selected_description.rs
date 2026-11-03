#![cfg(feature = "player")]
use std::{io::Cursor, path::Path};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-constant-selected-description")
            .join(n),
    )
    .unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[key]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn indexed_controls_retain_selected_headers_and_exact_packets() {
    for c in cases("cases").into_iter().chain(cases("refusals")) {
        let mut r = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let mut control = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let selected = if c["kind"] == "audio" { 1 } else { 0 };
        let count = r.tracks()[selected].samples.len();
        for i in 0..count {
            assert_eq!(
                r.tracks()[selected]
                    .samples
                    .get(i)
                    .unwrap()
                    .description_index,
                c["selected"].as_u64().unwrap() as u32
            );
            let (mut a, mut b) = (Vec::new(), Vec::new());
            r.read_packet(selected, i, &mut a).unwrap();
            control.read_packet(selected, i, &mut b).unwrap();
            assert_eq!(a, b);
        }
    }
}
#[test]
fn constant_api_uses_the_actual_header_and_keeps_coded_bytes() {
    let mut failures = Vec::new();
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let mut r = match fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let control = fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let t = if c["kind"] == "audio" { 1 } else { 0 };
        let a = &r.tracks()[t];
        let b = &control.tracks()[t];
        assert_eq!(
            (a.codec, a.sample_rate, a.channels, a.width, a.height),
            (b.codec, b.sample_rate, b.channels, b.width, b.height),
            "{name}"
        );
        assert_eq!(a.configuration, b.configuration);
        assert!(a.ensure_constant_description().is_ok());
        let (mut p, mut q) = (Vec::new(), Vec::new());
        r.read_packet(t, 0, &mut p).unwrap();
        let mut control = control;
        control.read_packet(t, 0, &mut q).unwrap();
        assert_eq!(p, q);
    }
    assert!(
        failures.is_empty(),
        "constant selected refusals: {failures:?}"
    );
}
#[test]
fn selected_opaque_descriptions_are_not_constant_playback_acceptance() {
    for c in cases("refusals") {
        let result = fvid::container::mp4::Mp4Reader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        );
        assert!(
            result.is_err(),
            "unknown selected description must not inherit unused known decoder metadata"
        );
    }
}

#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn actual_audio_workers_and_video_seek_keep_the_selected_content() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        if c["kind"] == "audio" {
            let source = || {
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes(name)),
                    Default::default(),
                )
                .unwrap()
            };
            let mut reader = source();
            let full = rational::collect(&mut reader);
            assert_eq!(
                full.iter()
                    .flat_map(|p| p.data.iter().copied())
                    .collect::<Vec<_>>(),
                bytes(c["expected"].as_str().unwrap())
            );
            let frames = c["frames"].as_u64().unwrap() as u128;
            let end = (frames, 48000);
            assert_eq!(reader.duration(), Some(rational::duration(frames, 48000)));
            for at in [
                Duration::ZERO,
                rational::duration(frames / 2, 48000),
                Duration::MAX,
            ] {
                let wanted = rational::suffix(&full, at, end);
                reader.seek_time(at);
                rational::exact(&rational::collect(&mut reader), &wanted, name);
                let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
                let copy = raw.clone();
                let trace = Arc::new(Mutex::new(transitions::Trace::default()));
                let mut worker =
                    fvid::audio_thread::AudioPlayback::start(Box::new(source()), move || {
                        Box::new(rational::RawCapture {
                            inner: transitions::Capture::new(trace),
                            raw: copy,
                        })
                    });
                worker.seek(at);
                worker.play();
                transitions::ended(&worker, worker.generation());
                let got = raw.lock().unwrap();
                rational::exact(&got.packets, &wanted, name);
                assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
            }
            reader.rewind();
            rational::exact(&rational::collect(&mut reader), &full, name);
        } else {
            let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
                Cursor::new(bytes(name)),
                Default::default(),
                32 << 20,
            )
            .unwrap();
            for target in [0, 1, 2, i64::MAX] {
                let landed = target.min(2);
                assert_eq!(reader.seek_to_sync(target), landed);
                for i in landed..3 {
                    let f = reader.read_frame().unwrap().unwrap();
                    let got = f
                        .picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>();
                    assert_eq!(got, bytes(&format!("frame-{i}.yuv16")), "{name}");
                }
                assert!(reader.read_frame().unwrap().is_none());
            }
        }
    }
}

#[test]
fn owned_constant_api_preserves_the_same_admission_and_primary_metadata() {
    let changing = include_bytes!("fixtures/playback-errors/mp4-audio-reconfiguration/pcm.mp4");
    assert!(
        fvid::container::mp4::Mp4Reader::open(Cursor::new(&changing[..]), Default::default())
            .is_err()
    );
    assert!(
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&changing[..]), Default::default())
            .is_err()
    );
    for c in cases("cases") {
        let mut reader = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let mut control = fvid_media::owned_mp4::Mp4Reader::open(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let index = if c["kind"] == "audio" { 1 } else { 0 };
        let actual = &reader.tracks()[index];
        let expected = &control.tracks()[index];
        assert_eq!(
            (
                actual.codec,
                actual.width,
                actual.height,
                actual.sample_rate,
                actual.channels
            ),
            (
                expected.codec,
                expected.width,
                expected.height,
                expected.sample_rate,
                expected.channels
            )
        );
        assert_eq!(actual.configuration, expected.configuration);
        for i in 0..actual.samples.len() {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            reader.read_packet(index, i, &mut a).unwrap();
            control.read_packet(index, i, &mut b).unwrap();
            assert_eq!(a, b);
        }
    }
    for c in cases("refusals") {
        assert!(
            fvid_media::owned_mp4::Mp4Reader::open(
                Cursor::new(bytes(c["file"].as_str().unwrap())),
                Default::default()
            )
            .is_err()
        );
    }
}
