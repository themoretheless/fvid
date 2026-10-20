#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn decode(source:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{let mut engine=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=engine.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn xm_signed8_samples_match_equivalent_signed16_pcm_without_byte_replication_bias(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-sample-scale");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    for value in -128i16..=127 {
        assert_eq!(fvid::container::xm::decode_delta_8(&[value as u8]),vec![value*256]);
    }
    assert_eq!(fvid::container::xm::decode_delta_8(&[127,1,255,129]),vec![32512,-32768,32512,0]);
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){
        let value=case["value"].as_i64().unwrap();let eight=std::fs::read(base.join(case["files"]["8"].as_str().unwrap())).unwrap();let sixteen=std::fs::read(base.join(case["files"]["16"].as_str().unwrap())).unwrap();let mut source=open(eight.clone());let mut reference=open(sixteen.clone());
        let expected_frame=value*256;assert!(reference.module().instruments[0].samples[0].frames.iter().all(|v|i64::from(*v)==expected_frame),"unrelated16bit control error");
        let frames=&source.module().instruments[0].samples[0].frames;if frames.iter().any(|v|i64::from(*v)!=expected_frame){failures.push(format!("value{value}: native8bit expands to{} instead of{expected_frame}",frames[0]));}
        let expected=decode(&mut reference);let actual=decode(&mut source);if actual!=expected {failures.push(format!("value{value}:8bit native PCM differs from equivalent16bit control"));}
        capture::worker_check_with_seek(Box::new(open(sixteen)),&expected,reference.duration());
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(eight)),&expected,Duration::from_millis(120),&expected[1..]))).is_err(){failures.push(format!("value{value}:8bit actual playback differs from16bit control"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
