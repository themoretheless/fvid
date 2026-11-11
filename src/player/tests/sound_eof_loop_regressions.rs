//! A healthy native audio EOF must remain owned until A-B/stop handling.
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
        .join("tests/fixtures/playback-errors/player-sound-eof-loop")
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
    drained: usize,
    failed_drains: usize,
    flushes: Vec<Duration>,
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
        self.trace.lock().unwrap().flushes.push(t);
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
            self.trace.lock().unwrap().failed_drains += 1;
            Err(crate::audio::AudioError::PlaybackError(
                "synthetic output drain failure".into(),
            ))
        } else {
            let done=self.position() >= self.frontier;
            if done { self.trace.lock().unwrap().drained += 1; }
            Ok(done)
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

fn until(mut ready:impl FnMut()->bool) {
    let end=Instant::now()+Duration::from_secs(5);
    while !ready(){assert!(Instant::now()<end,"native audio acknowledgment timed out");std::thread::sleep(Duration::from_millis(1));}
}
// Worker::run sends the decode/drain event before processing another command.
// A backend Pause followed by Play acknowledgment therefore fences queued EOF
// without draining/reinjecting events or introducing a test-only player hook.
fn queued_eof(p:&Player,clock:&AtomicU64,trace:&Mutex<Trace>) {
    clock.store(200_000_000,Ordering::SeqCst);
    until(||trace.lock().unwrap().drained>0);
    let a=p.audio.as_ref().unwrap();let old=trace.lock().unwrap().pauses;
    a.pause();until(||trace.lock().unwrap().pauses>old);
    let old=trace.lock().unwrap().resumes;
    a.play();until(||trace.lock().unwrap().resumes>old);
    assert_eq!(a.position(),Duration::from_millis(200));
    assert_eq!(a.generation(),0);
}
fn finish(p:&mut Player,ctx:&egui::Context,clock:&AtomicU64) {
    p.loop_a=None;p.loop_b=None;clock.store(200_000_000,Ordering::SeqCst);
    until(||{tick(p,ctx);p.ended});
    assert!(p.audio_ended);assert!(p.error.is_none());
}
#[test]
fn exact_authored_video_and_pcm_seek_native_eof_control() {
    let mut r=crate::playback_mp4::Mp4VideoReader::open_software(Cursor::new(bytes("movie.mp4")),Default::default(),32<<20).unwrap();
    for i in 0..5 {
        let f=r.read_frame().unwrap().unwrap();
        assert_eq!((f.sample_index,f.presentation_time.ticks,f.duration.ticks),(i,i as i64*300,300));
        let yuv:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|v|v.to_le_bytes()).collect();
        assert_eq!(yuv,bytes(&format!("frame-{}.yuv16",i%3)));
    }
    assert!(r.read_frame().unwrap().is_none());
    let(mut p,ctx,clock,trace)=open();
    assert_eq!(trace.lock().unwrap().pcm,bytes("sound-short.f32le"));
    p.loop_a=None;p.loop_b=None;p.seek_time(Duration::from_millis(125));
    until(||trace.lock().unwrap().flushes==[Duration::from_millis(125)]);
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(125));
    finish(&mut p,&ctx,&clock);
    assert_eq!(p.audio.as_ref().unwrap().generation(),1);
}
fn repeat_eof(b:u64) {
    let(mut p,ctx,clock,trace)=open();p.loop_b=Some(Duration::from_millis(b));
    queued_eof(&p,&clock,&trace);tick(&mut p,&ctx);
    assert!(!p.ended,"healthy queued EOF closed the A-B item");
    assert!(!p.audio_ended);assert!(p.error.is_none());
    assert_eq!(p.audio.as_ref().unwrap().generation(),1,"exactly one A seek must follow EOF");
    until(||trace.lock().unwrap().flushes==[Duration::from_millis(25)]);
    let expected=bytes("sound-short.f32le");
    until(||trace.lock().unwrap().pcm.len()==expected.len()+expected.len()-800);
    let actual=trace.lock().unwrap().pcm.clone();
    assert_eq!(&actual[..expected.len()],expected);
    assert_eq!(&actual[expected.len()..],&expected[800..],"real seek must replay exact PCM from A");
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(25));
    finish(&mut p,&ctx,&clock);
}
#[test]
fn native_eof_at_b_replays_exact_audio_from_a(){repeat_eof(200);}
#[test]
fn delayed_eof_past_b_replays_exact_audio_from_a(){repeat_eof(100);}
#[test]
fn eof_keeps_earlier_explicit_stop_ahead_of_loop_rewind() {
    let(mut p,ctx,clock,trace)=open();p.stop_time=Some(Duration::from_millis(75));
    queued_eof(&p,&clock,&trace);tick(&mut p,&ctx);
    assert!(p.ended);assert_eq!(p.stop_time,None);
    assert_eq!(p.terminal_clock,Some(Duration::from_millis(75)));
    assert_eq!(p.audio.as_ref().unwrap().generation(),0);
    assert_eq!(p.presentation_clock(),Some(Duration::from_millis(75)));
    until(||trace.lock().unwrap().pauses==2);
}
#[test]
fn paused_native_eof_defers_loop_until_actual_resume() {
    let(mut p,ctx,clock,trace)=open();p.loop_b=Some(Duration::from_millis(200));
    queued_eof(&p,&clock,&trace);p.toggle_pause();tick(&mut p,&ctx);
    assert!(p.paused);assert!(!p.ended,"paused EOF must retain the pending loop");
    assert!(p.audio_ended);assert_eq!(p.audio.as_ref().unwrap().generation(),0);
    p.toggle_pause();tick(&mut p,&ctx);
    assert!(!p.paused && !p.ended && !p.audio_ended);
    assert_eq!(p.audio.as_ref().unwrap().generation(),1);
    until(||trace.lock().unwrap().flushes==[Duration::from_millis(25)]);
    clock_at(p.audio.as_ref().unwrap(),&clock,Duration::from_millis(25));
    finish(&mut p,&ctx,&clock);
}
#[test]
fn loop_b_beyond_native_end_does_not_fabricate_a_repeat() {
    let(mut p,ctx,clock,trace)=open();p.loop_b=Some(Duration::from_millis(300));
    queued_eof(&p,&clock,&trace);tick(&mut p,&ctx);
    assert!(p.ended && p.audio_ended);assert!(p.error.is_none());
    assert_eq!(p.audio.as_ref().unwrap().generation(),0);
    assert!(trace.lock().unwrap().flushes.is_empty());
}
#[test]
fn failed_output_drain_is_refused_without_replaying_the_worker() {
    let clock=Arc::new(AtomicU64::new(0));let trace=Arc::new(Mutex::new(Trace::default()));
    let fail=Arc::new(AtomicBool::new(false));let a=sound(clock.clone(),trace.clone(),fail.clone(),true);
    a.play();until(||trace.lock().unwrap().pcm.len()==bytes("sound-short.f32le").len());
    fail.store(true,Ordering::SeqCst);until(||trace.lock().unwrap().failed_drains>0);
    let mut p=Player{queue:vec![root().join("sound-short.wav")],audio:Some(a),paused:false,
        loop_a:Some(Duration::from_millis(25)),loop_b:Some(Duration::from_millis(100)),..Default::default()};
    let ctx=egui::Context::default();until(||{tick(&mut p,&ctx);p.audio.is_none()});
    assert!(p.ended && p.audio_ended);assert!(trace.lock().unwrap().flushes.is_empty());
    assert_eq!(trace.lock().unwrap().pcm,bytes("sound-short.f32le"));
}

#[test]
fn native_eof_preserves_a_reached_stop_without_any_loop() {
    let(mut p,ctx,clock,trace)=open();p.loop_a=None;p.loop_b=None;
    p.stop_time=Some(Duration::from_millis(75));
    queued_eof(&p,&clock,&trace);tick(&mut p,&ctx);
    assert!(p.ended);assert_eq!(p.stop_time,None);
    assert_eq!(p.terminal_clock,Some(Duration::from_millis(75)));
    assert_eq!(p.presentation_clock(),Some(Duration::from_millis(75)));
    assert_eq!(p.audio.as_ref().unwrap().generation(),0);
    until(||trace.lock().unwrap().pauses==2);
}
