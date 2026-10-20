#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
#[test]
fn xm_tail_end_uses_one_absolute_rounding_and_retains_the_last_seek_millisecond(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-tail-endpoint");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["file"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let mut source=open(bytes.clone());let last=source.packet_count()-1;let(start,length)=source.module().packet_time(last);let end_ms=(start+length)/1000;let wanted_end=Duration::from_millis(end_ms);let tail_start=(start/1000) as i64;let rate=u64::from(source.sample_rate());let stride=4*usize::from(source.channels());let mut decoder=source.make_decoder().unwrap();let mut actual=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();actual.push((f.packet.presentation_time(),f.packet.data));}
        if source.duration()!=Some(wanted_end){failures.push(format!("{name}: endpoint{:?} differs from absolute{wanted_end:?}",source.duration()));}
        let prior=actual[..last].iter().map(|(_,p)|p.len()/stride).sum::<usize>();assert_eq!(prior as u64,(start/1000)*rate/1000,"{name}: unrelated prefix frame drift");assert!(actual[last].1.chunks_exact(4).all(|s|f32::from_le_bytes(s.try_into().unwrap())==0.),"{name}: unrelated nonsilent tail");let total=(end_ms*rate/1000) as usize;let mut wanted=actual.clone();assert!(wanted[last].1.len()<=(total-prior)*stride);wanted[last].1.resize((total-prior)*stride,0);
        if actual!=wanted{failures.push(format!("{name}: release tail ends before absolute sample endpoint"));}
        for (target,landed,has_packet) in [(end_ms as i64,end_ms as i64,false),(end_ms as i64-1,tail_start,true)]{let mut cold=open(bytes.clone());let at=cold.seek_to(target);let next=cold.next_packet().unwrap();if at!=landed||next.is_some()!=has_packet{failures.push(format!("{name}: seek{target}lands{at}instead of{landed},packet={}",next.is_some()));}}
        for(target,suffix)in[(wanted_end,&wanted[wanted.len()..]),(wanted_end-Duration::from_millis(1),&wanted[last..])]{if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&wanted,target,suffix))).is_err(){failures.push(format!("{name}: actual tail playback/navigation differs from absolute endpoint"));}}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
