#![cfg(feature="player")]
use fvid::audio::{AudioStream,AudioDecode};
use std::{io::Cursor,path::{Path,PathBuf},time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn base()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-set-volume")}
fn open(bytes:Vec<u8>)->fvid::playback_xm::XmAudioReader{fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>)->(Vec<(Duration,Vec<u8>)>,u32){let mut source=open(bytes);let mut decoder=fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(),source.sample_rate(),source.channels()).unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}(out,decoder.ignored().effects)}
#[test]
fn native_xm_set_volume_changes_the_previously_ignored_control(){let base=base();let(actual,ignored)=render(std::fs::read(base.join("volume-0.xm")).unwrap());let(without,_)=render(std::fs::read(base.join("volume-0-without.xm")).unwrap());assert!(actual!=without&&ignored==0,"native Cxx must now change volume and be reported supported");}
#[test]
fn native_xm_set_volume_acceptance_matches_volume_column(){
    let base=base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let source=open(bytes.clone());let reference=open(control.clone());assert_eq!(source.duration(),reference.duration(),"{name}: unrelated flow/tempo mismatch");let(expected,control_ignored)=render(control.clone());let(actual,ignored)=render(bytes.clone());assert_eq!(control_ignored,0,"{name}: unsupported reference effect");if ignored!=0{failures.push(format!("{name}: supported Cxx still reported ignored"));}if actual!=expected{failures.push(format!("{name}: CxxPCM differs from volume-column control"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,reference.duration());let row=case["seek"].as_u64().unwrap() as usize;
        for (target,suffix) in [(Duration::from_millis(row as u64*120),&expected[row..]),(reference.duration().unwrap(),&expected[expected.len()..])]{if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&expected,target,suffix))).is_err(){failures.push(format!("{name}: Cxxplayback/seek differs from reference"));}}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
