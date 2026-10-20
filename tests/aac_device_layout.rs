#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn open(name: &str, ext: &str) -> Box<dyn AudioStream> {
    let data = std::fs::read(if ext == "aac" {
        root().join(format!("aac-height-{name}.aac"))
    } else {
        root().join(format!("aac-device-layout/{name}.{ext}"))
    })
    .unwrap();
    match ext {
        "aac" => Box::new(
            fvid::playback_aac::AacAudioReader::open(data.as_slice(), Default::default()).unwrap(),
        ),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(data), Default::default())
                .unwrap(),
        ),
        _ => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap(),
        ),
    }
}
#[test]
fn authored_aac_layout_containers_decode_before_device_routing_acceptance() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut missing = Vec::new();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let oracle = std::fs::read(root().join(format!("aac-height-{name}.f32le"))).unwrap();
        for ext in ["aac", "mkv", "mp4"] {
            let mut stream = open(name, ext);
            let mut decoder = stream.make_decoder().unwrap();
            let mut pcm = Vec::new();
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    pcm.extend(frame.packet.data);
                }
            }
            assert_eq!(pcm.len(), oracle.len(), "{name}/{ext}");
            for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 1e-6, "{name}/{ext}: {a} vs {b}");
            }
            if ext != "aac" {
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(
                        std::fs::read(root().join(format!("aac-device-layout/{name}.{ext}")))
                            .unwrap(),
                    ),
                    32 << 20,
                )
                .unwrap();
                let mut frames = 0;
                while reader.read_frame_raw().unwrap().is_some() {
                    frames += 1;
                }
                assert!(frames > 0, "{name}/{ext}");
            }
            if stream.stereo_router().unwrap().is_none() {
                missing.push(format!("{name}/{ext}"));
            }
        }
    }
    assert!(missing.is_empty(), "AAC device layout absent: {missing:?}");
}
fn same_stereo(actual: &[f32], expected: &[u8], name: &str) {
    assert_eq!(actual.len() * 4, expected.len(), "{name}");
    if expected.is_empty() {
        return;
    }
    let mut squared = 0f64;
    let mut peak = 0f32;
    for (&a, b) in actual.iter().zip(expected.chunks_exact(4)) {
        let b = f32::from_le_bytes(b.try_into().unwrap());
        let d = (a - b).abs();
        peak = peak.max(d);
        squared += f64::from(d).powi(2);
    }
    assert!(
        peak < 2e-6 && (squared / actual.len() as f64).sqrt() < 4e-7,
        "{name}: stereo peak {peak}"
    );
}
#[test]
fn emitted_aac_positions_route_to_independent_authored_stereo_oracles() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let expected =
            std::fs::read(root().join(format!("aac-device-layout/{name}.stereo.f32le"))).unwrap();
        for ext in ["aac", "mkv", "mp4"] {
            let mut stream = open(name, ext);
            let router = stream.stereo_router().unwrap().expect("explicit AAC route");
            assert_eq!(
                router.source_mask(),
                None,
                "PCE position order must not be reduced to a mask"
            );
            let mut decoder = stream.make_decoder().unwrap();
            let mut actual = Vec::new();
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    actual.extend(router.route(&frame.packet.data, 32 << 20).unwrap());
                }
            }
            same_stereo(&actual, &expected, &format!("{name}/{ext}"));
        }
    }
}
#[test]
fn routed_aac_workers_preserve_pcm_and_exact_sample_clocks_on_rewind_and_seek() {
    use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec};
    use fvid_media::owned_stereo_router::StereoRouter;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    struct Capture {
        router: Option<StereoRouter>,
        at: Duration,
        captured: Arc<Mutex<Vec<(Duration, Vec<f32>)>>>,
    }
    impl AudioBackend for Capture {
        fn start(&mut self, _: AudioSpec) -> Result<(), AudioError> {
            panic!("typed AAC route was lost")
        }
        fn start_with_router(
            &mut self,
            spec: AudioSpec,
            router: Option<StereoRouter>,
        ) -> Result<(), AudioError> {
            let router = router.expect("AAC route");
            assert_eq!(router.input_channels(), spec.channels);
            assert_eq!(spec.sample_rate, 48000);
            self.router = Some(router);
            Ok(())
        }
        fn push(&mut self, p: AudioPacket) -> Result<(), AudioError> {
            let pcm = self
                .router
                .as_ref()
                .unwrap()
                .route(&p.data, 32 << 20)
                .unwrap();
            let frame = (u128::from(p.pts) * u128::from(p.timebase_num) * 48000
                + u128::from(p.timebase_den) / 2)
                / u128::from(p.timebase_den);
            self.at =
                Duration::from_nanos(((frame + pcm.len() as u128 / 2) * 1000000000 / 48000) as u64);
            self.captured
                .lock()
                .unwrap()
                .push((p.presentation_time(), pcm));
            Ok(())
        }
        fn position(&self) -> Duration {
            self.at
        }
        fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
            self.at = at;
            self.captured.lock().unwrap().clear();
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn resume(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), AudioError> {
            Ok(())
        }
    }
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let expected =
            std::fs::read(root().join(format!("aac-device-layout/{name}.stereo.f32le"))).unwrap();
        for ext in ["aac", "mkv", "mp4"] {
            let captured = Arc::new(Mutex::new(Vec::new()));
            let sink = captured.clone();
            let mut playback =
                fvid::audio_thread::AudioPlayback::start(open(name, ext), move || {
                    Box::new(Capture {
                        router: None,
                        at: Duration::ZERO,
                        captured: sink,
                    })
                });
            for generation in 0..4 {
                match generation {
                    1 => playback.rewind(),
                    2 => playback.seek(Duration::from_millis(2)),
                    3 => playback.seek(Duration::from_millis(50)),
                    _ => {}
                }
                playback.play();
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    assert!(Instant::now() < deadline, "{name}/{ext}: timeout");
                    match playback.poll() {
                        Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => break,
                        Some(fvid::audio_thread::AudioEvent::Error(e)) => {
                            panic!("{name}/{ext}: {e}")
                        }
                        _ => {}
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                let packets = captured.lock().unwrap();
                assert_eq!(
                    packets.len(),
                    if generation == 3 { 4 } else { 6 },
                    "{name}/{ext}"
                );
                for (i, p) in packets.iter().enumerate() {
                    let wanted = if generation == 3 && i == 0 {
                        Duration::from_millis(50)
                    } else if generation == 2 && i == 0 {
                        Duration::from_millis(2)
                    } else {
                        Duration::from_nanos(
                            (i + if generation == 3 { 2 } else { 0 }) as u64 * 1024 * 1000000000
                                / 48000,
                        )
                    };
                    assert_eq!(p.0, wanted, "{name}/{ext} packet{i}");
                }
                let actual: Vec<_> = packets.iter().flat_map(|p| p.1.iter().copied()).collect();
                same_stereo(
                    &actual,
                    if generation == 3 {
                        &expected[2400 * 2 * 4..]
                    } else if generation == 2 {
                        &expected[96 * 2 * 4..]
                    } else {
                        &expected
                    },
                    &format!("{name}/{ext}/generation{generation}"),
                );
                assert_eq!(playback.position(), Duration::from_millis(128));
            }
        }
    }
}
#[test]
fn positive_mp4_edits_apply_priming_once_before_exact_seek_and_stereo_fold() {
    use std::time::Duration;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["edits"].as_array().unwrap().len(), 2);
    for c in manifest["edits"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let base = c["base"].as_str().unwrap();
        let source = std::fs::read(root().join("aac-device-layout").join(name)).unwrap();
        let expected =
            std::fs::read(root().join(format!("aac-device-layout/{base}.stereo.f32le"))).unwrap();
        let head = c["head"].as_u64().unwrap() as usize;
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(source.clone()), 32 << 20)
                .unwrap();
        let mut pictures = 0;
        while video.read_frame_raw().unwrap().is_some() {
            pictures += 1;
        }
        assert!(pictures > 0, "{name}");
        for requested in [0usize, 96, 777, 6095, 6096, 9999] {
            let target = requested.min(6096);
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(source.clone()),
                Default::default(),
            )
            .unwrap();
            let landed = stream.seek_to(requested as i64);
            assert_eq!(landed, head as i64 + target as i64);
            assert_eq!(
                stream.time_of(landed),
                Duration::from_nanos(target as u64 * 1000000000 / 48000)
            );
            assert_eq!(stream.duration(), Some(Duration::from_millis(127)));
            let router = stream.stereo_router().unwrap().unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut first = None;
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(pcm) = stream
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        first.get_or_insert(pcm.presentation_time());
                        actual.extend(router.route(&pcm.data, 32 << 20).unwrap());
                    }
                }
            }
            assert_eq!(
                first,
                (target < 6096).then(|| Duration::from_nanos(target as u64 * 1000000000 / 48000))
            );
            same_stereo(&actual, &expected[(head + target) * 2 * 4..], name);
        }
    }
}
#[test]
fn quantized_matroska_codec_delay_preserves_exact_pcm_and_seek_samples() {
    use std::time::Duration;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-device-layout/manifest.json")).unwrap(),
    )
    .unwrap();
    let cases = manifest["delays"].as_array().unwrap();
    assert_eq!(cases.len(), 2);
    for c in cases {
        let name = c["file"].as_str().unwrap();
        let base = c["base"].as_str().unwrap();
        let data = std::fs::read(root().join("aac-device-layout").join(name)).unwrap();
        let expected =
            std::fs::read(root().join(format!("aac-device-layout/{base}.stereo.f32le"))).unwrap();
        let head = c["head"].as_u64().unwrap() as usize;
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(data.clone()), 32 << 20)
                .unwrap();
        let mut frames = 0;
        while video.read_frame_raw().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 6);
        for target in [0usize, 96, 2400] {
            let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(data.clone()),
                Default::default(),
            )
            .unwrap();
            let requested = target as u64 * 1000000000 / 48000;
            assert_eq!(stream.seek_to(requested as i64), requested as i64);
            let router = stream.stereo_router().unwrap().unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut first = None;
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                {
                    if let Some(pcm) = stream
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        first.get_or_insert(pcm.presentation_time());
                        actual.extend(router.route(&pcm.data, 32 << 20).unwrap());
                    }
                }
            }
            assert_eq!(first, Some(Duration::from_nanos(requested)));
            same_stereo(&actual, &expected[(head + target) * 2 * 4..], name);
        }
    }
}
