#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(source:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn xm_muting_a_held_note_preserves_sample_phase_and_natural_end(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-volume-lifetime");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let mut source=open(bytes.clone());let mut reference=open(control.clone());assert_eq!(source.duration(),reference.duration());let actual=render(&mut source);let expected=render(&mut reference);let mute=case["mute"].as_u64().unwrap() as usize;let resume=case["resume"].as_u64().unwrap() as usize;let guard=64*4*usize::from(source.channels());assert!(expected[0].1.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01),"{name}: silent control");
        assert!(actual[mute].1[guard..].chunks_exact(4).all(|s|f32::from_le_bytes(s.try_into().unwrap())==0.),"{name}: mute/keyoff did not reach silence");let audible=expected[resume].1[guard..].chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01);assert_eq!(audible,case["audible_after"].as_bool().unwrap(),"{name}: unrelated continuation control");
        let mut wanted=actual.clone();for index in resume..wanted.len(){assert_eq!(actual[index].0,expected[index].0);let cut=if index==resume{guard}else{0};wanted[index].1[cut..].copy_from_slice(&expected[index].1[cut..]);}
        if actual!=wanted{failures.push(format!("{name}: restored volume loses continuous sample phase"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,reference.duration());
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&wanted,Duration::from_millis(resume as u64*120),&wanted[resume..]))).is_err(){failures.push(format!("{name}: actual playback/seek loses the held note"));}
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek(Box::new(open(bytes)),&wanted,source.duration()))).is_err(){failures.push(format!("{name}: actual playback/EOF differs from continuity oracle"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
