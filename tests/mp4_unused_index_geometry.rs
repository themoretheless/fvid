use std::{io::Cursor, path::Path};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-unused-index-geometry")
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
fn controls_fit_three_video_entries_and_one_audio_chunk() {
    for c in cases("controls") {
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 4;
        let r = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            limits,
        )
        .unwrap();
        assert_eq!(r.tracks().len(), 2);
        assert_eq!(r.tracks()[1].samples.cost(), 1);
        assert_eq!(
            r.tracks()[1].samples.len(),
            c["units"].as_u64().unwrap() as usize
        );
    }
}
#[test]
fn unused_coded_descriptions_do_not_expand_the_selected_chunk_index() {
    let mut failures = Vec::new();
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 4;
        let r =
            match fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(bytes(name)), limits) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{name}: {e}"));
                    continue;
                }
            };
        assert_eq!(
            r.tracks().len(),
            2,
            "{name}: dormant metadata dropped the audio"
        );
        assert_eq!(r.tracks()[1].samples.cost(), 1, "{name}");
        let mut owned_limits = fvid_media::owned_mp4::Limits::default();
        owned_limits.samples = 4;
        let owned =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(bytes(name)), owned_limits)
                .unwrap();
        assert_eq!(owned.tracks().len(), 2);
        assert_eq!(owned.tracks()[1].samples.cost(), 1);
        assert_eq!(
            owned.tracks()[1].samples.len(),
            c["units"].as_u64().unwrap() as usize
        );
        assert_eq!(
            r.tracks()[1].samples.len(),
            c["units"].as_u64().unwrap() as usize
        );
    }
    assert!(
        failures.is_empty(),
        "dormant index geometry refusals: {failures:?}"
    );
}

#[test]
fn high_budget_packets_match_controls_and_record_retained_index_geometry() {
    let mut metrics = Vec::new();
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let mut reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let mut control = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let samples = reader.tracks()[1].samples.len();
        assert_eq!(samples, control.tracks()[1].samples.len());
        for i in 0..samples {
            let a = reader.tracks()[1].samples.get(i).unwrap();
            let b = control.tracks()[1].samples.get(i).unwrap();
            assert_eq!(
                (a.dts, a.pts, a.duration, a.size),
                (b.dts, b.pts, b.duration, b.size),
                "{name}"
            );
            let (mut a, mut b) = (Vec::new(), Vec::new());
            reader.read_packet(1, i, &mut a).unwrap();
            control.read_packet(1, i, &mut b).unwrap();
            assert_eq!(a, b, "{name}");
        }
        metrics.push(serde_json::json!({"file":name,"units":samples,"cost":reader.tracks()[1].samples.cost(),"retained_payload":reader.estimated_index_payload_bytes().unwrap()}));
    }
    println!("INDEX_METRICS:{}", serde_json::to_string(&metrics).unwrap());
}

#[test]
fn malformed_mapping_and_block_units_remain_specific_refusals() {
    for c in cases("complex") {
        let data = bytes(c["file"].as_str().unwrap());
        let reader =
            fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&data), Default::default())
                .unwrap();
        let control = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.tracks()[1].samples.cost(),
            c["units"].as_u64().unwrap() as usize
        );
        assert!(reader.tracks()[1].samples.expanded().is_some());
        for i in 0..reader.tracks()[1].samples.len() {
            let a = reader.tracks()[1].samples.get(i).unwrap();
            let b = control.tracks()[1].samples.get(i).unwrap();
            assert_eq!(
                (a.pts, a.dts, a.duration, a.size),
                (b.pts, b.dts, b.duration, b.size)
            );
        }
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 4;
        if let Ok(limited) =
            fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&data), limits)
        {
            assert!(limited.tracks().iter().all(|t| t.handler != *b"soun"));
        }
    }
    for c in cases("bad") {
        let data = bytes(c["file"].as_str().unwrap());
        let error = match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(&data),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("bad table admitted"),
        };
        assert!(matches!(error, fvid::Error::Invalid(_)), "{error:?}");
        assert!(
            error.to_string().contains(c["needle"].as_str().unwrap()),
            "{error}"
        );
        let error = match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(&data),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("bad owned table admitted"),
        };
        assert!(!error.is_unsupported());
        assert!(
            error.to_string().contains(c["needle"].as_str().unwrap()),
            "{error}"
        );
    }
    for c in cases("controls") {
        let mut limits = fvid::container::mp4::Limits::default();
        limits.samples = 3;
        let reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            limits,
        )
        .unwrap();
        assert!(reader.tracks().iter().all(|t| t.handler != *b"soun"));
        assert_eq!(reader.tracks()[0].samples.cost(), 3);
    }
}

#[cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn tight_index_budget_keeps_actual_pcm_mace_and_worker_navigation() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases("controls") {
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame().unwrap().is_some());
        }
        assert!(video.read_frame().unwrap().is_none());
    }
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let source = || {
            let mut limits = fvid::container::mp4::Limits::default();
            limits.samples = 4;
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes(name)), limits)
                .unwrap()
        };
        let mut control = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(c["control"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap();
        let full = rational::collect(&mut control);
        let flat = full
            .iter()
            .flat_map(|p| p.data.iter().copied())
            .collect::<Vec<_>>();
        let frames = c["frames"].as_u64().unwrap() as u128;
        assert_eq!(flat.len(), frames as usize * 4);
        assert!(
            flat.chunks_exact(4)
                .any(|p| f32::from_le_bytes(p.try_into().unwrap()) != 0.0)
        );
        if c["codec"] == "sowt" {
            assert_eq!(flat, 0.125f32.to_le_bytes().repeat(frames as usize));
        }
        let mut owned_pcm = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(name)),
            &mut owned_pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, frames as u64);
        assert_eq!(owned_pcm, flat, "{name}: owned export");
        let mut reader = source();
        assert_eq!(reader.duration(), Some(rational::duration(frames, 48000)));
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            rational::duration(frames / 2, 48000),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, (frames, 48000));
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
            assert_eq!(
                got.anchors,
                vec![rational::anchor(&wanted, (frames, 48000))]
            );
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
    }
}
