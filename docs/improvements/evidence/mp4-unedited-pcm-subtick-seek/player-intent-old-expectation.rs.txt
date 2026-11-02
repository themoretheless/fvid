//! Pending sound navigation must compose without changing the actual device clock.
use super::*;
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex, Condvar,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-audio-seek-intent")
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
    segments: Vec<(Duration, Vec<u8>)>,
    pauses: usize,
    resumes: usize,
}
struct Device {
    clock: Arc<AtomicU64>,
    trace: Arc<Mutex<Trace>>,
    fail: Arc<AtomicBool>,
    gate: Arc<(Mutex<Gate>, Condvar)>,
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
        let mut trace=self.trace.lock().unwrap();
        trace.pcm.extend(&p.data);
        if let Some((_,pcm))=trace.segments.last_mut(){pcm.extend(p.data);}
        Ok(())
    }
    fn flush(&mut self, t: Duration) -> Result<(), crate::audio::AudioError> {
        let (lock,ready)=&*self.gate;let mut g=lock.lock().unwrap();
        if g.armed {g.armed=false;g.entered=true;ready.notify_all();
            while !g.released {let (next,timeout)=ready.wait_timeout(g,Duration::from_secs(5)).unwrap();g=next;assert!(!timeout.timed_out(),"owned flush gate timed out");}}
        drop(g);self.trace.lock().unwrap().segments.push((t,Vec::new()));
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

#[derive(Default)]
struct Gate{armed:bool,entered:bool,released:bool}
struct Release(Arc<(Mutex<Gate>,Condvar)>);
impl Drop for Release{fn drop(&mut self){release(&self.0);}}
fn release(g:&(Mutex<Gate>,Condvar)){g.0.lock().unwrap().released=true;g.1.notify_all();}
fn until(mut ready:impl FnMut()->bool){let end=Instant::now()+Duration::from_secs(5);while !ready(){assert!(Instant::now()<end,"actual audio state timed out");std::thread::sleep(Duration::from_millis(1));}}
fn open(at:u64)->(Player,egui::Context,Arc<AtomicU64>,Arc<Mutex<Trace>>,Arc<(Mutex<Gate>,Condvar)>){
    let clock=Arc::new(AtomicU64::new(at*1_000_000));let trace=Arc::new(Mutex::new(Trace::default()));let gate=Arc::new((Mutex::new(Gate::default()),Condvar::new()));
    let source=crate::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes("sound.mp4")),Default::default()).unwrap();
    let(c,t,g)=(clock.clone(),trace.clone(),gate.clone());
    let audio=crate::audio_thread::AudioPlayback::start(Box::new(source),move||Box::new(Device{clock:c,trace:t,gate:g,fail:Arc::new(AtomicBool::new(false)),running:false,anchor:Duration::ZERO,frontier:Duration::ZERO}));
    audio.play();until(||trace.lock().unwrap().pcm.len()==bytes("sound.f32le").len());until(||audio.position()==Duration::from_millis(at));
    (Player{queue:vec![root().join("sound.mp4")],audio:Some(audio),duration:Some(Duration::from_millis(200)),seekable:true,paused:false,..Default::default()},egui::Context::default(),clock,trace,gate)
}
fn blocked(g:&(Mutex<Gate>,Condvar)){until(||g.0.lock().unwrap().entered);}
fn last_pcm(trace:&Mutex<Trace>,target:Duration){
    let oracle=bytes("sound.f32le");let start=(target.as_nanos()*8000/1_000_000_000)as usize*4;
    until(||trace.lock().unwrap().segments.last().is_some_and(|(at,pcm)|*at==target&&pcm.len()==oracle.len()-start));
    assert_eq!(trace.lock().unwrap().segments.last().unwrap().1,&oracle[start..]);
}
fn finish(p:&mut Player,ctx:&egui::Context,clock:&AtomicU64){clock.store(200_000_000,Ordering::SeqCst);until(||{tick(p,ctx);p.ended});assert!(p.audio_ended&&p.error.is_none());}
#[test]
fn exact_mp4_video_and_actual_pcm_seek_eof_control(){
    let mut r=crate::playback_mp4::Mp4VideoReader::open_software(Cursor::new(bytes("movie.mp4")),Default::default(),32<<20).unwrap();
    for i in 0..5{let f=r.read_frame().unwrap().unwrap();assert_eq!((f.sample_index,f.presentation_time.ticks,f.duration.ticks),(i,i as i64*300,300));let yuv:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|v|v.to_le_bytes()).collect();assert_eq!(yuv,bytes(&format!("frame-{}.yuv16",i%3)));}assert!(r.read_frame().unwrap().is_none());
    let(mut p,ctx,clock,trace,_gate)=open(25);assert_eq!(trace.lock().unwrap().pcm,bytes("sound.f32le"));p.seek_time(Duration::from_millis(125));last_pcm(&trace,Duration::from_millis(125));until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(125));finish(&mut p,&ctx,&clock);
}
#[test]
fn pending_absolute_seek_keeps_requested_transport_but_not_device_clock(){
    let(mut p,ctx,clock,trace,gate)=open(25);let _release=Release(gate.clone());gate.0.lock().unwrap().armed=true;
    p.seek_time(Duration::from_millis(75));blocked(&gate);
    assert_eq!(p.audio.as_ref().unwrap().position(),Duration::from_millis(25),"physical clock must remain actual");
    assert_eq!(p.presentation_clock(),Some(Duration::from_millis(75)),"pending seek intent was lost");
    assert_eq!(p.timeline().0,Some(Duration::from_millis(75)));
    release(&gate);last_pcm(&trace,Duration::from_millis(75));until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(75));
    clock.store(100_000_000,Ordering::SeqCst);until(||p.presentation_clock()==Some(Duration::from_millis(100)));finish(&mut p,&ctx,&clock);
}
fn compose(start:u64,forward:bool,paused:bool,expected:u64){
    let(mut p,ctx,clock,trace,gate)=open(start);let _release=Release(gate.clone());
    if paused{p.toggle_pause();until(||trace.lock().unwrap().pauses==1);}
    gate.0.lock().unwrap().armed=true;p.jump(Duration::from_millis(50),forward);blocked(&gate);p.jump(Duration::from_millis(25),forward);
    release(&gate);until(||trace.lock().unwrap().segments.len()==2);
    let targets:Vec<_>=trace.lock().unwrap().segments.iter().map(|(at,_)|*at).collect();assert_eq!(targets,[Duration::from_millis(75),Duration::from_millis(expected)],"successive jumps did not compose from pending intent");
    assert_eq!(p.paused,paused);if paused{p.toggle_pause();}
    last_pcm(&trace,Duration::from_millis(expected));until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(expected));finish(&mut p,&ctx,&clock);
}
#[test]fn queued_forward_jumps_preserve_both_offsets(){compose(25,true,false,100);}
#[test]fn queued_backward_jumps_preserve_both_offsets(){compose(125,false,false,50);}
#[test]fn paused_backward_jumps_preserve_both_offsets_and_resume_pcm(){compose(125,false,true,50);}
#[test]
fn restart_supersedes_a_blocked_seek_with_zero_transport_phase(){
    let(mut p,ctx,clock,trace,gate)=open(25);let _release=Release(gate.clone());gate.0.lock().unwrap().armed=true;p.seek_time(Duration::from_millis(75));blocked(&gate);p.restart();
    assert_eq!(p.presentation_clock(),Some(Duration::ZERO));assert_eq!(p.timeline().0,Some(Duration::ZERO));
    release(&gate);last_pcm(&trace,Duration::ZERO);until(||p.audio.as_ref().unwrap().position()==Duration::ZERO);finish(&mut p,&ctx,&clock);
}
#[test]
fn acknowledged_fractional_request_releases_to_quantized_actual_mp4_clock(){
    let(mut p,ctx,clock,trace,_gate)=open(25);p.seek_time(Duration::from_nanos(75_000_001));last_pcm(&trace,Duration::from_millis(75));until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(75));assert_eq!(p.presentation_clock(),Some(Duration::from_millis(75)));
    clock.store(100_000_000,Ordering::SeqCst);until(||p.presentation_clock()==Some(Duration::from_millis(100)));finish(&mut p,&ctx,&clock);
}
