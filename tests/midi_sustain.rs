#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::{Path,PathBuf},time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn base()->PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/midi-sustain")}
fn open(bytes:Vec<u8>)->fvid::playback_smf::SmfAudioReader{fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>)->Vec<(Duration,Vec<u8>)>{let mut source=open(bytes);let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn native_midi_sustain_changes_the_previously_ignored_control(){
    let base=base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap().iter().take(2){let actual=render(std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap());let without=render(std::fs::read(base.join(case["without"].as_str().unwrap())).unwrap());assert!(actual!=without,"sustain must now differ from the previously identical no-pedal control");}
}
#[test]
fn native_midi_sustain_acceptance_matches_explicit_delayed_note_offs(){
    let base=base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let end=open(control.clone()).duration().unwrap();let expected=render(control.clone());let actual=render(bytes.clone());assert!(expected[0].1.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01),"{name}: silent control");if actual!=expected{failures.push(format!("{name}: sustain PCM differs from explicit delayed note-off"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,Some(end));
        for target in [100,200,end.as_millis() as usize]{let suffix=if target==end.as_millis() as usize{&expected[expected.len()..]}else{&expected[target/100..]};if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes.clone())),&expected,Duration::from_millis(target as u64),suffix))).is_err(){failures.push(format!("{name}: sustain worker seek{target} differs"));}}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
