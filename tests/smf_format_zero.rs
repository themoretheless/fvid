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
fn midi_format_zero_cannot_silently_merge_multiple_declared_tracks(){
 let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/smf-format-zero");let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for name in ["single-zero.mid","format-one.mid"]{let bytes=std::fs::read(base.join(name)).unwrap();let expected=decoded(bytes.clone());assert!(expected.iter().any(|(_,p)|p.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap()).abs()>0.01)));let source=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();let end=source.duration();capture::worker_check_with_seek(Box::new(source),&expected,end);}
 let bytes=std::fs::read(base.join("multi-zero.mid")).unwrap();let parsed=fvid::container::smf::SmfFile::parse(&bytes,&Default::default());assert!(parsed.is_err(),"invalidformat0multi-track score silently merged");let error=parsed.unwrap_err().to_string();assert!(error.contains("format0")&&error.contains("one track"),"wrong refusal{error}");assert!(fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).is_err());
}
