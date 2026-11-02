#![cfg(feature="player")]
#[path="support/audio_transitions.rs"]mod transitions;
#[path="support/rational_audio_seek.rs"]mod rational;
use fvid::{audio::AudioStream,playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,time::Duration,sync::{Arc,Mutex}};
use rational::{Packet,collect,exact,suffix,anchor};
fn root()->std::path::PathBuf{std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-mixed-dolby-seek")}
fn cases()->Vec<serde_json::Value>{serde_json::from_slice::<serde_json::Value>(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()["cases"].as_array().unwrap().clone()}
fn bytes(n:&str)->Vec<u8>{std::fs::read(root().join(n)).unwrap()}
fn open(n:&str)->Mp4AudioReader<Cursor<Vec<u8>>>{
 let c=cases().into_iter().find(|c|c["file"].as_str()==Some(n));
 match c.and_then(|c|c["programme"].as_u64()) {
 Some(id)=>Mp4AudioReader::open_programme(Cursor::new(bytes(n)),Default::default(),0,id as u8).unwrap(),
 None=>Mp4AudioReader::open(Cursor::new(bytes(n)),Default::default()).unwrap()
 }
}
fn end(c:&serde_json::Value)->(u128,u128){(u128::from(c["end_num"].as_u64().unwrap()),u128::from(c["end_den"].as_u64().unwrap()))}
fn queries(c:&serde_json::Value)->Vec<Duration>{let mut v:Vec<_>=c["queries_ns"].as_array().unwrap().iter().map(|n|Duration::from_nanos(n.as_u64().unwrap())).collect();v.push(Duration::MAX);v}
#[test]fn rational_oracle_preserves_phase_hidden_by_floored_nanoseconds(){
 let p=Packet{rate:48000,channels:1,num:1,den:48000,data:vec![0;12]};assert_eq!(p.point(2),(1,16000));assert_eq!(rational::duration(p.point(2).0,p.point(2).1),Duration::from_nanos(62500));
 let mut other=p.clone();other.num=85001;other.den=4_080_000_000;assert_eq!(p.time(),other.time());assert_ne!(p.point(0),other.point(0));
 let s=suffix(&[p.clone()],Duration::from_nanos(41667),(4,48000));assert_eq!(s[0].point(0),(1,16000));assert!(suffix(&[p],Duration::MAX,(4,48000)).is_empty());
}
#[test]fn complete_native_pcm_and_short_video_controls(){for c in cases(){let n=c["file"].as_str().unwrap();let mut r=open(n);let e=end(&c);assert_eq!(r.duration(),Some(rational::duration(e.0,e.1)),"{n}:duration");let full=collect(&mut r);assert_eq!(full.iter().map(Packet::frames).sum::<usize>(),c["native_frames"].as_u64().unwrap()as usize,"{n}:frame count");assert!(full.iter().all(|p|matches!(p.rate,48000|32000|44100)&&matches!(p.channels,1|2|3|6)));assert!(full.iter().any(|p|p.data.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap())!=0.0)));for p in &full{assert!(p.num*e.1<e.0*p.den,"{n}:packet beyond declared end");}if c["opening_silence_frames"].as_u64().unwrap()>0{let p=&full[0];assert_eq!((p.rate,p.channels,p.num),(48000,2,0));assert!(p.data.len()>=48*8 && p.data[..48*8].iter().all(|b|*b==0));}let mut v=fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)),32<<20).unwrap();for _ in 0..6{assert_eq!(v.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}assert!(v.read_frame_raw().unwrap().is_none());}}
#[test]fn precise_mixed_coded_seek_matches_exact_rational_native_suffixes(){let mut failures=Vec::new();for c in cases(){let n=c["file"].as_str().unwrap();let mut r=open(n);let full=collect(&mut r);for t in queries(&c){let wanted=suffix(&full,t,end(&c));let at=r.seek_time(t);let got=collect(&mut r);let actual:usize=got.iter().map(|p|p.data.len()).sum();let expected:usize=wanted.iter().map(|p|p.data.len()).sum();if actual!=expected{failures.push(format!("{n}:{t:?}:{actual}bytes expected{expected}"));continue;}exact(&got,&wanted,&format!("{n}:{t:?}"));if r.time_of(at)!=anchor(&wanted,end(&c)){failures.push(format!("{n}:{t:?}:anchor{:?} expected{:?}",r.time_of(at),anchor(&wanted,end(&c))));}}}assert!(failures.is_empty(),"{failures:?}");}
#[test]fn raw_tick_reseek_restores_exact_original_timebase_and_decoder_history(){for c in cases(){let n=c["file"].as_str().unwrap();let mut r=open(n);let full=collect(&mut r);r.seek_time(Duration::from_nanos(500001));let _=collect(&mut r);let at=r.seek_to(0);assert_eq!(at,0);assert_eq!(r.time_of(at),Duration::ZERO);exact(&collect(&mut r),&full,n);}}
#[test]fn actual_worker_mixed_coded_seeks_preserve_raw_timebases_and_device_anchors(){use fvid::audio_thread::AudioPlayback;use transitions::{Capture,Trace,ended};let mut failures=Vec::new();for c in cases(){let n=c["file"].as_str().unwrap();let mut r=open(n);let full=collect(&mut r);let trace=Arc::new(Mutex::new(Trace::default()));let shared=trace.clone();let raw=Arc::new(Mutex::new(rational::RawTrace::default()));let captured=raw.clone();let mut w=AudioPlayback::start(Box::new(open(n)),move||Box::new(rational::RawCapture{inner:Capture::new(shared),raw:captured}));w.play();ended(&w,0);exact(&raw.lock().unwrap().packets,&full,n);for t in queries(&c){let wanted=suffix(&full,t,end(&c));let expected=anchor(&wanted,end(&c));raw.lock().unwrap().anchors.clear();w.seek(t);ended(&w,w.generation());let guard=raw.lock().unwrap();if guard.anchors!=vec![expected]{failures.push(format!("{n}:{t:?}:flush{:?} expected{expected:?}",guard.anchors));}let actual:usize=guard.packets.iter().map(|p|p.data.len()).sum();let size:usize=wanted.iter().map(|p|p.data.len()).sum();if actual!=size{failures.push(format!("{n}:{t:?}:{actual}bytes expected{size}"));continue;}exact(&guard.packets,&wanted,&format!("{n}:{t:?}"));if wanted.is_empty(){assert_eq!(w.position(),expected);}}}assert!(failures.is_empty(),"{failures:?}");}
#[test]
fn independent_reset_controls_preserve_native_sample_clocks_and_pcm() {
 for c in cases().into_iter().filter(|c|!c["edited"].as_bool().unwrap()) {
  let n=c["file"].as_str().unwrap();let mut wanted=Vec::new();
  for g in c["groups"].as_array().unwrap() {
   let name=g["control"].as_str().unwrap();
   let mut r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
   let packets=collect(&mut r);let rate=g["rate"].as_u64().unwrap();let channels=g["channels"].as_u64().unwrap();let mut cursor=0usize;
   for p in &packets {assert_eq!((p.rate as u64,p.channels as u64),(rate,channels),"{name}");assert_eq!(p.num*u128::from(rate),cursor as u128*p.den,"{name}:native cursor");cursor+=p.frames();}
   assert_eq!(cursor as u64,g["frames"].as_u64().unwrap(),"{name}");
   for mut p in packets {p.num=p.num*14_112_000+u128::from(g["start"].as_u64().unwrap())*p.den;p.den*=14_112_000;wanted.push(p);}
  }
  exact(&collect(&mut open(n)),&wanted,n);
 }
}
#[test]
fn fresh_reader_seeks_probe_selected_programme_before_playback() {
 for c in cases(){let n=c["file"].as_str().unwrap();let full=collect(&mut open(n));for q in c["preplay_ns"].as_array().unwrap(){let target=Duration::from_nanos(q.as_u64().unwrap());let wanted=suffix(&full,target,end(&c));let mut r=open(n);let at=r.seek_time(target);exact(&collect(&mut r),&wanted,&format!("{n}:fresh:{target:?}"));assert_eq!(r.time_of(at),anchor(&wanted,end(&c)),"{n}:fresh anchor");}}
}
#[test]
fn fresh_worker_seeks_use_selected_programme_and_exact_device_anchor() {
 use fvid::audio_thread::AudioPlayback;use transitions::{Capture,Trace,ended};
 for c in cases(){let n=c["file"].as_str().unwrap();let full=collect(&mut open(n));for q in c["preplay_ns"].as_array().unwrap(){let t=Duration::from_nanos(q.as_u64().unwrap());let wanted=suffix(&full,t,end(&c));let raw=Arc::new(Mutex::new(rational::RawTrace::default()));let captured=raw.clone();let trace=Arc::new(Mutex::new(Trace::default()));let mut w=AudioPlayback::start(Box::new(open(n)),move||Box::new(rational::RawCapture{inner:Capture::new(trace),raw:captured}));w.seek(t);w.play();ended(&w,w.generation());let guard=raw.lock().unwrap();exact(&guard.packets,&wanted,&format!("{n}:fresh worker:{t:?}"));assert_eq!(guard.anchors,vec![anchor(&wanted,end(&c))],"{n}:fresh flush");}}
}
#[test]
fn missing_programme_and_packet_budget_are_refusal_controls() {
 let manifest:serde_json::Value=serde_json::from_slice(&bytes("manifest.json")).unwrap();let m=&manifest["refusal"];
 let error=Mp4AudioReader::open_programme(Cursor::new(bytes(m["file"].as_str().unwrap())),Default::default(),0,m["programme"].as_u64().unwrap()as u8).err().expect("missing programme must be rejected");assert!(error.to_string().contains("selected E-AC-3 programme is missing"),"{error}");
 let b=&manifest["budget"];let mut limits=fvid::container::mp4::Limits::default();limits.packet_bytes=b["bytes"].as_u64().unwrap()as usize;
 let mut r=Mp4AudioReader::open(Cursor::new(bytes(b["file"].as_str().unwrap())),limits).unwrap();r.seek_time(Duration::from_nanos(b["target_ns"].as_u64().unwrap()));let error=r.next_step().err().expect("seek must retain full packet admission");assert!(error.to_string().contains("packet"),"{error}");
}
#[test]
fn crc_bad_ac3_control_is_successful_concealment() {
 let mut r=Mp4AudioReader::open(Cursor::new(bytes("control-5.mp4")),Default::default()).unwrap();let p=collect(&mut r);let data:Vec<_>=p.iter().flat_map(|p|p.data.iter().copied()).collect();assert_eq!(data.len(),3072*6*4);assert!(data[..1536*6*4].iter().all(|b|*b==0));assert!(data[1536*6*4..].iter().any(|b|*b!=0));
}
