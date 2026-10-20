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
fn midi_sequence_numbers_do_not_replace_textual_track_names(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/smf-sequence-name");let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let expected=decoded(std::fs::read(base.join("audio-control.mid")).unwrap());let mut failures=Vec::new();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["file"].as_str().unwrap();let wanted=case["name"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let parsed=fvid::container::smf::SmfFile::parse(&bytes,&Default::default()).unwrap();if parsed.name!=wanted{failures.push(format!("{name}: parsername{:?}instead of{wanted:?}",parsed.name));}let source=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();if source.audio_tracks()[0].name!=wanted{failures.push(format!("{name}: publicaudio tracklabel differs"));}assert_eq!(decoded(bytes),expected);capture::worker_check_with_seek(Box::new(source),&expected,Some(Duration::from_millis(100)));}
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
