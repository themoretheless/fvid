#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
fn open(bytes:Vec<u8>)->fvid::playback_smf::SmfAudioReader{fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap()}
fn render(bytes:Vec<u8>)->Vec<(Duration,Vec<u8>)>{let mut source=open(bytes);let mut decoder=source.make_decoder().unwrap();let mut out=Vec::new();while let Some(p)=source.next_packet().unwrap(){let f=decoder.decode_packet(&p.data,p.pts,p.duration as u64).unwrap().unwrap();out.push((f.packet.presentation_time(),f.packet.data));}out}
#[test]
fn stopping_one_midi_key_or_channel_does_not_prune_unrelated_unrendered_attacks(){
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/midi-attack-lifetime");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures=Vec::new();
    for case in manifest["cases"].as_array().unwrap(){let name=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let expected=render(control.clone());let actual=render(bytes.clone());let audible=expected[0].1.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01);assert_eq!(audible,case["audible"].as_bool().unwrap(),"{name}: unrelated explicit-note control");
        if actual!=expected{failures.push(format!("{name}: stop event removes an unrelated fresh attack"));}
        capture::worker_check_with_seek(Box::new(open(control)),&expected,Some(Duration::from_millis(300)));
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(||capture::worker_check_with_seek_expected(Box::new(open(bytes)),&expected,Duration::from_millis(100),&expected[1..]))).is_err(){failures.push(format!("{name}: actual worker loses the fresh note"));}
    }
    assert!(failures.is_empty(),"{}",failures.join("; "));
}
