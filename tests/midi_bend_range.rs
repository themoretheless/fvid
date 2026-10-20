#![cfg(feature="player")]
use fvid::audio::{AudioStream,AudioDecode};
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn base()->std::path::PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/midi-bend-range")}
fn open(bytes:Vec<u8>)->fvid::playback_smf::SmfAudioReader{fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>,channels:u16)->Vec<(Duration,Vec<u8>)>{let mut source=open(bytes);let mut decoder=fvid::codec::midi_decoder::MidiDecoder::new(source.sample_rate(),channels).unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn midi_bend_sensitivity_is_supported_and_changes_pitch(){let base=base();assert_ne!(render(std::fs::read(base.join("range-12.mid")).unwrap(),2),render(std::fs::read(base.join("range-2-default.mid")).unwrap(),2),"RPN0 changes bend sensitivity");}
#[test]
fn midi_bend_sensitivity_acceptance_matches_equivalent_default_bends(){
 let base=base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut failures=Vec::new();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["file"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();assert_eq!(open(bytes.clone()).duration(),open(control.clone()).duration());let mut stereo=Vec::new();for channels in [1,2]{let expected=render(control.clone(),channels);if render(bytes.clone(),channels)!=expected{failures.push(format!("{name}/{channels}: nativeRPN0differs from independent default-range control"));}if channels==2{stereo=expected;}}let end=open(bytes.clone()).duration().unwrap();for target in [Duration::from_millis(100),end]{if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek(Box::new(open(bytes.clone())),&stereo,Some(target)))).is_err(){failures.push(format!("{name}: actualRPN0playback differs"));}}
 }
 let resource=std::fs::read(base.join("range-resource.mid")).unwrap();let expected=render(resource.clone(),2);assert!(expected.iter().flat_map(|(_,p)|p.chunks_exact(4)).all(|s|{let x=f32::from_le_bytes(s.try_into().unwrap());x.is_finite()&&x.abs()<=1.0}));let end=open(resource.clone()).duration();capture::worker_check_with_seek(Box::new(open(resource)),&expected,end);
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
