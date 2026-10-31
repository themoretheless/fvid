#![cfg(feature="player")]
use std::{collections::BTreeMap,io::Cursor,path::Path,time::Duration};
const BUDGET:usize=32<<20;
fn bytes(name:&str)->Vec<u8>{std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-reordered-seek").join(name)).unwrap()}
fn cases()->Vec<serde_json::Value>{serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"].as_array().unwrap().clone()}
fn frames(r:&mut fvid::playback_mp4::Mp4VideoReader<Cursor<Vec<u8>>>)->Vec<(usize,i64,i64,Vec<u8>)>{
 static ORACLES:std::sync::OnceLock<[Vec<u8>;3]>=std::sync::OnceLock::new();let oracles=ORACLES.get_or_init(||std::array::from_fn(|i|bytes(&format!("frame-{i}.yuv16"))));
 let mut out=Vec::new();while let Some(f)=r.read_frame().unwrap(){
  let pcm:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|x|x.to_le_bytes()).collect();assert_eq!(pcm,oracles[f.sample_index%3]);out.push((f.sample_index,f.presentation_time.ticks,f.duration.ticks,pcm));
 }assert!(r.read_frame().unwrap().is_none());out
}
#[test]
fn all_landings_and_decoded_suffixes_match_original_prefix_coverage(){
 for c in cases(){
  let name=c["file"].as_str().unwrap();let mut r=fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(bytes(name)),Default::default(),BUDGET).unwrap();
  let samples=r.track().samples.expanded().unwrap().to_vec();assert_eq!(samples.len(),c["count"].as_u64().unwrap()as usize);
  let mut expected=BTreeMap::new();for(i,s)in samples.iter().enumerate(){assert_eq!(s.pts,c["pts"][i].as_i64().unwrap());assert!(s.sync);expected.insert(s.pts,i);}
  let seq=frames(&mut r);assert_eq!(seq.iter().map(|f|f.0).collect::<Vec<_>>(),expected.values().copied().collect::<Vec<_>>(),"{name}: full display order");
  for target in expected.keys().next().unwrap()-2..=expected.keys().next_back().unwrap()+2{
   // Independent compatibility oracle: the original full-prefix algorithm.
   let mut index=0;let mut prefix=None;
   for(i,s)in samples.iter().enumerate(){if s.sync&&s.pts<=target&&prefix.is_none_or(|p|p<s.pts){index=i;}prefix=Some(prefix.map_or(s.pts,|p:i64|p.max(s.pts)));}
   assert_eq!(r.seek_to_sync(target),samples[index].pts,"{name}: landing{target}");
   let got=frames(&mut r);let want:Vec<_>=seq.iter().filter(|f|f.0>=index).cloned().collect();assert_eq!(got,want,"{name}: suffix{target}");
  }
  r.rewind();assert_eq!(frames(&mut r),seq,"{name}: rewind");
 }
}
#[test]
fn native_rgb_seeks_preserve_all_intervals_and_pixels(){
 for c in cases(){
  let name=c["file"].as_str().unwrap();let mut r=fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)),BUDGET).unwrap();let mut seq=Vec::new();
  while let Some(f)=r.read_frame_raw().unwrap(){seq.push((r.frame_interval().unwrap(),f.into_rgb(BUDGET).unwrap()));}
  let expected:BTreeMap<_,_>=c["pts"].as_array().unwrap().iter().filter_map(|p|{let n=p.as_i64().unwrap();(n>=0).then_some((n,()))}).collect();assert_eq!(seq.len(),expected.len(),"{name}: native display count");
  for ticks in 0..=*expected.keys().next_back().unwrap()+2{
   let want=seq.iter().rfind(|(span,_)|span.0<=ticks as u128).unwrap_or(&seq[0]);
   let got=r.seek_raw(Duration::from_millis(ticks as u64*20)).unwrap().unwrap().into_rgb(BUDGET).unwrap();assert_eq!(got,want.1,"{name}: RGB{ticks}");assert_eq!(r.frame_interval(),Some(want.0),"{name}: interval{ticks}");
  }
 }
}
#[test]
fn dense_reordered_video_now_fits_the_unchanged_two_mib_budget(){
 let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ordered-index-budget/reordered.mp4");
 let mut r=fvid::playback_mp4::Mp4VideoReader::open_software(Cursor::new(std::fs::read(path).unwrap()),Default::default(),2<<20).unwrap();
 let oracles:[Vec<u8>;3]=std::array::from_fn(|i|bytes(&format!("frame-{i}.yuv16")));
 for rank in 0..150000{
  let i=if rank<3{[0,2,1][rank]}else{rank};let f=r.read_frame().unwrap().unwrap();assert_eq!((f.sample_index,f.presentation_time.ticks,f.duration.ticks),(i,rank as i64,1));let pcm:Vec<_>=f.picture.y.iter().chain(&f.picture.cb).chain(&f.picture.cr).flat_map(|x|x.to_le_bytes()).collect();assert_eq!(pcm,oracles[i%3]);
 }assert!(r.read_frame().unwrap().is_none());r.rewind();assert_eq!(r.read_frame().unwrap().unwrap().presentation_time.ticks,0);assert_eq!(r.seek_to_sync(149999),149999);assert_eq!(r.read_frame().unwrap().unwrap().presentation_time.ticks,149999);assert!(r.read_frame().unwrap().is_none());
}
