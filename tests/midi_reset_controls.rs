#![cfg(feature="player")]
use fvid::audio::{AudioStream,AudioDecode};
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_smf::SmfAudioReader{fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>,channel:u8)->(Vec<(Duration,Vec<u8>)>,u8){let mut source=open(bytes);let mut decoder=fvid::codec::midi_decoder::MidiDecoder::new(source.sample_rate(),source.channels()).unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}(out,decoder.program(channel).unwrap())}
#[test]
fn midi_controller_reset_retunes_active_notes_and_preserves_volume_and_program(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/midi-reset-controls");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let channel=case["channel"].as_u64().unwrap() as u8;let program=case["program"].as_u64().unwrap() as u8;let(expected,reference_program)=render(control.clone(),channel);let(actual,actual_program)=render(bytes.clone(),channel);
        assert_eq!(reference_program,program,"{name}: unrelated program control error");assert_eq!(actual[0],expected[0],"{name}: unrelated PCM difference before CC121");
        if actual_program!=program{failures.push(format!("{name}: CC121 changes program{program} to{actual_program}"));}if actual!=expected{failures.push(format!("{name}: CC121 PCM differs from explicit expression/pitch reset"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,Some(Duration::from_millis(400)));
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes)),&expected,Duration::from_millis(100),&expected[1..]))).is_err(){failures.push(format!("{name}: actual CC121 playback differs from explicit reset"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
