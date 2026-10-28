//! Full worker path: typed ES group transport, queueing and navigation.
use super::*;
use crate::audio::AudioStream;
fn check(
    packets: &[crate::audio::AudioPacket],
    full: &[u8],
    begin: usize,
    rate: u32,
    stride: usize,
) {
    let actual: Vec<_> = packets
        .iter()
        .flat_map(|p| p.data.iter().copied())
        .collect();
    assert_eq!(actual, full[begin.min(full.len())..]);
    let mut cursor = begin.min(full.len()) / stride;
    for p in packets {
        assert_eq!(
            u128::from(p.pts) * u128::from(p.timebase_num) * u128::from(rate),
            (cursor as u128) * u128::from(p.timebase_den)
        );
        cursor += p.data.len() / stride;
    }
    assert_eq!(cursor, full.len() / stride);
}
fn run(worker: &mut Worker) {
    for _ in 0..256 {
        match worker.decode_next().0 {
            Some(AudioEvent::Ended(_)) => return,
            Some(AudioEvent::Error(e)) => panic!("EP1 worker: {e}"),
            _ => {}
        }
    }
    panic!("EP1 worker did not finish");
}
#[test]
fn ep1_worker_queues_exact_pcm_after_partial_playback_seek_rewind_and_edits() {
    check_corpus("mp4-ep1");
    check_corpus("mp4-ep1-empty");
    check_corpus("mp4-ep1-ps");
    check_corpus("mp4-ep1-epochs");
    check_corpus("mp4-ep1-transitions");
    check_corpus("mp4-ep1-grouped-sbr");
    check_corpus("mp4-ep1-sbr-batches");
    check_corpus("mp4-ep1-grouped-core");
    check_corpus("mp4-ep1-grouped-er");
}
fn check_corpus(corpus: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(corpus);
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["name"].as_str().unwrap().ends_with("-signed"))
        .chain(
            m.get("exports")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten(),
        )
        .chain(
            m.get("ld_cases")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten(),
        );
    for case in cases {
        let bytes = std::fs::read(root.join(case["video"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&bytes),
            &mut full,
            None,
            &Default::default(),
        )
        .unwrap();
        let stream = crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        let scale = stream.timescale();
        let rate = stream.sample_rate();
        let stride = usize::from(stream.channels()) * 4;
        let decoder = stream.make_decoder().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(1);
        let mut worker = Worker {
            output_configuration: None,
            reconfiguration: None,
            output_needs_restart: false,
            duration: Arc::new(Mutex::new(None)),
            checkpoints: Vec::new(),
            dolby_policy: DolbyPlaybackPolicy::default(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(super::presentation_window_tests::Capture(captured.clone())),
            commands,
            events,
            playing: true,
            ended: false,
            draining: false,
            queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        run(&mut worker);
        let actual: Vec<_> = captured
            .lock()
            .unwrap()
            .iter()
            .flat_map(|p| p.data.iter().copied())
            .collect();
        assert_eq!(actual, full, "{} initial worker", case["name"]);
        check(&captured.lock().unwrap(), &full, 0, rate, stride);
        let mut seeks = vec![13i64, 0, 7, 7, (full.len() / stride) as i64, 13, 0];
        if let Some(edges) = case.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                let edge = edge.as_i64().unwrap();
                seeks.extend([edge - 1, edge, edge + 1]);
            }
        }
        for sample in seeks {
            // Leave a live decoder and queue before applying the next seek.
            worker.drain_commands(Some(Command::Rewind));
            let prefix = if case["name"].as_str().unwrap().ends_with("-edits") {
                4
            } else {
                2
            };
            for _ in 0..prefix {
                assert!(!matches!(
                    worker.decode_next().0,
                    Some(AudioEvent::Error(_))
                ));
            }
            assert!(
                !worker.ended,
                "{} ended before live-history seek",
                case["name"]
            );
            captured.lock().unwrap().clear();
            let ticks = (i128::from(sample) * i128::from(scale) / i128::from(rate)) as i64;
            worker.drain_commands(Some(Command::Seek(ticks)));
            run(&mut worker);
            let begin = (u128::try_from(ticks).unwrap() * u128::from(rate))
                .div_ceil(u128::from(scale)) as usize
                * stride;
            let actual: Vec<_> = captured
                .lock()
                .unwrap()
                .iter()
                .flat_map(|p| p.data.iter().copied())
                .collect();
            check(&captured.lock().unwrap(), &full, begin, rate, stride);
            assert_eq!(
                actual,
                full[begin.min(full.len())..],
                "{} worker seek {sample}",
                case["name"]
            );
        }
        captured.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Rewind));
        run(&mut worker);
        let actual: Vec<_> = captured
            .lock()
            .unwrap()
            .iter()
            .flat_map(|p| p.data.iter().copied())
            .collect();
        assert_eq!(actual, full, "{} worker rewind", case["name"]);
    }
}

struct ActiveChainCapture {
    channels: u16,
    packets: Arc<Mutex<Vec<(crate::audio::AudioPacket, u16)>>>,
}
impl crate::audio::AudioBackend for ActiveChainCapture {
    fn start(&mut self, spec: crate::audio::AudioSpec) -> Result<(), crate::audio::AudioError> {
        self.channels = spec.channels;
        Ok(())
    }
    fn reconfigure(
        &mut self,
        spec: crate::audio::AudioSpec,
        router: Option<fvid_media::owned_stereo_router::StereoRouter>,
        _: Duration,
        _: bool,
    ) -> Result<(), crate::audio::AudioError> {
        assert_eq!(spec.sample_rate, 48000);
        assert!(matches!(spec.channels, 1 | 8));
        let router = router.expect("native AAC must retain its speaker routing");
        assert_eq!(router.input_channels(), spec.channels);
        assert_eq!(
            router.source_mask(),
            Some(if spec.channels == 1 { 4 } else { 0xff })
        );
        self.channels = spec.channels;
        Ok(())
    }
    fn push(&mut self, p: crate::audio::AudioPacket) -> Result<(), crate::audio::AudioError> {
        assert!(p.data.len().is_multiple_of(usize::from(self.channels) * 4));
        self.packets.lock().unwrap().push((p, self.channels));
        Ok(())
    }
    fn position(&self) -> Duration {
        Duration::ZERO
    }
    fn flush(&mut self, _: Duration) -> Result<(), crate::audio::AudioError> {
        self.packets.lock().unwrap().clear();
        Ok(())
    }
    fn pause(&mut self) -> Result<(), crate::audio::AudioError> {
        Ok(())
    }
    fn resume(&mut self) -> Result<(), crate::audio::AudioError> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), crate::audio::AudioError> {
        Ok(())
    }
}
#[test]
fn active_esc_chain_worker_reconfigures_native_layout_and_seeks_between_all_epochs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-active-chain");
    let gold: Vec<_> = ["control-0.f32le", "control-1.f32le"]
        .iter()
        .map(|n| std::fs::read(root.join(n)).unwrap())
        .collect();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let names: Vec<_> = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| {
            ["video", "equivalent"]
                .into_iter()
                .map(move |key| c[key].as_str().unwrap())
        })
        .collect();
    for name in names {
        let data = std::fs::read(root.join(name)).unwrap();
        let stream = crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(data),
            Default::default(),
        )
        .unwrap();
        let scale = stream.timescale();
        let decoder = stream.make_decoder().unwrap();
        let output_configuration = Some((
            crate::audio::AudioSpec {
                sample_rate: 48000,
                channels: 1,
                format: crate::audio::SampleFormat::F32,
            },
            stream.stereo_router().unwrap(),
        ));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(1);
        let mut worker = Worker {
            output_configuration,
            reconfiguration: None,
            output_needs_restart: false,
            duration: Arc::new(Mutex::new(None)),
            checkpoints: Vec::new(),
            dolby_policy: Default::default(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(ActiveChainCapture {
                channels: 1,
                packets: captured.clone(),
            }),
            commands,
            events,
            playing: true,
            ended: false,
            draining: false,
            queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        let check = |from: usize| {
            let mut cursor = from.min(17664);
            for (p, channels) in captured.lock().unwrap().iter() {
                assert_eq!(
                    u128::from(p.pts) * u128::from(p.timebase_num) * 48000,
                    cursor as u128 * u128::from(p.timebase_den)
                );
                let phase = if cursor < 5760 {
                    0
                } else if cursor < 11904 {
                    1
                } else {
                    2
                };
                let origin = [0, 5760, 11904][phase];
                let end = [5760, 11904, 17664][phase];
                assert_eq!(*channels, [1, 8, 1][phase]);
                let stride = usize::from(*channels) * 4;
                let frames = p.data.len() / stride;
                assert!(frames > 0 && cursor + frames <= end);
                let expected = &gold[usize::from(phase == 1)]
                    [(cursor - origin) * stride..(cursor - origin + frames) * stride];
                for (a, b) in p.data.chunks_exact(4).zip(expected.chunks_exact(4)) {
                    assert!(
                        (f32::from_le_bytes(a.try_into().unwrap())
                            - f32::from_le_bytes(b.try_into().unwrap()))
                        .abs()
                            < 1e-7
                    );
                }
                cursor += frames;
            }
            assert_eq!(cursor, 17664);
        };
        run(&mut worker);
        check(0);
        for sample in [
            5759i64, 5760, 5761, 11903, 11904, 11905, 17664, 17665, 7, 7, 0,
        ] {
            assert!(worker.drain_commands(Some(Command::Rewind)));
            for _ in 0..3 {
                assert!(!matches!(
                    worker.decode_next().0,
                    Some(AudioEvent::Error(_))
                ));
            }
            assert!(!worker.ended);
            let ticks = (i128::from(sample) * i128::from(scale) / 48000) as i64;
            assert!(worker.drain_commands(Some(Command::Seek(ticks))));
            run(&mut worker);
            check((u128::try_from(ticks).unwrap() * 48000).div_ceil(u128::from(scale)) as usize);
        }
        assert!(worker.drain_commands(Some(Command::Rewind)));
        run(&mut worker);
        check(0);
    }
}
