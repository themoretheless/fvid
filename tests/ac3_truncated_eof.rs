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
fn open(bytes: Vec<u8>) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
    )
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
fn decoded(bytes: Vec<u8>) -> Vec<(Duration, Vec<u8>)> {
    let mut source = open(bytes);
    let mut engine = source.make_decoder().unwrap();
    let mut expected = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = engine
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        let packet = source
            .present_decoded(f.packet, f.source_pts)
            .unwrap()
            .unwrap();
        expected.push((packet.presentation_time(), packet.data));
    }
    expected
}
#[test]
fn a_known_truncated_ac3_frame_is_not_hidden_by_the_file_coverage_heuristic() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-truncated-eof");
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let prefix = decoded(std::fs::read(base.join("prefix.ac3")).unwrap());
    assert_eq!(prefix.len(), 15);
    let prefix_pcm = prefix
        .iter()
        .flat_map(|(_, data)| data.iter().copied())
        .collect::<Vec<_>>();
    for file in ["full.ac3", "prefix.ac3", "padded.ac3"] {
        let bytes = std::fs::read(base.join(file)).unwrap();
        let expected = decoded(bytes.clone());
        let source = open(bytes);
        let end = source.duration().unwrap();
        source.validate_eof().unwrap();
        capture::worker_check_with_seek(source, &expected, Some(end));
    }
    let mut failures = Vec::new();
    for file in ["truncated-one.ac3", "truncated-half.ac3"] {
        let bytes = std::fs::read(base.join(file)).unwrap();
        let source = open(bytes.clone());
        assert_eq!(decoded(bytes), prefix);
        let end = source.duration().unwrap();
        match source.validate_eof() {
            Ok(()) => failures.push(format!(
                "{file}: raw successful EOF despite known last frame header"
            )),
            Err(e) => assert!(
                e.to_string().contains("AC-3 final frame is truncated"),
                "{e}"
            ),
        };
        let output = Arc::new(Mutex::new(Vec::new()));
        let out = output.clone();
        let mut playback = fvid::audio_thread::AudioPlayback::start(source, move || {
            Box::new(Recorder {
                pcm: out,
                rate: 0,
                channels: 0,
                at: Duration::ZERO,
            })
        });
        for generation in 0..3 {
            if generation == 1 {
                playback.rewind();
            }
            if generation == 2 {
                playback.seek(end);
            }
            playback.play();
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < until);
                match playback.poll() {
                    Some(fvid::audio_thread::AudioEvent::Error(e)) => {
                        assert!(e.contains("AC-3 final frame is truncated"), "{e}");
                        break;
                    }
                    Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => {
                        failures.push(format!("{file} phase{generation}: worker successful Ended"));
                        break;
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(
                output.lock().unwrap().as_slice(),
                if generation < 2 {
                    prefix_pcm.as_slice()
                } else {
                    &[]
                }
            );
            assert_eq!(playback.position(), end);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
