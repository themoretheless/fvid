#![cfg(feature="player")]
#[path="support/audio_transitions.rs"] mod transitions;
use fvid::{audio::AudioStream,playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,time::Duration,sync::{Arc,Mutex}};
use transitions::{Frames,collect};
const NAMES:&[&str]=&["ima4","ima-wav","ms-adpcm","ac3","ac3-grouped","ac3-one-packet","eac3-48000","eac3-32000"];
fn bytes(n:&str)->Vec<u8>{std::fs::read(format!("{}/tests/fixtures/playback-errors/mp4-constant-edit-subtick-seek/{n}.mp4",env!("CARGO_MANIFEST_DIR"))).unwrap()}
fn open(n:&str)->Mp4AudioReader<Cursor<Vec<u8>>>{Mp4AudioReader::open(Cursor::new(bytes(n)),Default::default()).unwrap()}
// The oracle is natural playback, never a second seek. Integer arithmetic
// independently selects the first native sample at or after the requested time.
fn suffix(full:&Frames,t:Duration)->Frames{full.iter().filter_map(|p|{
 let rate=u128::from(p.0);let count=p.3.len()/(usize::from(p.1)*4);
 // Duration is a floor of the exact native timestamp. Recover its integer
 // native index first; subtracting floored nanoseconds loses the 48k phase.
 let base=(p.2.as_nanos()*rate).div_ceil(1_000_000_000);
 let wanted=(t.as_nanos()*rate).div_ceil(1_000_000_000);
 let skip=wanted.saturating_sub(base).min(count as u128)as usize;
 (skip<count).then(||(p.0,p.1,Duration::from_nanos(((base+skip as u128)*1_000_000_000/rate)as u64),p.3[skip*usize::from(p.1)*4..].to_vec()))
 }).collect()}
fn queries(end:Duration)->Vec<Duration>{[Duration::ZERO,Duration::from_nanos(2_999_999),Duration::from_nanos(3_000_001),Duration::from_nanos(4_999_999),Duration::from_nanos(5_000_001),Duration::from_nanos(10_999_999),Duration::from_nanos(11_000_001),Duration::from_nanos(12_999_999),Duration::from_nanos(13_000_001),Duration::from_nanos(69_999_999),Duration::from_nanos(70_000_001),Duration::from_nanos(74_999_999),Duration::from_nanos(75_000_001),Duration::from_nanos(1),Duration::from_nanos(125001),Duration::from_nanos(14_999_999),Duration::from_nanos(15_000_001),Duration::from_nanos(29_999_999),Duration::from_nanos(30_000_001),Duration::from_nanos(44_999_999),Duration::from_nanos(45_000_001),end.saturating_sub(Duration::from_nanos(1)),end,end+Duration::from_nanos(1),Duration::MAX].to_vec()}
fn anchor(wanted:&Frames,end:Duration)->Duration{wanted.first().map_or(end,|p|p.2)}
fn exact(got:&Frames,wanted:&Frames){
 // Packet grouping may change after preroll; compare every native point and bit.
 fn points(f:&Frames)->Vec<(u32,u16,Duration,Vec<u8>)>{let mut out=Vec::new();for p in f{let rate=u128::from(p.0);let base=(p.2.as_nanos()*rate).div_ceil(1_000_000_000);for(i,b)in p.3.chunks_exact(usize::from(p.1)*4).enumerate(){out.push((p.0,p.1,Duration::from_nanos(((base+i as u128)*1_000_000_000/rate)as u64),b.to_vec()));}}out}
 assert_eq!(points(got),points(wanted));
}
#[test]
fn natural_pcm_and_native_video_controls(){
 for n in NAMES{
  let (rate,ch,video)=match *n {"ima4"|"ms-adpcm"=>(8000,1,6),"ima-wav"=>(16000,2,6),"eac3-48000"=>(48000,2,17),"eac3-32000"=>(32000,1,17),_=>(48000,6,5)};
  let mut r=open(n);assert_eq!(r.duration(),Some(Duration::from_millis(21)),"{n}");let full=collect(&mut r);
  assert!(full.iter().all(|p|p.0==rate && p.1==ch),"{n}:geometry");
  let pcm:Vec<u8>=full.iter().flat_map(|p|p.3.iter().copied()).collect();
  assert_eq!(pcm.len(),rate as usize*21/1000*usize::from(ch)*4,"{n}:sample count");
  assert!(pcm[..rate as usize*3/1000*usize::from(ch)*4].iter().all(|b|*b==0),"{n}:opening silence");
  assert!(pcm.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap())!=0.0),"{n}:audible source");
  let mut v=fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)),32<<20).unwrap();
  for _ in 0..video{assert_eq!(v.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}
  assert!(v.read_frame_raw().unwrap().is_none());
 }
}
#[test]fn constant_edit_native_duration_seek_preserves_history_and_floor(){let mut failures=Vec::new();for n in NAMES{let mut r=open(n);let end=r.duration().unwrap();let full=collect(&mut r);for t in queries(end){let wanted=suffix(&full,t);let at=r.seek_time(t);let got=collect(&mut r);let actual:usize=got.iter().map(|p|p.3.len()).sum();let expected:usize=wanted.iter().map(|p|p.3.len()).sum();if actual!=expected{failures.push(format!("{n}:{t:?}:{actual}bytes expected{expected}"));continue;}exact(&got,&wanted);if r.time_of(at)!=anchor(&wanted,end){failures.push(format!("{n}:{t:?}:anchor{:?} expected{:?}",r.time_of(at),anchor(&wanted,end)));}}}assert!(failures.is_empty(),"{failures:?}");}
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


#[test]fn actual_worker_constant_edit_native_seek_preserves_history_and_flush(){
 use fvid::audio_thread::AudioPlayback;use transitions::{Capture,Trace,ended};let mut failures=Vec::new();
 for n in NAMES{let mut r=open(n);let end=r.duration().unwrap();let full=collect(&mut r);let trace=Arc::new(Mutex::new(Trace::default()));let shared=trace.clone();let anchors=Arc::new(Mutex::new(Vec::new()));let times=anchors.clone();let mut w=AudioPlayback::start(Box::new(open(n)),move||Box::new(FlushCapture{inner:Capture::new(shared),anchors:times}));w.play();ended(&w,0);exact(&trace.lock().unwrap().packets,&full);
 for t in queries(end){let wanted=suffix(&full,t);let expected=anchor(&wanted,end);anchors.lock().unwrap().clear();w.seek(t);ended(&w,w.generation());let got=anchors.lock().unwrap().clone();if got!=vec![expected]{failures.push(format!("{n}:{t:?}:flush{got:?} expected{expected:?}"));}let guard=trace.lock().unwrap();let actual:usize=guard.packets.iter().map(|p|p.3.len()).sum();let size:usize=wanted.iter().map(|p|p.3.len()).sum();if actual!=size{failures.push(format!("{n}:{t:?}:{actual}bytes expected{size}"));continue;}exact(&guard.packets,&wanted);if wanted.is_empty(){assert_eq!(w.position(),end);}}
 }assert!(failures.is_empty(),"{failures:?}");}
