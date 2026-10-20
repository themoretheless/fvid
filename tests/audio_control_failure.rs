#![cfg(feature = "player")]
use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec};
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Device {
    pcm: Arc<Mutex<Vec<u8>>>,
    total: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    resumes: usize,
    operation: &'static str,
    fail: bool,
}
impl Device {
    fn command(&self, operation: &str) -> Result<(), AudioError> {
        if self.fail && self.operation == operation {
            Err(AudioError::PlaybackError(format!(
                "injected {operation} refusal"
            )))
        } else {
            Ok(())
        }
    }
}
impl AudioBackend for Device {
    fn start(&mut self, _: AudioSpec) -> Result<(), AudioError> {
        Ok(())
    }
    fn push(&mut self, p: AudioPacket) -> Result<(), AudioError> {
        self.total.fetch_add(p.data.len(), Ordering::AcqRel);
        self.pcm.lock().unwrap().extend(p.data);
        Ok(())
    }
    fn position(&self) -> Duration {
        Duration::ZERO
    }
    fn flush(&mut self, _: Duration) -> Result<(), AudioError> {
        self.pcm.lock().unwrap().clear();
        Ok(())
    }
    fn pause(&mut self) -> Result<(), AudioError> {
        self.paused.store(true, Ordering::Release);
        Ok(())
    }
    fn resume(&mut self) -> Result<(), AudioError> {
        self.resumes += 1;
        if self.resumes > 1 {
            self.command("resume")
        } else {
            Ok(())
        }
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn set_volume(&mut self, _: u32) -> Result<(), AudioError> {
        self.command("volume")
    }
    fn set_rate(&mut self, _: u32) -> Result<(), AudioError> {
        self.command("rate")
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        Ok(false)
    }
}
#[test]
fn device_control_refusal_is_terminal_before_followup_navigation() {
    let media = include_bytes!("fixtures/playback-errors/audio-control-failure/control.mkv");
    let expected = include_bytes!("fixtures/playback-errors/audio-control-failure/expected.f32le");
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(media.as_slice()), 32 << 20)
            .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut wrong = Vec::new();
    for operation in ["resume", "volume", "rate"] {
        for fail in [false, true] {
            let source = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(media.to_vec()),
                Default::default(),
            )
            .unwrap();
            let pcm = Arc::new(Mutex::new(Vec::new()));
            let total = Arc::new(AtomicUsize::new(0));
            let stopped = Arc::new(AtomicBool::new(false));
            let paused = Arc::new(AtomicBool::new(false));
            let device = Device {
                pcm: pcm.clone(),
                total: total.clone(),
                stopped: stopped.clone(),
                paused: paused.clone(),
                resumes: 0,
                operation,
                fail,
            };
            let mut playback =
                fvid::audio_thread::AudioPlayback::start(Box::new(source), move || {
                    Box::new(device)
                });
            playback.play();
            let until = Instant::now() + Duration::from_secs(5);
            while pcm.lock().unwrap().len() != expected.len() {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(pcm.lock().unwrap().as_slice(), expected);
            match operation {
                "resume" => {
                    playback.pause();
                    let until = Instant::now() + Duration::from_secs(5);
                    while !paused.load(Ordering::Acquire) {
                        assert!(Instant::now() < until);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    playback.play();
                }
                "volume" => playback.set_volume(500),
                _ => playback.set_rate(1250),
            }
            let until = Instant::now() + Duration::from_millis(100);
            let mut refusal = false;
            while Instant::now() < until {
                if let Some(fvid::audio_thread::AudioEvent::Error(e)) = playback.poll() {
                    assert!(e.contains(&format!("injected {operation} refusal")));
                    refusal = true;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(refusal, fail);
            playback.seek(Duration::from_millis(20));
            playback.play();
            std::thread::sleep(Duration::from_millis(30));
            if fail {
                if !stopped.load(Ordering::Acquire)
                    || total.load(Ordering::Acquire) != expected.len()
                {
                    wrong.push(format!(
                        "{operation}: stopped={}, queued={} expected{}",
                        stopped.load(Ordering::Acquire),
                        total.load(Ordering::Acquire),
                        expected.len()
                    ));
                }
            } else {
                assert_eq!(pcm.lock().unwrap().as_slice(), &expected[640..]);
                assert!(!stopped.load(Ordering::Acquire));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "terminal error left the device/worker active: {wrong:?}"
    );
}
