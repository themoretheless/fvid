#![cfg(feature = "player")]
use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec, AudioStream};
use std::{
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "wav" => Box::new(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "au" => Box::new(
            fvid::playback_au::AuAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        "aiff" => Box::new(
            fvid::playback_aiff::AiffAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
struct Recorder {
    pcm: Arc<Mutex<Vec<u8>>>,
    rate: u32,
    channels: u16,
    at: Duration,
}
impl AudioBackend for Recorder {
    fn start(&mut self, s: AudioSpec) -> Result<(), AudioError> {
        self.rate = s.sample_rate;
        self.channels = s.channels;
        Ok(())
    }
    fn push(&mut self, p: AudioPacket) -> Result<(), AudioError> {
        self.at = p.presentation_time()
            + Duration::from_nanos(
                (p.data.len() / 4 / usize::from(self.channels)) as u64 * 1_000_000_000
                    / u64::from(self.rate),
            );
        self.pcm.lock().unwrap().extend(p.data);
        Ok(())
    }
    fn position(&self) -> Duration {
        self.at
    }
    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.at = at;
        self.pcm.lock().unwrap().clear();
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
#[test]
fn known_missing_audio_payload_reports_incomplete_eof_instead_of_successful_prefix_playback() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-truncated-eof");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let truncated = case["truncated"].as_bool().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let expected = std::fs::read(base.join(case["expected"].as_str().unwrap())).unwrap();
        let mut source = open(kind, bytes.clone());
        let end = source.time_of((expected.len() / 4) as i64);
        assert_eq!(source.duration(), Some(end));
        let mut decoder = source.make_decoder().unwrap();
        let mut actual = Vec::new();
        let mut intervals = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            actual.extend_from_slice(&frame.packet.data);
            intervals.push((frame.packet.presentation_time(), frame.packet.data));
        }
        assert_eq!(actual, expected, "{name} numeric available-prefix control");
        match source.validate_eof() {
            Ok(()) if truncated => wrong.push(format!(
                "{name}: raw EOF declared complete despite a known missing payload"
            )),
            Err(error) if truncated => assert!(
                error.to_string().contains("payload is truncated")
                    && error
                        .to_string()
                        .contains(if kind == "aiff" { "160" } else { "320" })
                    && error
                        .to_string()
                        .contains(if kind == "aiff" { "80" } else { "160" }),
                "{name}: {error}"
            ),
            Ok(()) => {}
            Err(error) => panic!("{name} valid/unknown control failed{error}"),
        }
        if !truncated {
            capture::worker_check_with_seek(open(kind, bytes), &intervals, Some(end));
            continue;
        }
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        let mut playback = fvid::audio_thread::AudioPlayback::start(open(kind, bytes), move || {
            Box::new(Recorder {
                pcm: sink,
                rate: 0,
                channels: 0,
                at: Duration::ZERO,
            })
        });
        for generation in 0..5 {
            if generation == 1 {
                playback.rewind();
            }
            if generation == 2 {
                playback.seek(Duration::ZERO);
            }
            if generation == 3 {
                playback.seek(end);
            }
            if generation == 4 {
                playback.seek(Duration::from_millis(20));
            }
            playback.play();
            let until = Instant::now() + Duration::from_secs(5);
            let mut ended = false;
            loop {
                assert!(Instant::now() < until, "{name} no terminal completion");
                match playback.poll() {
                    Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => {
                        ended = true;
                        break;
                    }
                    Some(fvid::audio_thread::AudioEvent::Error(e)) => {
                        assert!(
                            e.contains("payload is truncated"),
                            "{name} unrelated worker error{e}"
                        );
                        break;
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(
                *output.lock().unwrap(),
                if generation < 3 {
                    expected.as_slice()
                } else {
                    &[]
                },
                "{name} available native prefix remains usable"
            );
            assert_eq!(playback.position(), end);
            if ended {
                wrong.push(format!(
                    "{name} generation{generation}: reported successful Ended after{}frames",
                    output.lock().unwrap().len() / 4
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "known truncated payload completed as success: {wrong:?}"
    );
}
