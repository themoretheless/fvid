#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(kind:&str,bytes:Vec<u8>)->Box<dyn AudioStream>{match kind{
"flac"=>Box::new(fvid::playback_flac::FlacAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),
"wav"=>Box::new(fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),
"aiff"=>Box::new(fvid::playback_aiff::AiffAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),
"au"=>Box::new(fvid::playback_au::AuAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()),_=>unreachable!()}}
#[test]
fn native_sample_duration_and_public_eof_seek_share_the_integer_grid(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/sample-duration-grid");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut failures=Vec::new();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["file"].as_str().unwrap();let kind=case["kind"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let rate=case["rate"].as_u64().unwrap();let samples=case["samples"].as_u64().unwrap();let size=case["size"].as_u64().unwrap();let end=Duration::from_nanos(case["end_ns"].as_u64().unwrap());let mut source=open(kind,bytes.clone());let published=source.duration().unwrap();if published!=end{failures.push(format!("{name}: publicduration{published:?}instead ofcanonical{end:?}"));}assert_eq!(source.time_of(samples as i64),end);let mut decoder=source.make_decoder().unwrap();let mut packets=Vec::new();let mut flat=Vec::new();let mut cursor=0u64;while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();let pts=Duration::from_nanos((u128::from(cursor)*1_000_000_000/u128::from(rate))as u64);assert_eq!(f.packet.presentation_time(),pts);cursor+=f.packet.data.len()as u64/4;flat.extend_from_slice(&f.packet.data);packets.push((pts,f.packet.data));}assert_eq!(cursor,samples);let wanted=(0..samples).flat_map(|i|([4096.0f32,-4096.0,8192.0][(i/size%3)as usize]/32768.0).to_le_bytes()).collect::<Vec<_>>();assert_eq!(flat,wanted,"{name}: independentPCM");let landed=source.seek_to(samples as i64);assert_eq!(landed,samples as i64);assert!(source.next_packet().unwrap().is_none());assert_eq!(source.time_of(landed),end);
 if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek(open(kind,bytes),&packets,Some(published)))).is_err(){failures.push(format!("{name}: actualpublicEOFseekclock differs"));}
 }
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
