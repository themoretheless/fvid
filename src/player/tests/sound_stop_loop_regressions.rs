//! Explicit sound-only stop must precede an A-B rewind at a delayed callback.
use super::*;
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-sound-stop-loop")
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
    fail: Arc<AtomicBool>,
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
        if self.fail.load(Ordering::SeqCst) {
            Err(crate::audio::AudioError::PlaybackError(
                "synthetic output drain failure".into(),
            ))
        } else {
            Ok(self.position() >= self.frontier)
        }
    }
}
fn sound(
    clock: Arc<AtomicU64>,
    trace: Arc<Mutex<Trace>>,
    fail: Arc<AtomicBool>,
    short: bool,
) -> crate::audio_thread::AudioPlayback {
    assert!(short);
    let source = crate::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("sound-short.wav")), Default::default()).unwrap();
    crate::audio_thread::AudioPlayback::start(Box::new(source), move || {
        Box::new(Device {
            clock,
            trace,
            fail,
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


fn open() -> (Player, egui::Context, Arc<AtomicU64>, Arc<Mutex<Trace>>) {
    let clock = Arc::new(AtomicU64::new(0));
    let trace = Arc::new(Mutex::new(Trace::default()));
    let a = sound(clock.clone(), trace.clone(), Arc::new(AtomicBool::new(false)), true);
    a.play();
    let until = Instant::now() + Duration::from_secs(5);
    while trace.lock().unwrap().pcm.len() != bytes("sound-short.f32le").len() {
        assert!(Instant::now() < until, "actual PCM did not reach device");
        std::thread::sleep(Duration::from_millis(1));
    }
    clock_at(&a, &clock, Duration::ZERO);
    let p = Player {
        queue: vec![root().join("sound-short.wav")], audio: Some(a),
        duration: Some(Duration::from_millis(200)), paused: false,
        loop_a: Some(Duration::from_millis(25)), loop_b: Some(Duration::from_millis(100)),
        ..Default::default()
    };
    (p, egui::Context::default(), clock, trace)
}
#[test]
fn exact_authored_video_and_actual_pcm_seek_eof_controls() {
    let mut r=crate::playback_mp4::Mp4VideoReader::open_software(Cursor::new(bytes("movie.mp4")),Default::default(),32<<20).unwrap();
    for i in 0..5 {
        let f=r.read_frame().unwrap().unwrap();
        assert_eq!((f.sample_index,f.presentation_time.ticks,f.duration.ticks),(i,i as i64*300,300));
        let yuv:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|v|v.to_le_bytes()).collect();
        assert_eq!(yuv,bytes(&format!("frame-{}.yuv16",i%3)));
    }
    assert!(r.read_frame().unwrap().is_none());
    let (mut p,ctx,clock,trace)=open();
    assert_eq!(trace.lock().unwrap().pcm,bytes("sound-short.f32le"));
    p.loop_a=None;p.loop_b=None;
    p.seek_time(Duration::from_millis(125));
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(125));
    assert_eq!(p.audio.as_ref().unwrap().generation(),1);
    clock.store(200_000_000,Ordering::SeqCst);
    let until=Instant::now()+Duration::from_secs(5);
    while !p.ended {assert!(Instant::now()<until);tick(&mut p,&ctx);std::thread::sleep(Duration::from_millis(1));}
    assert!(p.audio_ended);assert!(p.error.is_none());
    assert_eq!(p.audio.as_ref().unwrap().position(),Duration::from_millis(200));
}
fn stop_precedes_loop(stop:u64) {
    let (mut p,ctx,clock,trace)=open();
    p.stop_time=Some(Duration::from_millis(stop));
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(150));
    tick(&mut p,&ctx);
    assert!(p.ended,"A-B rewind concealed an already reached stop");
    assert_eq!(p.stop_time,None);
    assert_eq!(p.terminal_clock,Some(Duration::from_millis(stop)));
    assert_eq!(p.presentation_clock(),Some(Duration::from_millis(stop)));
    assert_eq!(p.audio.as_ref().unwrap().generation(),0,"stop must not seek to A");
    let until=Instant::now()+Duration::from_secs(5);
    while trace.lock().unwrap().pauses==0 {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
    assert_eq!(trace.lock().unwrap().pauses,1);
    for _ in 0..3 {tick(&mut p,&ctx);assert!(p.ended);}
    assert_eq!(p.audio.as_ref().unwrap().generation(),0);
    p.loop_a=None;p.loop_b=None;
    p.seek_time(Duration::from_millis(125));
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(125));
    assert!(!p.ended);assert_eq!(p.terminal_clock,None);
    clock.store(200_000_000,Ordering::SeqCst);
    let until=Instant::now()+Duration::from_secs(5);
    while !p.ended {assert!(Instant::now()<until);tick(&mut p,&ctx);std::thread::sleep(Duration::from_millis(1));}
    assert!(p.audio_ended);assert!(p.error.is_none());
}
#[test]
fn earlier_stop_survives_delayed_callback_past_loop_b(){stop_precedes_loop(75);}
#[test]
fn coincident_stop_survives_delayed_callback_past_loop_b(){stop_precedes_loop(100);}
#[test]
fn unreached_stop_still_allows_actual_loop_seek() {
    let(mut p,ctx,clock,_trace)=open();p.stop_time=Some(Duration::from_millis(175));
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(150));tick(&mut p,&ctx);
    assert!(!p.ended);assert_eq!(p.stop_time,Some(Duration::from_millis(175)));
    assert_eq!(p.audio.as_ref().unwrap().generation(),1);
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(25));
    assert_eq!(p.presentation_clock(),Some(Duration::from_millis(25)));
}
#[test]
fn paused_transport_does_not_spend_stop_or_loop() {
    let(mut p,ctx,clock,_trace)=open();p.stop_time=Some(Duration::from_millis(75));
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(150));p.toggle_pause();tick(&mut p,&ctx);
    assert!(p.paused);assert!(!p.ended);assert_eq!(p.stop_time,Some(Duration::from_millis(75)));
    assert_eq!(p.audio.as_ref().unwrap().generation(),0);
}
