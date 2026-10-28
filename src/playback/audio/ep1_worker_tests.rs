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
        .chain(m.get("ld_cases").and_then(|v| v.as_array()).into_iter().flatten());
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
        for sample in [13i64, 0, 7, 7, (full.len() / stride) as i64, 13, 0] {
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
