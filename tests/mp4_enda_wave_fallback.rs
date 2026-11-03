use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-enda-wave-fallback")
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
fn decode(n: &str, wide: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if wide {
        fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
    } else {
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 2880);
    }
    out
}
#[test]
fn direct_enda_and_big_endian_wave_controls_have_independent_exact_pcm() {
    for c in cases() {
        let control = c["control"].as_str().unwrap();
        assert_eq!(
            decode(control, false),
            bytes(c["expected"].as_str().unwrap()),
            "{control}"
        );
        assert_eq!(
            decode(control, true),
            bytes(c["expected64"].as_str().unwrap()),
            "{control}"
        );
        if !c["little"].as_bool().unwrap() {
            let n = c["file"].as_str().unwrap();
            assert_eq!(decode(n, false), bytes(c["expected"].as_str().unwrap()));
            assert_eq!(decode(n, true), bytes(c["expected64"].as_str().unwrap()));
        }
    }
    let name = cases()[0]["control"].as_str().unwrap().to_owned();
    let video = fvid_media::decode_video(&root().join(name)).unwrap();
    assert_eq!(video.video_frames, 3);
    assert_eq!(video.decode_errors, 0);
}
#[test]
fn unrelated_wave_must_not_hide_the_explicit_direct_pcm_byte_order() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap();
        let cfg = &reader.tracks()[1].configuration;
        let correct = vec![u8::from(c["little"].as_bool().unwrap())];
        let a = decode(name, false);
        let b = decode(name, true);
        if cfg != &correct
            || a != bytes(c["expected"].as_str().unwrap())
            || b != bytes(c["expected64"].as_str().unwrap())
        {
            failures.push(format!(
                "{name}:cfg{cfg:?},expected{correct:?},f32={},f64={}",
                a == bytes(c["expected"].as_str().unwrap()),
                b == bytes(c["expected64"].as_str().unwrap())
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} byte-order failures: {failures:?}",
        failures.len()
    );
}
#[test]
fn malformed_direct_enda_cannot_bypass_existing_strict_refusals() {
    let m = manifest();
    let mut admitted = Vec::new();
    for c in m["bad"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let control = format!("control-{name}");
        let original = match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(&control)),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("bad direct control admitted"),
        };
        assert!(matches!(original, fvid::Error::Invalid(_)));
        match fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Err(e) => {
                assert!(matches!(e, fvid::Error::Invalid(_)));
                assert_eq!(e.to_string(), original.to_string(), "{name}");
            }
            Ok(_) => admitted.push(name.to_owned()),
        }
        let original = match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(&control)),
            Default::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("bad owned control admitted"),
        };
        assert!(!original.is_unsupported());
        if let Err(e) = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            assert!(!e.is_unsupported());
            assert_eq!(e.to_string(), original.to_string());
        } else {
            admitted.push(format!("owned:{name}"));
        }
    }
    assert!(
        admitted.is_empty(),
        "malformed direct flags hidden by wave: {admitted:?}"
    );
}
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("fvid-enda-wave-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn public_exports_intervals_and_resource_refusals_keep_exact_pcm() {
    let dir = Temp::new();
    for (i, c) in cases().into_iter().enumerate() {
        let name = c["file"].as_str().unwrap();
        let source = root().join(name);
        let opts = fvid_media::CopyOptions {
            max_controlled_bytes: Some(64 << 20),
            ..Default::default()
        };
        fvid_media::plan_decode_audio(&source, &Default::default(), &opts).unwrap();
        let out = dir.0.join(format!("{i}.f32le"));
        let stats = fvid::media::decode_audio(&source, &out, &opts).unwrap();
        assert_eq!(stats.sample_frames, 2880);
        assert_eq!(
            std::fs::read(out).unwrap(),
            bytes(c["expected"].as_str().unwrap())
        );
        for from in [
            std::time::Duration::from_nanos(1),
            std::time::Duration::from_millis(20),
        ] {
            let first = if from.as_nanos() == 1 { 1 } else { 960 };
            let interval = Some((from, std::time::Duration::from_millis(60)));
            let ch = c["channels"].as_u64().unwrap() as usize;
            let mut out = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &opts,
            )
            .unwrap();
            assert_eq!(
                out,
                bytes(c["expected"].as_str().unwrap())[first * ch * 4..],
                "{name}"
            );
            out.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(bytes(name)),
                &mut out,
                interval,
                &opts,
            )
            .unwrap();
            assert_eq!(
                out,
                bytes(c["expected64"].as_str().unwrap())[first * ch * 8..],
                "{name} f64"
            );
        }
        let tight = fvid_media::CopyOptions {
            max_packet_bytes: 1,
            ..opts.clone()
        };
        let mut out = Vec::new();
        let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(name)),
            &mut out,
            None,
            &tight,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("packet"), "{e}");
        assert!(out.is_empty());
        let tiny = fvid_media::CopyOptions {
            max_controlled_bytes: Some(1),
            ..opts.clone()
        };
        assert!(fvid_media::plan_decode_audio(&source, &Default::default(), &tiny).is_err());
        let cancel = fvid::media_control::CancelFlag::new();
        cancel.cancel();
        let canceled = fvid_media::CopyOptions {
            cancel: Some(cancel),
            ..opts
        };
        assert!(
            fvid_media::plan_decode_audio(&source, &Default::default(), &canceled)
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
fn actual_playback_preserves_pcm_byte_order_clocks_seek_and_video() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let open = |n: &str| {
            fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes(n)),
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
            bytes(c["expected"].as_str().unwrap())
        );
        let mut reader = open(name);
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
            Cursor::new(bytes(name)),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for i in 0..3 {
            let frame = video.read_frame().unwrap().unwrap();
            assert_eq!(
                frame
                    .picture
                    .y
                    .iter()
                    .chain(&frame.picture.cb)
                    .chain(&frame.picture.cr)
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
                bytes(&format!("frame-{i}.yuv16"))
            );
        }
        assert!(video.read_frame().unwrap().is_none());
    }
}
