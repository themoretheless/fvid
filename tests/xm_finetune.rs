#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(source:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn xm_raw_finetune_is_one_128th_of_a_semitone(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-finetune");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut failures=Vec::new();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let fine=case["finetune"].as_i64().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let mut source=open(bytes.clone());let actual=render(&mut source);let rate=8363.0f64*2.0f64.powf(fine as f64/(128.0*12.0));let mut worst=0.0f32;let mut index=0;
 for(_,pcm)in actual.iter().take(2){for frame in pcm.chunks_exact(8){let phase=(index as f64*rate/44100.0)%64.0;let a=phase.floor();let b=(a+1.0)%64.0;let weight=(phase-a)as f32;let va=(a as f32-32.0)/64.0;let vb=(b as f32-32.0)/64.0;let value=va+(vb-va)*weight;let level=((index+1)as f32/256.0).min(0.25);let expected=value*level*(0.5f64.sqrt()as f32);for sample in frame.chunks_exact(4){let x=f32::from_le_bytes(sample.try_into().unwrap());assert!(x.is_finite());worst=worst.max((x-expected).abs());}index+=1;}}
 eprintln!("{name}: maximum independent PCM error{worst}");if worst>1e-6{failures.push(format!("{name}: signed finetune pitch oracle differs by{worst}"));}
 if fine == -128{let mut control=open(std::fs::read(base.join("minus-semitone-control.xm")).unwrap());if actual!=render(&mut control){failures.push(format!("{name}: minus-one-semitone note control differs"));}}
 capture::worker_check_with_seek(Box::new(open(bytes.clone())),&actual,source.duration());capture::worker_check_with_seek_expected(Box::new(open(bytes)),&actual,Duration::from_millis(120),&actual[1..]);
 }
 let mut amiga=open(std::fs::read(base.join("amiga-fine--128.xm")).unwrap());let mut amiga_control=open(std::fs::read(base.join("amiga-minus-semitone-control.xm")).unwrap());assert_eq!(render(&mut amiga),render(&mut amiga_control),"Amiga raw -128 equals one lower note with zero fine tune");
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
