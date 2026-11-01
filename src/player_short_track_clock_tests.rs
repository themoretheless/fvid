//! Replacing an audio clock with an already-ended source must not rewind video.
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
        .join("tests/fixtures/playback-errors/player-short-track-clock")
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
        self.frontier = t; // Flush discards the previous queued output.
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

fn until(mut ready:impl FnMut()->bool){let end=Instant::now()+Duration::from_secs(5);while !ready(){assert!(Instant::now()<end,"native track clock timed out");std::thread::sleep(Duration::from_millis(1));}}
fn factory(clock:Arc<AtomicU64>,trace:Arc<Mutex<Trace>>)->impl FnOnce()->Box<dyn crate::audio::AudioBackend>+Send+'static{move||Box::new(Device{clock,trace,fail:Arc::new(AtomicBool::new(false)),running:false,anchor:Duration::ZERO,frontier:Duration::ZERO})}

fn open(ctx:&egui::Context,phase:u64)->(Player,Arc<AtomicU64>,Arc<Mutex<Trace>>){
    let mut p=Player{queue:vec![root().join("movie.mp4")],no_audio:true,asynchronous_open:false,..Default::default()};p.play_index(0);until(||{tick(&mut p,ctx);p.presented.is_some()&&!p.buffering&&!p.seek_preview});
    let clock=Arc::new(AtomicU64::new(phase*1_000_000));let trace=Arc::new(Mutex::new(Trace::default()));assert!(p.try_start_audio_choice_with_backend(&root().join("movie.mp4"),Some(0),factory(clock.clone(),trace.clone())));p.audio.as_ref().unwrap().play();until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(phase));
    p.seek_time(Duration::from_millis(phase));until(||{tick(&mut p,ctx);!p.seek_preview&&!p.buffering&&p.picture_audio_clock&&p.audio.as_ref().unwrap().position()==Duration::from_millis(phase)});until(||trace.lock().unwrap().resumes>=2);assert_eq!(p.presentation_clock(),Some(Duration::from_millis(phase)));assert!(!p.audio_ended);(p,clock,trace)
}
fn replace(p:&mut Player)->(Arc<AtomicU64>,Arc<Mutex<Trace>>){let clock=Arc::new(AtomicU64::new(0));let trace=Arc::new(Mutex::new(Trace::default()));p.cycle_audio_track_with_backend(true,factory(clock.clone(),trace.clone()));assert_eq!(p.audio_track,1);(clock,trace)}
fn landed_at_short_end(p:&Player){until(||p.audio.as_ref().unwrap().position()==Duration::from_nanos(599_875_000));}
fn monotonic_eof(p:&mut Player,ctx:&egui::Context,before:Duration,started:Instant){let mut last=before;until(||{tick(p,ctx);let now=p.presentation_clock().unwrap();assert!(now>=last,"replacement EOF rewound video:{last:?}->{now:?}");assert!(now<=before+started.elapsed()+Duration::from_millis(10),"replacement EOF jumped past elapsed media phase:{now:?}");last=now;p.audio_ended});}
fn video_end(p:&mut Player,ctx:&egui::Context){until(||{tick(p,ctx);p.ended});assert!(p.error.is_none());assert_eq!(p.interval,Some((1200,1500,1000)));assert_eq!(p.presentation_clock(),Some(Duration::from_millis(1500)));}
fn source_prefix(trace:&Mutex<Trace>,at:u64){until(||!trace.lock().unwrap().pcm.is_empty());let actual=trace.lock().unwrap().pcm.clone();let expected=bytes("track-1.f32le");let start=at as usize*8*4;assert_eq!(actual,&expected[start..start+actual.len()]);}
#[test]
fn exact_mp4_unequal_tracks_video_pcm_seek_and_eof_controls(){
    let mut r=crate::playback_mp4::Mp4VideoReader::open_software(Cursor::new(bytes("movie.mp4")),Default::default(),32<<20).unwrap();for i in 0..5{let f=r.read_frame().unwrap().unwrap();assert_eq!((f.sample_index,f.presentation_time.ticks,f.duration.ticks),(i,i as i64*300,300));let yuv:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|v|v.to_le_bytes()).collect();assert_eq!(yuv,bytes(&format!("frame-{}.yuv16",i%3)));}assert!(r.read_frame().unwrap().is_none());
    for(nth,end)in[(0,1500),(1,600)]{let source=crate::playback_mp4_audio::Mp4AudioReader::open_at(Cursor::new(bytes("movie.mp4")),Default::default(),nth).unwrap();let trace=Arc::new(Mutex::new(Trace::default()));let a=crate::audio_thread::AudioPlayback::start(Box::new(source),factory(Arc::new(AtomicU64::new(end*1_000_000)),trace.clone()));a.play();until(||matches!(a.poll(),Some(crate::audio_thread::AudioEvent::Ended(_))));assert_eq!(trace.lock().unwrap().pcm,bytes(&format!("track-{nth}.f32le")));assert_eq!(a.duration(),Some(Duration::from_millis(end)));}
}
#[test]
fn already_ended_replacement_never_rewinds_live_video(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,750);assert_eq!(p.interval,Some((600,900,1000)));let before=p.presentation_clock().unwrap();let generation=p.playback.as_ref().unwrap().generation();let image=display_rgb(p.presented.as_ref().unwrap()).unwrap();let started=Instant::now();let(_clock,trace)=replace(&mut p);landed_at_short_end(&p);assert!(p.presentation_clock().unwrap()>=before,"landed short-track clock stole the current video phase");monotonic_eof(&mut p,&ctx,before,started);assert_eq!(p.playback.as_ref().unwrap().generation(),generation);assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(),image);assert!(trace.lock().unwrap().pcm.is_empty(),"past-end seek replayed tail PCM");video_end(&mut p,&ctx);
}
#[test]
fn paused_ended_replacement_keeps_phase_and_remaining_video_hold(){
    let ctx=egui::Context::default();let(mut p,_old,old)=open(&ctx,750);p.toggle_pause();until(||old.lock().unwrap().pauses>=2);let(_clock,trace)=replace(&mut p);landed_at_short_end(&p);assert_eq!(p.presentation_clock(),Some(Duration::from_millis(750)));assert!(p.paused);assert_eq!(trace.lock().unwrap().resumes,0);p.toggle_pause();let started=Instant::now();monotonic_eof(&mut p,&ctx,Duration::from_millis(750),started);assert!(trace.lock().unwrap().pcm.is_empty());video_end(&mut p,&ctx);
}
#[test]
fn replacement_eof_keeps_stop_bound_and_allows_backward_recovery(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,750);p.stop_time=Some(Duration::from_millis(775));let started=Instant::now();let(clock,trace)=replace(&mut p);landed_at_short_end(&p);monotonic_eof(&mut p,&ctx,Duration::from_millis(750),started);until(||{tick(&mut p,&ctx);p.ended});assert_eq!(p.terminal_clock,Some(Duration::from_millis(775)));assert_eq!(p.interval,Some((600,900,1000)));assert_eq!(p.presentation_clock(),Some(Duration::from_millis(775)));
    p.seek_time(Duration::from_millis(450));until(||{tick(&mut p,&ctx);!p.seek_preview&&!p.buffering&&p.audio.as_ref().unwrap().position()==Duration::from_millis(450)});assert_eq!(p.presentation_clock(),Some(Duration::from_millis(450)));assert_eq!(p.interval,Some((300,600,1000)));source_prefix(&trace,450);clock.store(600_000_000,Ordering::SeqCst);until(||{tick(&mut p,&ctx);p.audio_ended});video_end(&mut p,&ctx);
}
#[test]
fn explicit_backward_seek_releases_any_replacement_clock_floor_control(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,750);let(clock,trace)=replace(&mut p);landed_at_short_end(&p);p.seek_time(Duration::from_millis(450));until(||{tick(&mut p,&ctx);!p.seek_preview&&!p.buffering&&p.audio.as_ref().unwrap().position()==Duration::from_millis(450)});assert_eq!(p.presentation_clock(),Some(Duration::from_millis(450)));source_prefix(&trace,450);clock.store(600_000_000,Ordering::SeqCst);until(||{tick(&mut p,&ctx);p.audio_ended});video_end(&mut p,&ctx);
}
#[test]
fn unexpired_replacement_uses_real_audio_clock_and_exact_pcm_control(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,75);let(clock,trace)=replace(&mut p);until(||p.audio.as_ref().unwrap().position()==Duration::from_millis(75));tick(&mut p,&ctx);assert_eq!(p.presentation_clock(),Some(Duration::from_millis(75)));source_prefix(&trace,75);clock.store(150_000_000,Ordering::SeqCst);until(||{tick(&mut p,&ctx);p.presentation_clock()==Some(Duration::from_millis(150))});clock.store(600_000_000,Ordering::SeqCst);until(||{tick(&mut p,&ctx);p.audio_ended});video_end(&mut p,&ctx);
}
#[test]
fn sound_only_already_ended_replacement_finishes_normally_control(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,750);p.playback=None;p.presented=None;p.interval=None;let(_clock,trace)=replace(&mut p);until(||{tick(&mut p,&ctx);p.ended});assert!(p.audio_ended);assert!(p.error.is_none());assert_eq!(p.duration,Some(Duration::from_millis(600)));assert!(trace.lock().unwrap().pcm.is_empty());
}
#[test]
fn silent_paused_picture_keeps_its_phase_when_selected_audio_is_already_over(){
    let ctx=egui::Context::default();let mut p=Player{queue:vec![root().join("movie.mp4")],no_audio:true,asynchronous_open:false,..Default::default()};p.play_index(0);until(||{tick(&mut p,&ctx);p.presented.is_some()&&!p.buffering});p.toggle_pause();p.seek_time(Duration::from_millis(750));until(||{tick(&mut p,&ctx);!p.seek_preview&&!p.buffering});assert_eq!(p.presentation_clock(),Some(Duration::from_millis(750)));assert!(!p.picture_audio_clock);
    let source=crate::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes("movie.mp4")),Default::default()).unwrap();p.audio_tracks=crate::audio::AudioStream::audio_tracks(&source);p.audio_track=0;drop(source);let(_clock,trace)=replace(&mut p);landed_at_short_end(&p);assert_eq!(p.presentation_clock(),Some(Duration::from_millis(750)));p.toggle_pause();let started=Instant::now();monotonic_eof(&mut p,&ctx,Duration::from_millis(750),started);assert!(trace.lock().unwrap().pcm.is_empty());video_end(&mut p,&ctx);
}

#[test]
fn immediate_return_to_long_track_uses_the_preserved_source_phase(){
    let ctx=egui::Context::default();let(mut p,_old,_trace)=open(&ctx,750);let(_short_clock,short)=replace(&mut p);landed_at_short_end(&p);assert!(short.lock().unwrap().pcm.is_empty());
    let clock=Arc::new(AtomicU64::new(0));let trace=Arc::new(Mutex::new(Trace::default()));p.cycle_audio_track_with_backend(false,factory(clock.clone(),trace.clone()));assert_eq!(p.audio_track,0);until(||!trace.lock().unwrap().flushes.is_empty());let targets={trace.lock().unwrap().flushes.clone()};assert_eq!(targets,[Duration::from_millis(750)],"next track inherited the previous source EOF instead of live video phase");until(||!trace.lock().unwrap().pcm.is_empty());let actual=trace.lock().unwrap().pcm.clone();let expected=bytes("track-0.f32le");assert_eq!(actual,&expected[24000..24000+actual.len()]);clock.store(1_500_000_000,Ordering::SeqCst);video_end(&mut p,&ctx);
}
