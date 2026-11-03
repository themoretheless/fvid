use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-unused-expansion-budget")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn check(c: &serde_json::Value, budget: bool) -> Result<(), String> {
    let name = c["file"].as_str().unwrap();
    let source = std::fs::read(root().join(name)).unwrap();
    let expected = std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
    let opts = fvid_media::CopyOptions {
        max_controlled_bytes: budget.then(|| c["budget"].as_u64().unwrap() as usize),
        ..Default::default()
    };
    let mut out = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&source),
        &mut out,
        None,
        &opts,
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(stats.sample_frames, 480);
    assert_eq!(out, expected, "{name}");
    out.clear();
    fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(Cursor::new(&source), &mut out, None, &opts)
        .map_err(|e| e.to_string())?;
    let wide: Vec<_> = expected
        .chunks_exact(4)
        .flat_map(|b| (f32::from_le_bytes(b.try_into().unwrap()) as f64).to_le_bytes())
        .collect();
    assert_eq!(out, wide, "{name}");
    Ok(())
}
#[test]
fn exact_pcm_catalogs_and_loose_budget_controls_are_valid() {
    let m = manifest();
    for c in m["controls"].as_array().unwrap() {
        check(c, true).unwrap();
    }
    for c in m["cases"].as_array().unwrap() {
        check(c, false).unwrap();
    }
}
#[test]
fn unused_mace_cannot_charge_pcm_for_a_decoder_it_never_uses() {
    let mut failures = Vec::new();
    for c in manifest()["cases"].as_array().unwrap() {
        if let Err(e) = check(c, true) {
            failures.push(format!("{}:{e}", c["file"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} false memory refusals: {failures:?}",
        failures.len()
    );
}
#[test]
fn public_admission_keeps_index_memory_and_cancellation_checks() {
    for c in manifest()["cases"].as_array().unwrap() {
        let source = root().join(c["file"].as_str().unwrap());
        let opts = fvid_media::CopyOptions {
            max_controlled_bytes: Some(c["budget"].as_u64().unwrap() as usize),
            ..Default::default()
        };
        fvid_media::plan_decode_audio(&source, &Default::default(), &opts).unwrap();
        let tiny = fvid_media::CopyOptions {
            max_controlled_bytes: Some(1),
            ..opts.clone()
        };
        assert!(fvid_media::plan_decode_audio(&source, &Default::default(), &tiny).is_err());
        let cancel = fvid::media_control::CancelFlag::new();
        cancel.cancel();
        let opts = fvid_media::CopyOptions {
            cancel: Some(cancel),
            ..opts
        };
        assert!(
            fvid_media::plan_decode_audio(&source, &Default::default(), &opts)
                .unwrap_err()
                .contains("cancel")
        );
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
fn native_selected_pcm_workers_navigation_and_pictures_are_unchanged() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let open = |n: &str| {
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(std::fs::read(root().join(n)).unwrap()),
                Default::default(),
            )
            .unwrap()
        };
        let mut control = open(c["control"].as_str().unwrap());
        let full = rational::collect(&mut control);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap()
        );
        let mut reader = open(name);
        assert_eq!(reader.duration(), Some(Duration::from_millis(60)));
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(20),
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
        for at in [Duration::ZERO, Duration::from_millis(20), Duration::MAX] {
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let wanted = rational::suffix(&full, at, (3, 50));
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, name);
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, (3, 50))]);
        }
        let mut video = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(std::fs::read(root().join(name)).unwrap()),
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
                std::fs::read(root().join(format!("frame-{i}.yuv16"))).unwrap()
            );
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
#[test]
fn selected_mace_keeps_its_full_packet_expansion_reservation() {
    use fvid_media::owned_mace::{MaceCoding, MaceDecoder};
    for c in manifest()["active_mace"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let input = std::fs::read(root().join(name)).unwrap();
        let code = std::fs::read(root().join(c["coded"].as_str().unwrap())).unwrap();
        let channels = c["channels"].as_u64().unwrap() as u16;
        let coding = if c["codec"] == "MAC3" {
            MaceCoding::ThreeToOne
        } else {
            MaceCoding::SixToOne
        };
        let mut decoder = MaceDecoder::new(coding, 8000, channels).unwrap();
        let expected: Vec<_> = decoder
            .decode_pcm(&code)
            .unwrap()
            .into_iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(expected.len(), 9216 * usize::from(channels) * 4);
        assert!(expected.iter().any(|b| *b != 0));
        let generous = fvid_media::CopyOptions {
            max_controlled_bytes: Some(256 << 10),
            ..Default::default()
        };
        let mut out = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&input),
            &mut out,
            None,
            &generous,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 9216);
        assert_eq!(out, expected, "{name}");
        let tight = fvid_media::CopyOptions {
            max_controlled_bytes: Some(c["budget"].as_u64().unwrap() as usize),
            ..generous
        };
        out.clear();
        let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&input),
            &mut out,
            None,
            &tight,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("controlled memory budget exceeded"),
            "{name}:{e}"
        );
        assert!(out.is_empty());
    }
}
