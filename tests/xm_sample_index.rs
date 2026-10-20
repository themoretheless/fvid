#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
#[test]
fn xm_zero_based_sample_maps_select_the_declared_waveform(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-sample-index");
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
        let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let mut source=open(bytes.clone());
        for (index,value) in case["values"].as_array().unwrap().iter().enumerate(){let frames=&source.module().instruments[0].samples[index].frames;assert_eq!(frames.len(),128,"{name}: unrelated sample length");assert!(frames.iter().all(|v|*v==frames[0]&&i64::from(*v).signum()==value.as_i64().unwrap().signum()),"{name}: unrelated sample shape/polarity error");}
        for (row,slot) in case["mapping"].as_array().unwrap().iter().enumerate(){let expected=slot.as_u64().unwrap() as usize;if source.module().sample_slot(0,49+row as u8)!=Some(expected){failures.push(format!("{name}: note{} does not select zero-based sample{expected}",49+row));}}
        let mut decoder=source.make_decoder().unwrap();let mut expected=Vec::new();
        while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();expected.push((f.packet.presentation_time(),f.packet.data));}
        for row in 0..2 {let slot=case["mapping"][row].as_u64().unwrap() as usize;let value=case["values"][slot].as_i64().unwrap();let samples=expected[row].1.chunks_exact(4).map(|s|f32::from_le_bytes(s.try_into().unwrap())).collect::<Vec<_>>();if !samples.iter().any(|v|v.abs()>0.01)||samples.iter().any(|v|(*v>0.&&value<0)||(*v<0.&&value>0)){failures.push(format!("{name}: row{row} renders silence or the other sample's polarity"));}}
        capture::worker_check_with_seek(Box::new(open(bytes.clone())),&expected,source.duration());
        capture::worker_check_with_seek_expected(Box::new(open(bytes)),&expected,Duration::from_millis(120),&expected[1..]);
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
