#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader {
    fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()
}
#[test]
fn xm_seek_on_a_published_packet_start_selects_that_packet(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-published-seek");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
        let name=case["file"].as_str().unwrap();let bytes=std::fs::read(base.join(case["source"].as_str().unwrap())).unwrap();
        let mut source=open(bytes.clone());let mut decoder=source.make_decoder().unwrap();let mut packets=Vec::new();let mut expected=Vec::new();
        while let Some(p)=source.next_packet().unwrap(){
            let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();expected.push((f.packet.presentation_time(),f.packet.data));packets.push(p);
        }
        for (index,p) in packets.iter().enumerate(){
            let mut cold=open(bytes.clone());let landed=cold.seek_to(p.pts);let next=cold.next_packet().unwrap().unwrap();
            if landed!=p.pts || next.pts!=p.pts || next.data!=p.data {failures.push(format!("{name}: published start{} selects{landed} instead of row{index}",p.pts));}
        }
        for index in [0,1,2,packets.len()/2,packets.len()-2,packets.len()-1]{
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&expected,Duration::from_millis(packets[index].pts as u64),&expected[index..]))).is_err(){failures.push(format!("{name}: actual worker selects wrong suffix at row{index}"));}
        }
        capture::worker_check_with_seek(Box::new(open(bytes)),&expected,source.duration());
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
