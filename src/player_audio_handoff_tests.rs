//! A completed short audio source must hand its clock to continuing video.
use super::*;
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-audio-handoff")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn tick(p: &mut Player, ctx: &egui::Context) {
    let mut out = ctx.run_ui(Default::default(), |ui| p.present(ui.ctx()));
    out.textures_delta.clear();
}
#[derive(Default)]
struct Trace {
    pcm: Vec<u8>,
    pauses: usize,
    resumes: usize,
}
struct Device {
    clock: Arc<AtomicU64>,
    trace: Arc<Mutex<Trace>>,
    running: bool,
    anchor: Duration,
    frontier: Duration,
}
impl crate::audio::AudioBackend for Device {
    fn start(&mut self, s: crate::audio::AudioSpec) -> Result<(), crate::audio::AudioError> {
        assert_eq!((s.sample_rate, s.channels), (8000, 1));
        Ok(())
    }
    fn position(&self) -> Duration {
        if self.running {
            Duration::from_nanos(self.clock.load(Ordering::SeqCst))
        } else {
            self.anchor
        }
    }
    fn push(&mut self, p: crate::audio::AudioPacket) -> Result<(), crate::audio::AudioError> {
        self.frontier =
            p.presentation_time() + Duration::from_nanos(p.data.len() as u64 / 4 * 125000);
        self.trace.lock().unwrap().pcm.extend(p.data);
        Ok(())
    }
    fn flush(&mut self, t: Duration) -> Result<(), crate::audio::AudioError> {
        self.anchor = t;
        self.clock.store(t.as_nanos() as u64, Ordering::SeqCst);
        Ok(())
    }
    fn pause(&mut self) -> Result<(), crate::audio::AudioError> {
        self.anchor = self.position();
        self.running = false;
        self.trace.lock().unwrap().pauses += 1;
        Ok(())
    }
    fn resume(&mut self) -> Result<(), crate::audio::AudioError> {
        self.running = true;
        self.trace.lock().unwrap().resumes += 1;
        Ok(())
    }
    fn stop(&mut self) -> Result<(), crate::audio::AudioError> {
        self.running = false;
        Ok(())
    }
    fn drain_status(&self) -> Result<bool, crate::audio::AudioError> {
        Ok(self.position() >= self.frontier)
    }
}
fn sound(clock: Arc<AtomicU64>, trace: Arc<Mutex<Trace>>) -> crate::audio_thread::AudioPlayback {
    let source = crate::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("sound.wav")),
        Default::default(),
    )
    .unwrap();
    crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
        Box::new(Device {
            clock,
            trace,
            running: false,
            anchor: Duration::ZERO,
            frontier: Duration::ZERO,
        })
    })
}
fn clock_at(a: &crate::audio_thread::AudioPlayback, clock: &AtomicU64, t: Duration) {
    clock.store(t.as_nanos() as u64, Ordering::SeqCst);
    let until = Instant::now() + Duration::from_secs(5);
    while a.position() != t {
        if let Some(crate::audio_thread::AudioEvent::Error(e)) = a.poll() {
            panic!("{e}");
        }
        assert!(
            Instant::now() < until,
            "audio clock {:?} expected{t:?}",
            a.position()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn exact_video_and_short_audio_source_controls() {
    let mut r = crate::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("movie.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..5 {
        let f = r.read_frame().unwrap().unwrap();
        assert_eq!(
            (f.sample_index, f.presentation_time.ticks, f.duration.ticks),
            (i, i as i64 * 20, 20)
        );
        let yuv: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(yuv, bytes(&format!("frame-{}.yuv16", i % 3)));
    }
    assert!(r.read_frame().unwrap().is_none());
    let clock = Arc::new(AtomicU64::new(0));
    let trace = Arc::new(Mutex::new(Trace::default()));
    let a = sound(clock.clone(), trace.clone());
    a.play();
    let until = Instant::now() + Duration::from_secs(5);
    while trace.lock().unwrap().pcm.len() != bytes("sound.f32le").len() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(trace.lock().unwrap().pcm, bytes("sound.f32le"));
    clock.store(40_000_000, Ordering::SeqCst);
    loop {
        assert!(Instant::now() < until);
        match a.poll() {
            Some(crate::audio_thread::AudioEvent::Ended(_)) => break,
            Some(crate::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(a.position(), Duration::from_millis(40));
}
#[test]
fn short_audio_completion_does_not_return_video_to_the_unused_startup_deadline() {
    let ctx = egui::Context::default();
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        ..Default::default()
    };
    p.play_index(0);
    let clock = Arc::new(AtomicU64::new(0));
    let trace = Arc::new(Mutex::new(Trace::default()));
    p.audio = Some(sound(clock.clone(), trace.clone()));
    p.audio_ended = false;
    let until = Instant::now() + Duration::from_secs(5);
    while p.presented.is_none() {
        assert!(Instant::now() < until);
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((0, 20, 1000)));
    while trace.lock().unwrap().pcm.len() != bytes("sound.f32le").len() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    clock.store(40_000_000, Ordering::SeqCst);
    while !p.audio_ended {
        assert!(Instant::now() < until);
        p.poll_audio();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        p.audio.as_ref().unwrap().position(),
        Duration::from_millis(40)
    );
    assert_eq!(trace.lock().unwrap().pcm, bytes("sound.f32le"));
    let completed = Instant::now();
    while p.interval.unwrap().0 == 0 && completed.elapsed() < Duration::from_millis(70) {
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        p.interval.unwrap().0 >= 40,
        "audio completed40ms but video stayed at0ms against unused startup deadline {:?}",
        p.deadline.saturating_duration_since(Instant::now())
    );
    assert!(!p.ended);
    assert!(p.error.is_none());
    let tail_end = Instant::now() + Duration::from_secs(1);
    while !p.ended {
        assert!(Instant::now() < tail_end, "silent video tail stalled");
        tick(&mut p, &ctx);
        assert!(p.error.is_none(), "{:?}", p.error);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((80, 100, 1000)));
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let mut last = Vec::new();
    while let Some(raw) = reference.read_frame_raw().unwrap() {
        last = raw.into_rgb(32 << 20).unwrap();
    }
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), last);
    assert!(!p.buffering && !p.seek_preview);
}
