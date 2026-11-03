#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-lpcm-v2")
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
fn open(name: &str) -> fvid::Result<fvid::playback_mp4_audio::Mp4AudioReader<Cursor<Vec<u8>>>> {
    fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default())
}
#[test]
fn legacy_pcm_and_actual_video_are_valid_controls() {
    for c in cases() {
        if let Some(control) = c["control"].as_str() {
            let mut reader = open(control).unwrap();
            let got = rational::collect(&mut reader)
                .into_iter()
                .flat_map(|p| p.data)
                .collect::<Vec<_>>();
            assert_eq!(got, bytes(c["expected"].as_str().unwrap()), "{control}");
        }
    }
    let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("i16-le-c1-r96000.mov")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame().unwrap().is_some());
    }
    assert!(video.read_frame().unwrap().is_none());
}
#[test]
fn packed_lpcm_keeps_literal_pcm_native_rate_seek_and_eof() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut reader = match open(name) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        let expected = bytes(c["expected"].as_str().unwrap());
        assert_eq!(reader.sample_rate(), rate as u32);
        assert_eq!(reader.channels(), (stride / 4) as u16);
        assert_eq!(
            reader.duration(),
            Some(rational::duration(12, rate as u128))
        );
        let full = rational::collect(&mut reader);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            rational::duration(6, rate as u128),
            Duration::MAX,
        ] {
            reader.seek_time(at);
            let got = rational::collect(&mut reader);
            let want = rational::suffix(&full, at, (12, rate as u128));
            rational::exact(&got, &want, name);
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
        reader.validate_eof().unwrap();
    }
    assert!(failures.is_empty(), "LPCM refusals: {failures:?}");
}
#[test]
fn actual_workers_preserve_high_rate_lpcm_navigation() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut reader = open(name).unwrap();
        let full = rational::collect(&mut reader);
        let rate = c["rate"].as_u64().unwrap() as u128;
        let end = (12, rate);
        for at in [Duration::ZERO, rational::duration(6, rate), Duration::MAX] {
            let wanted = rational::suffix(&full, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker = fvid::audio_thread::AudioPlayback::start(
                Box::new(open(name).unwrap()),
                move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                },
            );
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, name);
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}

#[test]
fn malformed_v2_headers_refuse_specifically_and_other_layouts_stay_unsupported() {
    let manifest: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in manifest["bad"].as_array().unwrap() {
        let error = match open(c["file"].as_str().unwrap()) {
            Err(e) => e,
            Ok(_) => panic!("malformed v2 admitted"),
        };
        assert!(matches!(error, fvid::Error::Invalid(_)), "{error:?}");
        assert!(
            error.to_string().contains(c["needle"].as_str().unwrap()),
            "{error}"
        );
    }
    for name in manifest["unsupported"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let mut container =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(name)), Default::default())
                .unwrap();
        assert!(container.tracks().iter().all(|t| t.handler != *b"soun"));
        assert!(open(name).is_err());
        let _ = &mut container;
    }
}

#[test]
fn owned_normalized_and_precise_exports_keep_rate_geometry_and_low_float_bits() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let pcm = bytes(c["expected"].as_str().unwrap());
        let precise = bytes(c["expected64"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        for cut in [false, true] {
            let interval = cut.then(|| {
                (
                    rational::duration(6, rate as u128),
                    rational::duration(12, rate as u128),
                )
            });
            let first = if cut { 6 * channels } else { 0 };
            let mut out = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, if cut { 6 } else { 12 });
            assert_eq!(out, pcm[first * 4..], "{name} f32 cut={cut}");
            out.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(&data),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(out, precise[first * 8..], "{name} f64 cut={cut}");
        }
    }
}
