#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn bare_and_matroska_flac_streams_expose_comment_masks_to_device_negotiation() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-layout");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expected = Some(case["mask"].as_u64().unwrap() as u32);
        let bare = fvid::playback_flac::FlacAudioReader::open(
            std::fs::File::open(root.join(format!("{name}.flac"))).unwrap(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(bare.speaker_mask().unwrap(), expected, "{name}/raw");
        let matroska = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(std::fs::read(root.join(format!("{name}.mkv"))).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            matroska.speaker_mask().unwrap(),
            expected,
            "{name}/Matroska"
        );
    }
}
#[test]
fn audio_worker_delivers_flac_layout_and_preserves_routed_sample_clock_on_rewind() {
    use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec};
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    struct Capture {
        mask: Option<u32>,
        source_channels: u16,
        at: Duration,
        captured: Arc<Mutex<Vec<(Duration, Vec<f32>)>>>,
        router: Option<fvid_media::owned_stereo_router::StereoRouter>,
    }
    impl AudioBackend for Capture {
        fn start(&mut self, _: AudioSpec) -> Result<(), AudioError> {
            panic!("worker must negotiate the source layout")
        }
        fn start_with_layout(
            &mut self,
            spec: AudioSpec,
            mask: Option<u32>,
        ) -> Result<(), AudioError> {
            assert_eq!(mask, self.mask);
            assert_eq!(spec.channels, self.source_channels);
            self.router = Some(
                fvid_media::owned_stereo_router::StereoRouter::new(spec.channels, mask.unwrap())
                    .unwrap(),
            );
            Ok(())
        }
        fn push(&mut self, p: AudioPacket) -> Result<(), AudioError> {
            let pcm = self
                .router
                .as_ref()
                .unwrap()
                .route(&p.data, 32 << 20)
                .unwrap();
            self.at = p.presentation_time()
                + Duration::from_nanos((pcm.len() / 2) as u64 * 1_000_000_000 / 8000);
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
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-layout");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        for ext in ["flac", "mkv"] {
            let data = std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
            let mut stream: Box<dyn AudioStream> = if ext == "flac" {
                Box::new(
                    fvid::playback_flac::FlacAudioReader::open(data.as_slice(), Default::default())
                        .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(data),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            let source_channels = stream.channels();
            let mask = stream.speaker_mask().unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut expected = Vec::new();
            while let Some(p) = stream.next_packet().unwrap() {
                if let Some(pcm) = decoder
                    .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                    .unwrap()
                {
                    expected.push((
                        pcm.presentation_time(),
                        fvid_media::owned_stereo_router::StereoRouter::new(
                            source_channels,
                            mask.unwrap(),
                        )
                        .unwrap()
                        .route(&pcm.data, 32 << 20)
                        .unwrap(),
                    ));
                }
            }
            stream.rewind();
            let captured = Arc::new(Mutex::new(Vec::new()));
            let sink = captured.clone();
            let mut playback = fvid::audio_thread::AudioPlayback::start(stream, move || {
                Box::new(Capture {
                    mask,
                    source_channels,
                    at: Duration::ZERO,
                    captured: sink,
                    router: None,
                })
            });
            for generation in 0..2 {
                if generation == 1 {
                    playback.rewind();
                }
                playback.play();
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    assert!(Instant::now() < deadline, "{name}/{ext}: worker timeout");
                    match playback.poll() {
                        Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => break,
                        Some(fvid::audio_thread::AudioEvent::Error(e)) => {
                            panic!("{name}/{ext}: {e}")
                        }
                        _ => {}
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(*captured.lock().unwrap(), expected, "{name}/{ext}");
                assert_eq!(playback.position(), Duration::from_millis(8));
            }
        }
    }
}
