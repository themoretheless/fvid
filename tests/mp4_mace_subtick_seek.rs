#![cfg(feature="player")]
#[path="support/audio_transitions.rs"] mod transitions;
use fvid::{audio::AudioStream,playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,time::Duration,sync::{Arc,Mutex}};
use transitions::{Frames,collect};
const NAMES:&[&str]=&["control-0","control-1","control-2","control-6mono","control-6mono-grouped","changes","start-third","fragmented","fragmented-third","hybrid"];
fn bytes(n:&str)->Vec<u8>{std::fs::read(format!("{}/tests/fixtures/playback-errors/mp4-mace-subtick-seek/{n}.mp4",env!("CARGO_MANIFEST_DIR"))).unwrap()}
fn open(n:&str)->Mp4AudioReader<Cursor<Vec<u8>>>{Mp4AudioReader::open(Cursor::new(bytes(n)),Default::default()).unwrap()}
// The oracle is natural playback, never a second seek. Integer arithmetic
// independently selects the first native sample at or after the requested time.
fn suffix(full:&Frames,t:Duration)->Frames{full.iter().filter_map(|p|{
 let count=p.3.len()/(usize::from(p.1)*4);
 let skip=(t.as_nanos().saturating_sub(p.2.as_nanos())*u128::from(p.0)).div_ceil(1_000_000_000).min(count as u128)as usize;
 (skip<count).then(||(p.0,p.1,p.2+Duration::from_nanos((skip as u64*1_000_000_000)/u64::from(p.0)),p.3[skip*usize::from(p.1)*4..].to_vec()))
 }).collect()}
fn queries(end:Duration)->Vec<Duration>{[Duration::ZERO,Duration::from_nanos(1),Duration::from_nanos(125001),Duration::from_nanos(14_999_999),Duration::from_nanos(15_000_001),Duration::from_nanos(29_999_999),Duration::from_nanos(30_000_001),Duration::from_nanos(44_999_999),Duration::from_nanos(45_000_001),end.saturating_sub(Duration::from_nanos(1)),end,end+Duration::from_nanos(1),Duration::MAX].to_vec()}
fn anchor(wanted:&Frames,end:Duration)->Duration{wanted.first().map_or(end,|p|p.2)}
fn exact(got:&Frames,wanted:&Frames){
 // Packet grouping may change after preroll; compare every native point and bit.
 fn points(f:&Frames)->Vec<(u32,u16,Duration,Vec<u8>)>{let mut out=Vec::new();for p in f{for(i,b)in p.3.chunks_exact(usize::from(p.1)*4).enumerate(){out.push((p.0,p.1,p.2+Duration::from_nanos(i as u64*1_000_000_000/u64::from(p.0)),b.to_vec()));}}out}
 assert_eq!(points(got),points(wanted));
}
#[test]fn natural_pcm_and_native_video_controls(){for n in NAMES{let mut r=open(n);let full=collect(&mut r);let count:usize=full.iter().map(|p|p.3.len()/(usize::from(p.1)*4)).sum();assert_eq!(count,if n.starts_with("control-"){240}else{960},"{n}");assert!(full.iter().any(|p|p.3.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap())!=0.0)));let mut v=fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)),32<<20).unwrap();for _ in 0..6{assert_eq!(v.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}assert!(v.read_frame_raw().unwrap().is_none());}}
#[test]fn mace_native_duration_seek_preserves_history_and_floor(){let mut failures=Vec::new();for n in NAMES{let mut r=open(n);let end=r.duration().unwrap();let full=collect(&mut r);for t in queries(end){let wanted=suffix(&full,t);let at=r.seek_time(t);let got=collect(&mut r);let actual:usize=got.iter().map(|p|p.3.len()).sum();let expected:usize=wanted.iter().map(|p|p.3.len()).sum();if actual!=expected{failures.push(format!("{n}:{t:?}:{actual}bytes expected{expected}"));continue;}exact(&got,&wanted);if r.time_of(at)!=anchor(&wanted,end){failures.push(format!("{n}:{t:?}:anchor{:?} expected{:?}",r.time_of(at),anchor(&wanted,end)));}}}assert!(failures.is_empty(),"{failures:?}");}
#[test]fn raw_seek_restores_the_original_history_and_clock(){for n in NAMES{let mut r=open(n);let full=collect(&mut r);r.seek_time(Duration::from_nanos(125001));let _=collect(&mut r);let at=r.seek_to(0);assert_eq!(at,0);assert_eq!(r.time_of(at),Duration::ZERO);exact(&collect(&mut r),&full);}}

struct FlushCapture {
    inner: transitions::Capture,
    anchors: std::sync::Arc<std::sync::Mutex<Vec<Duration>>>,
}
impl fvid::audio::AudioBackend for FlushCapture {
    fn start(&mut self, s: fvid::audio::AudioSpec) -> Result<(), fvid::audio::AudioError> {
        self.inner.start(s)
    }
    fn push(&mut self, p: fvid::audio::AudioPacket) -> Result<(), fvid::audio::AudioError> {
        self.inner.push(p)
    }
    fn position(&self) -> Duration {
        self.inner.position()
    }
    fn flush(&mut self, at: Duration) -> Result<(), fvid::audio::AudioError> {
        self.anchors.lock().unwrap().push(at);
        self.inner.flush(at)
    }
    fn begin_drain(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.begin_drain()
    }
    fn drain_status(&self) -> Result<bool, fvid::audio::AudioError> {
        self.inner.drain_status()
    }
    fn end_drain(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.end_drain()
    }
    fn pause(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.pause()
    }
    fn resume(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.resume()
    }
    fn stop(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.stop()
    }
    fn reconfigure(
        &mut self,
        s: fvid::audio::AudioSpec,
        r: Option<fvid_media::owned_stereo_router::StereoRouter>,
        at: Duration,
        p: bool,
    ) -> Result<(), fvid::audio::AudioError> {
        self.inner.reconfigure(s, r, at, p)
    }
}


#[test]fn actual_worker_mace_native_seek_preserves_history_and_flush(){
 use fvid::audio_thread::AudioPlayback;use transitions::{Capture,Trace,ended};let mut failures=Vec::new();
 for n in NAMES{let mut r=open(n);let end=r.duration().unwrap();let full=collect(&mut r);let trace=Arc::new(Mutex::new(Trace::default()));let shared=trace.clone();let anchors=Arc::new(Mutex::new(Vec::new()));let times=anchors.clone();let mut w=AudioPlayback::start(Box::new(open(n)),move||Box::new(FlushCapture{inner:Capture::new(shared),anchors:times}));w.play();ended(&w,0);exact(&trace.lock().unwrap().packets,&full);
 for t in queries(end){let wanted=suffix(&full,t);let expected=anchor(&wanted,end);anchors.lock().unwrap().clear();w.seek(t);ended(&w,w.generation());let got=anchors.lock().unwrap().clone();if got!=vec![expected]{failures.push(format!("{n}:{t:?}:flush{got:?} expected{expected:?}"));}let guard=trace.lock().unwrap();let actual:usize=guard.packets.iter().map(|p|p.3.len()).sum();let size:usize=wanted.iter().map(|p|p.3.len()).sum();if actual!=size{failures.push(format!("{n}:{t:?}:{actual}bytes expected{size}"));continue;}exact(&guard.packets,&wanted);if wanted.is_empty(){assert_eq!(w.position(),end);}}
 }assert!(failures.is_empty(),"{failures:?}");}
