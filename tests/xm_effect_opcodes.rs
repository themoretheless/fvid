#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(source:&mut dyn AudioStream)->Vec<(Duration,Vec<u8>)>{let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn xm_flow_and_tempo_effects_use_literal_format_opcodes(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-effect-opcodes");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let mut source=open(bytes.clone());let mut reference=open(control.clone());let paths=source.module().timeline().iter().map(|row|vec![row.order,row.row]).collect::<Vec<_>>();let wanted=case["paths"].as_array().unwrap().iter().map(|p|p.as_array().unwrap().iter().map(|v|v.as_u64().unwrap() as usize).collect::<Vec<_>>()).collect::<Vec<_>>();
        if paths!=wanted{failures.push(format!("{name}: wrong played row path"));}if source.duration()!=reference.duration()||source.module().duration()!=reference.module().duration(){failures.push(format!("{name}: wrong source timeline duration"));}
        let expected=render(&mut reference);let actual=render(&mut source);assert!(expected[0].1.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01),"{name}: silent control");if actual!=expected{failures.push(format!("{name}: native PCM/PTS differs from expanded effect-free timeline"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,reference.duration());
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek(Box::new(open(bytes.clone())),&expected,reference.duration()))).is_err(){failures.push(format!("{name}: actual effect playback differs from expanded control"));}
        // Native sample-grid PTS may round below encoded milliseconds: seek
        // inside the second row and compare the entire coarse landed suffix.
        let mid=expected.len()/2;let ticks=(expected[mid].0.as_nanos()*1000).div_ceil(1_000_000) as u64;
        let target=Duration::from_micros(ticks+1000);
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes)),&expected,target,&expected[mid..]))).is_err(){failures.push(format!("{name}: effect seek differs from expanded suffix"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
