#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn decoded(bytes: Vec<u8>) -> Vec<(Duration, Vec<u8>)> {
    let mut source =
        fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        pcm.push((f.packet.presentation_time(), f.packet.data));
    }
    pcm
}
#[test]
fn midi_known_meta_extensions_preserve_tempo_and_track_end(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/smf-meta-extensions");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());let expected=decoded(std::fs::read(base.join("valid.mid")).unwrap());assert!(expected.iter().any(|(_,p)|p.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01)));
 let mut refused=Vec::new();for name in manifest["valid"].as_array().unwrap(){let bytes=std::fs::read(base.join(name.as_str().unwrap())).unwrap();if fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes.clone()),Default::default()).is_err(){refused.push(name.as_str().unwrap().to_owned());continue;}assert_eq!(decoded(bytes.clone()),expected);let source=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();capture::worker_check_with_seek(Box::new(source),&expected,Some(Duration::from_millis(100)));}
 let mut accepted=Vec::new();for name in manifest["short"].as_array().unwrap(){let name=name.as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let parse=fvid::container::smf::SmfFile::parse(&bytes,&Default::default());if parse.is_ok(){accepted.push(name.to_string());}else{let error=parse.unwrap_err().to_string();assert!(error.contains("tempo change")&&error.contains("three bytes"),"{name}: wrong refusal{error}");}let opened=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default());if opened.is_ok(){accepted.push(format!("{name}:playback admitted"));}}
 assert!(accepted.is_empty(),"short tempo payload accepted:{}",accepted.join("; "));assert!(refused.is_empty(),"valid meta extensions refused:{}",refused.join("; "));
}
