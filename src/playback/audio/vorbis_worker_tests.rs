//! Worker acceptance for linked source layouts and stable backend output.
use super::*;
use crate::audio::{AudioPacket, AudioSpec, AudioStream, SampleFormat};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Capture {
    pcm: Arc<Mutex<Vec<AudioPacket>>>,
    starts: Arc<AtomicUsize>,
    channels: u16,
    rate: u32,
}
impl AudioBackend for Capture {
    fn start(&mut self, spec: AudioSpec) -> Result<(), crate::audio::AudioError> {
        assert_eq!(
            (spec.sample_rate, spec.channels),
            (self.rate, self.channels)
        );
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn push(&mut self, packet: AudioPacket) -> Result<(), crate::audio::AudioError> {
        assert_eq!(packet.data.len() % (usize::from(self.channels) * 4), 0);
        self.pcm.lock().unwrap().push(packet);
        Ok(())
    }
    fn position(&self) -> Duration {
        Duration::ZERO
    }
    fn flush(&mut self, _: Duration) -> Result<(), crate::audio::AudioError> {
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
fn run(worker: &mut Worker) {
    for _ in 0..64 {
        match worker.decode_next().0 {
            Some(AudioEvent::Ended(_)) => return,
            Some(AudioEvent::Error(error)) => panic!("Vorbis worker: {error}"),
            _ => {}
        }
    }
    panic!("Vorbis worker did not terminate");
}
#[test]
fn ogg_vorbis_channel_changes_keep_backend_geometry_through_seek_and_rewind() {
    for (first, second, channels) in [
        (1, 2, 3),
        (2, 1, 3),
        (4, 6, 6),
        (7, 8, 9),
        (9, 33, 33),
        (33, 9, 33),
        (2, 255, 255),
    ] {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(format!("vorbis-channel-chain-{first}-{second}.ogg")),
        )
        .unwrap();
        let stream =
            crate::playback_ogg_audio::OggAudioReader::open(bytes.as_slice(), Default::default())
                .unwrap();
        let decoder = stream.make_decoder().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(AtomicUsize::new(0));
        let mut backend = Capture {
            pcm: captured.clone(),
            starts: starts.clone(),
            channels,
            rate: 8000,
        };
        backend
            .start(AudioSpec {
                sample_rate: 8000,
                channels,
                format: SampleFormat::F32,
            })
            .unwrap();
        let (_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(8);
        let mut worker = Worker {
            duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(backend),
            commands,
            events,
            playing: true,
            ended: false,
            draining: false, queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        let check = |from: u64| {
            let packets = captured.lock().unwrap();
            let mut cursor = from;
            for packet in packets.iter() {
                assert_eq!(packet.pts, cursor);
                assert_eq!((packet.timebase_num, packet.timebase_den), (1, 8000));
                assert!(packet.data.iter().all(|&b| b == 0));
                cursor += packet.data.len() as u64 / (u64::from(channels) * 4);
            }
            assert_eq!(cursor, 822);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
        };
        run(&mut worker);
        check(0);
        for target in [19, 410, 411, 430, 821, 822, 0] {
            captured.lock().unwrap().clear();
            assert!(worker.drain_commands(Some(Command::Seek(target))));
            run(&mut worker);
            check(target as u64);
        }
        captured.lock().unwrap().clear();
        assert!(worker.drain_commands(Some(Command::Rewind)));
        run(&mut worker);
        check(0);
    }
}

#[test]
fn ogg_vorbis_rate_changes_keep_backend_clock_through_seek_and_rewind() {
    for (first, second) in [
        (8000u32, 12000u32),
        (12000, 8000),
        (44100, 48000),
        (48000, 44100),
    ] {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(format!("vorbis-rate-chain-{first}-{second}-offset-0.ogg")),
        )
        .unwrap();
        let stream =
            crate::playback_ogg_audio::OggAudioReader::open(bytes.as_slice(), Default::default())
                .unwrap();
        let decoder = stream.make_decoder().unwrap();
        let end = 411 + (411u64 * u64::from(first)).div_ceil(u64::from(second));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(AtomicUsize::new(0));
        let mut backend = Capture {
            pcm: captured.clone(),
            starts: starts.clone(),
            channels: 2,
            rate: first,
        };
        backend
            .start(AudioSpec {
                sample_rate: first,
                channels: 2,
                format: SampleFormat::F32,
            })
            .unwrap();
        let (_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(8);
        let mut worker = Worker {
            duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(backend),
            commands,
            events,
            playing: true,
            ended: false,
            draining: false, queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        let check = |from: u64| {
            let mut cursor = from;
            for packet in captured.lock().unwrap().iter() {
                assert_eq!(packet.pts, cursor);
                assert_eq!((packet.timebase_num, packet.timebase_den), (1, first));
                cursor += packet.data.len() as u64 / 8;
            }
            assert_eq!(cursor, end);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
        };
        run(&mut worker);
        check(0);
        for target in [19, 410, 411, end - 1, end, 0] {
            captured.lock().unwrap().clear();
            assert!(worker.drain_commands(Some(Command::Seek(target as i64))));
            run(&mut worker);
            check(target);
        }
        captured.lock().unwrap().clear();
        assert!(worker.drain_commands(Some(Command::Rewind)));
        run(&mut worker);
        check(0);
    }
}

#[test]
fn ogg_codec_changes_keep_backend_pcm_clock_through_navigation() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (name, rate, boundary) in [("vorbis-opus", 8000, 411), ("opus-vorbis", 48000, 5311)] {
        let path = root.join(format!("ogg-codec-chain-{name}.ogg"));
        let bytes = std::fs::read(&path).unwrap();
        let mut library = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            0,
        )
        .unwrap();
        let mut expected = Vec::new();
        library.decode_pcm(&mut expected).unwrap();
        let end = expected.len() / 24;
        let stream = crate::playback_ogg_catalog::Reader::open(&path, 0).unwrap();
        let decoder = stream.make_decoder().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(AtomicUsize::new(0));
        let mut backend = Capture {
            pcm: captured.clone(),
            starts: starts.clone(),
            channels: 6,
            rate,
        };
        backend
            .start(AudioSpec {
                sample_rate: rate,
                channels: 6,
                format: SampleFormat::F32,
            })
            .unwrap();
        let (_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(8);
        let mut worker = Worker {
            duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(backend),
            commands,
            events,
            playing: true,
            ended: false,
            draining: false, queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        let check = |from: usize| {
            let mut cursor = from as u64;
            let mut actual = Vec::new();
            for pcm in captured.lock().unwrap().iter() {
                assert_eq!(pcm.pts, cursor);
                assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
                cursor += pcm.data.len() as u64 / 24;
                actual.extend_from_slice(&pcm.data);
            }
            assert_eq!(cursor, end as u64);
            assert_eq!(actual, expected[from * 24..]);
            assert_eq!(starts.load(Ordering::SeqCst), 1);
        };
        run(&mut worker);
        check(0);
        for target in [19, boundary - 1, boundary, boundary + 1, end - 1, end, 0] {
            captured.lock().unwrap().clear();
            assert!(worker.drain_commands(Some(Command::Seek(target as i64))));
            run(&mut worker);
            check(target);
        }
        captured.lock().unwrap().clear();
        assert!(worker.drain_commands(Some(Command::Rewind)));
        run(&mut worker);
        check(0);
    }
}
