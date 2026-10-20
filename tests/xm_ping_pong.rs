#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(source:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn xm_ping_pong_keeps_direction_between_successive_sample_steps(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-ping-pong");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut failures=Vec::new();
 let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let mut actual_source=open(bytes.clone());let mut control=open(std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap());assert_eq!(actual_source.duration(),control.duration());let actual=render(&mut actual_source);let expected=render(&mut control);assert_eq!(actual.len(),expected.len());let mut worst=0.0f32;
 for ((pts,a),(ref_pts,b)) in actual.iter().zip(&expected){assert_eq!(pts,ref_pts);assert_eq!(a.len(),b.len());for(x,y)in a.chunks_exact(4).zip(b.chunks_exact(4)){let x=f32::from_le_bytes(x.try_into().unwrap());let y=f32::from_le_bytes(y.try_into().unwrap());assert!(x.is_finite()&&y.is_finite());worst=worst.max((x-y).abs());}}
 eprintln!("{name}: maximum independent PCM error {worst}");
 if worst>1e-6{failures.push(format!("{name}: unfolded forward-loop oracle differs by{worst}"));}
 // First establish independent waveform tolerance; workers then preserve every native byte.
 capture::worker_check_with_seek(Box::new(open(bytes.clone())),&actual,actual_source.duration());capture::worker_check_with_seek_expected(Box::new(open(bytes)),&actual,Duration::from_millis(240),&actual[2..]);
 }
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
