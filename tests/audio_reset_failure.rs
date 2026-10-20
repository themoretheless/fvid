#![cfg(feature = "player")]
use fvid::audio::{AudioBackend, AudioDecode, AudioError, AudioPacket, AudioSpec, AudioStream};
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
struct Source {
    inner: fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>>,
    fail: bool,
    resets: Arc<AtomicUsize>,
}
impl AudioStream for Source {
    fn codec(&self) -> &str {
        self.inner.codec()
    }
    fn timescale(&self) -> u32 {
        self.inner.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn bits_per_sample(&self) -> u16 {
        self.inner.bits_per_sample()
    }
    fn extra_data(&self) -> &[u8] {
        self.inner.extra_data()
    }
    fn audio_tracks(&self) -> Vec<fvid::audio::AudioTrack> {
        self.inner.audio_tracks()
    }
    fn next_packet(&mut self) -> fvid::Result<Option<fvid::audio::EncodedPacket>> {
        self.inner.next_packet()
    }
    fn present_decoded(&self, p: AudioPacket, pts: i64) -> fvid::Result<Option<AudioPacket>> {
        self.inner.present_decoded(p, pts)
    }
    fn rewind(&mut self) {
        AudioStream::rewind(&mut self.inner);
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.inner.seek_to(pts)
    }
    fn reset_decoder(&mut self, decoder: &mut Box<dyn AudioDecode>) -> fvid::Result<()> {
        self.resets.fetch_add(1, Ordering::AcqRel);
        if self.fail {
            Err(std::io::Error::other("injected decoder reset refusal").into())
        } else {
            self.inner.reset_decoder(decoder)
        }
    }
}
struct Device {
    pcm: Arc<Mutex<Vec<u8>>>,
    total: Arc<AtomicUsize>,
    flushed: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
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
        self.flushed.store(true, Ordering::Release);
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
        self.stopped.store(true, Ordering::Release);
        Ok(())
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        Ok(false)
    }
}
#[test]
fn decoder_reset_refusal_stops_navigation_before_device_flush_or_clock_publication() {
    let media = include_bytes!("fixtures/playback-errors/audio-reset-failure/navigation.mkv");
    let expected = include_bytes!("fixtures/playback-errors/audio-reset-failure/expected.f32le");
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(media.as_slice()), 32 << 20)
            .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut wrong = Vec::new();
    for rewind in [false, true] {
        for fail in [false, true] {
            let inner = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(media.to_vec()),
                Default::default(),
            )
            .unwrap();
            let pcm = Arc::new(Mutex::new(Vec::new()));
            let total = Arc::new(AtomicUsize::new(0));
            let flushed = Arc::new(AtomicBool::new(false));
            let stopped = Arc::new(AtomicBool::new(false));
            let device = Device {
                pcm: pcm.clone(),
                total: total.clone(),
                flushed: flushed.clone(),
                stopped: stopped.clone(),
            };
            let resets = Arc::new(AtomicUsize::new(0));
            let reset_count = resets.clone();
            let mut playback = fvid::audio_thread::AudioPlayback::start(
                Box::new(Source {
                    inner,
                    fail,
                    resets,
                }),
                move || Box::new(device),
            );
            playback.play();
            let until = Instant::now() + Duration::from_secs(5);
            while pcm.lock().unwrap().len() != expected.len() {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(pcm.lock().unwrap().as_slice(), expected);
            if rewind {
                playback.rewind();
            } else {
                playback.seek(Duration::from_millis(20));
            }
            let until = Instant::now() + Duration::from_millis(100);
            let mut refusal = false;
            while Instant::now() < until {
                if let Some(fvid::audio_thread::AudioEvent::Error(e)) = playback.poll() {
                    assert!(e.contains("injected decoder reset refusal"));
                    refusal = true;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if fail {
                assert!(refusal);
                playback.play();
                playback.seek(Duration::ZERO);
                std::thread::sleep(Duration::from_millis(10));
                assert_eq!(
                    reset_count.load(Ordering::Acquire),
                    1,
                    "terminal navigation error must not restart decoding"
                );
                assert_eq!(total.load(Ordering::Acquire), expected.len());
                if !stopped.load(Ordering::Acquire)
                    || flushed.load(Ordering::Acquire)
                    || playback.position() != Duration::ZERO
                {
                    wrong.push(format!(
                        "rewind={rewind}: stopped={}, flushed={}, position={:?}",
                        stopped.load(Ordering::Acquire),
                        flushed.load(Ordering::Acquire),
                        playback.position()
                    ));
                }
            } else {
                assert!(!refusal);
                assert!(flushed.load(Ordering::Acquire));
                assert_eq!(
                    pcm.lock().unwrap().as_slice(),
                    if rewind {
                        expected.as_slice()
                    } else {
                        &expected[640..]
                    }
                );
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "failed decoder reset continued device transition: {wrong:?}"
    );
}
