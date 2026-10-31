#![cfg(feature="player")]
use fvid::{audio::AudioStream,playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_transitions.rs"]mod transitions;
fn bytes(n:&str)->Vec<u8>{std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-audio-wide-time").join(n)).unwrap()}
fn cases()->Vec<serde_json::Value>{serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"].as_array().unwrap().clone()}
fn at(ticks:u64)->Duration{Duration::new(ticks/8000,((ticks%8000)*125000)as u32)}
#[test]
fn wide_audio_duration_and_time_of_preserve_representable_seconds(){
 let mut failures=Vec::new();
 for c in cases().into_iter().filter(|c|c["valid"]==true){
  let name=c["file"].as_str().unwrap();let r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
  let start=c["start"].as_u64().unwrap();let end=c["end"].as_u64().unwrap();
  if r.duration()!=Some(at(end)){failures.push(format!("{name}: duration {:?} expected {:?}",r.duration(),at(end)));}
  if r.time_of(start as i64)!=at(start){failures.push(format!("{name}: source time {:?} expected {:?}",r.time_of(start as i64),at(start)));}
 }
 assert!(failures.is_empty(),"narrowed wide audio time: {failures:?}");
}
#[test]
fn wide_audio_keeps_exact_pcm_and_sample_seek_suffix(){
 for c in cases().into_iter().filter(|c|c["valid"]==true){
  let name=c["file"].as_str().unwrap();let start=c["start"].as_u64().unwrap();let end=c["end"].as_u64().unwrap();
  let mut r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
  let frames=transitions::collect(&mut r);let got:Vec<_>=frames.iter().flat_map(|f|f.3.iter().copied()).collect();assert_eq!(got,bytes("expected.f32le"),"{name}");
  assert_eq!(frames.iter().map(|f|f.2).collect::<Vec<_>>(),[at(start),at(start+64),at(start+128)],"{name}");
  r.seek_time(at(start+1));let got:Vec<_>=transitions::collect(&mut r).into_iter().flat_map(|f|f.3).collect();assert_eq!(got,bytes("expected.f32le")[4..],"{name}: seek");
  r.seek_time(at(end));assert!(transitions::collect(&mut r).is_empty(),"{name}: EOF");
  r.rewind();let got:Vec<_>=transitions::collect(&mut r).into_iter().flat_map(|f|f.3).collect();assert_eq!(got,bytes("expected.f32le"),"{name}: rewind");
 }
}
#[test]
fn fragment_clocks_and_short_video_remain_valid_and_real_overflow_refuses(){
 for c in cases(){
  let name=c["file"].as_str().unwrap();
  let parsed=fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(bytes(name)),Default::default());
  if c["valid"]==false{let e=parsed.err().expect("signed overflow admitted");assert!(e.to_string().contains("overflow"),"{e}");continue;}
  let r=parsed.unwrap();let track=r.tracks().iter().find(|t|t.handler==*b"soun").unwrap();assert_eq!(track.samples.len(),3);
  for i in 0..3{let s=track.samples.get(i).unwrap();assert_eq!((s.dts,s.pts,s.duration),(c["start"].as_u64().unwrap()+i as u64*64,(c["start"].as_u64().unwrap()+i as u64*64)as i64,64));}
  let mut v=fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)),32<<20).unwrap();let mut count=0;while v.read_frame_raw().unwrap().is_some(){count+=1;}assert_eq!(count,3);
 }
}

#[test]
fn real_audio_worker_preserves_wide_pcm_and_eof(){
 use fvid::audio_thread::AudioPlayback;
 use std::sync::{Arc,Mutex};use transitions::{Capture,Trace,ended};
 for c in cases().into_iter().filter(|c|c["valid"]==true){
  let name=c["file"].as_str().unwrap();let start=c["start"].as_u64().unwrap();let end=c["end"].as_u64().unwrap();
  let r=Mp4AudioReader::open(Cursor::new(bytes(name)),Default::default()).unwrap();
  let trace=Arc::new(Mutex::new(Trace::default()));let shared=trace.clone();
  let mut worker=AudioPlayback::start(Box::new(r),move||Box::new(Capture::new(shared)));worker.play();ended(&worker,0);
  let frames=&trace.lock().unwrap().packets.clone();let got:Vec<_>=frames.iter().flat_map(|f|f.3.iter().copied()).collect();assert_eq!(got,bytes("expected.f32le"),"{name}");
  assert_eq!(frames.iter().map(|f|f.2).collect::<Vec<_>>(),[at(start),at(start+64),at(start+128)],"{name}: workerPTS");
  trace.lock().unwrap().packets.clear();worker.seek(at(start+1));ended(&worker,worker.generation());
  let got:Vec<_>=trace.lock().unwrap().packets.iter().flat_map(|f|f.3.iter().copied()).collect();assert_eq!(got,bytes("expected.f32le")[4..],"{name}: worker seek");
  trace.lock().unwrap().packets.clear();worker.seek(at(end));ended(&worker,worker.generation());assert!(trace.lock().unwrap().packets.is_empty(),"{name}: workerEOF");
 }
}
