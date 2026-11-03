#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-v0-pcm-aliases")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    manifest()["cases"].as_array().unwrap().clone()
}
fn open(n: &str) -> fvid::Result<fvid::playback_mp4_audio::Mp4AudioReader<Cursor<Vec<u8>>>> {
    fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default())
}
#[test]
fn classic_pcm_controls_and_actual_video_are_independently_valid() {
    for c in cases() {
        let mut control = open(c["control"].as_str().unwrap()).unwrap();
        let full = rational::collect(&mut control);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            bytes(c["expected"].as_str().unwrap())
        );
    }
    let name = cases()[0]["file"].as_str().unwrap().to_owned();
    let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes(&name)),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = video.read_frame().unwrap().unwrap();
        assert_eq!(
            f.picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            bytes(&format!("frame-{i}.yuv16"))
        );
    }
    assert!(video.read_frame().unwrap().is_none());
}
#[test]
fn v0_aliases_keep_native_pcm_metadata_clock_and_exact_navigation() {
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
        let mut control = open(c["control"].as_str().unwrap()).unwrap();
        let full = rational::collect(&mut control);
        assert_eq!(reader.codec(), if c["bits"] == 8 { "raw " } else { "twos" });
        assert_eq!(reader.bits_per_sample(), c["bits"].as_u64().unwrap() as u16);
        assert_eq!(reader.duration(), Some(Duration::from_millis(60)));
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_micros(125),
            Duration::from_millis(20),
            Duration::from_millis(60),
            Duration::MAX,
        ] {
            reader.seek_time(at);
            rational::exact(
                &rational::collect(&mut reader),
                &rational::suffix(&full, at, (3, 50)),
                name,
            );
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
        reader.validate_eof().unwrap();
    }
    assert!(failures.is_empty(), "v0 alias refusals: {failures:?}");
}
#[test]
fn actual_workers_play_pcm_aliases_and_keep_eof_anchors() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut control = open(c["control"].as_str().unwrap()).unwrap();
        let full = rational::collect(&mut control);
        for at in [Duration::ZERO, Duration::from_millis(20), Duration::MAX] {
            let wanted = rational::suffix(&full, at, (3, 50));
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
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, (3, 50))]);
        }
    }
}
#[test]
fn owned_exports_and_public_inspection_preserve_classic_pcm() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let expected = bytes(c["expected"].as_str().unwrap());
        let channels = c["channels"].as_u64().unwrap() as usize;
        let info =
            fvid::native_media::audio_source_info_selected(&root().join(name), None).unwrap();
        assert_eq!((info.sample_rate, info.channels), (8000, channels as u16));
        for cut in [false, true] {
            let interval = cut.then(|| (Duration::from_millis(20), Duration::from_millis(60)));
            let first = if cut { 160 * channels } else { 0 };
            let mut out = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, if cut { 320 } else { 480 });
            assert_eq!(out, expected[first * 4..], "{name}");
            out.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &Default::default(),
            )
            .unwrap();
            let expected64 = expected[first * 4..]
                .chunks_exact(4)
                .flat_map(|p| (f32::from_le_bytes(p.try_into().unwrap()) as f64).to_le_bytes())
                .collect::<Vec<_>>();
            assert_eq!(out, expected64, "{name} f64");
        }
    }
}
#[test]
fn nonclassic_descriptions_stay_refused_and_do_not_hide_a_later_valid_track() {
    let m = manifest();
    for name in m["unsupported"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        assert!(reader.tracks().iter().all(|t| t.handler != *b"soun"));
        assert!(open(name).is_err());
    }
    for name in m["fallback"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let mut reader = open(name).unwrap();
        assert_eq!(reader.codec(), "twos");
        let full = rational::collect(&mut reader);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            bytes("b16-c1.f32le")
        );
    }
}

#[test]
fn malformed_classic_headers_refuse_for_the_specific_field() {
    let m = manifest();
    for c in m["malformed"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let input = bytes(name);
        let error = match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(&input),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("malformed classic header admitted"),
        };
        assert!(
            matches!(error, fvid::Error::Invalid(_)),
            "{name}: {error:?}"
        );
        assert!(
            error.to_string().contains(c["needle"].as_str().unwrap()),
            "{name}: {error}"
        );
        let error = match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(&input),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("malformed owned header admitted"),
        };
        assert!(!error.is_unsupported());
        assert!(
            error.to_string().contains(c["needle"].as_str().unwrap()),
            "{name}: {error}"
        );
    }
}
