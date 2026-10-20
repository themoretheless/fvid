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
fn fixture_base()->std::path::PathBuf{Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/smf-smpte")}
#[test]
fn midi_smpte_supported_clock_metadata_is_explicit(){let bytes=std::fs::read(fixture_base().join("24-basic.mid")).unwrap();let file=fvid::container::smf::SmfFile::parse(&bytes,&Default::default()).unwrap();assert_eq!(file.ticks_per_quarter,0);assert_eq!(file.time_division,fvid::container::smf::TimeDivision::Smpte{frames_per_second_num:24,frames_per_second_den:1,ticks_per_frame:10});}

#[test]
fn midi_smpte_acceptance_matches_independent_ppq_clocks(){
 let base=fixture_base();let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();let mut failures=Vec::new();let mut video=fvid::playback_native::NativeReader::software(Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),32<<20).unwrap();assert!(video.read_frame_raw().unwrap().is_some());assert!(video.read_frame_raw().unwrap().is_none());
 for case in manifest["cases"].as_array().unwrap(){let name=case["file"].as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let control=std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap();let reference=fvid::container::smf::SmfFile::parse(&control,&Default::default()).unwrap();let expected=decoded(control);assert!(expected.iter().any(|(_,pcm)|pcm.chunks_exact(4).any(|s|f32::from_le_bytes(s.try_into().unwrap())!=0.0)));let parsed=match fvid::container::smf::SmfFile::parse(&bytes,&Default::default()){Ok(p)=>p,Err(e)=>{failures.push(format!("{name}: validSMPTErefused{e}"));continue;}};let fps=case["fps"].as_u64().unwrap() as u32;let(num,den)=if fps==29{(30000,1001)}else{(fps,1)};assert_eq!(parsed.time_division,fvid::container::smf::TimeDivision::Smpte{frames_per_second_num:num,frames_per_second_den:den,ticks_per_frame:case["ticks_per_frame"].as_u64().unwrap() as u8});assert_eq!(parsed.ticks_per_quarter,0);
assert_eq!(parsed.duration(),reference.duration(),"{name}: clock");assert_eq!(parsed.windows(),reference.windows());for window in 0..parsed.windows(){assert_eq!(parsed.window_packet(window),reference.window_packet(window),"{name}: exact event clock window{window}");}let actual=decoded(bytes.clone());assert_eq!(actual,expected,"{name}: PCM/PTS");let source=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes.clone()),Default::default()).unwrap();let end=source.duration().unwrap();capture::worker_check_with_seek(Box::new(source),&expected,Some(end));let source=fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).unwrap();capture::worker_check_with_seek(Box::new(source),&expected,Some(Duration::from_millis(100)));}
 for name in manifest["invalid"].as_array().unwrap(){let name=name.as_str().unwrap();let bytes=std::fs::read(base.join(name)).unwrap();let error=fvid::container::smf::SmfFile::parse(&bytes,&Default::default()).unwrap_err().to_string();assert!(error.contains("SMPTE"),"{name}:wrongrefusal{error}");assert!(error.contains(if name.ends_with("-0.mid"){"zero ticks"}else{"frame-rate code"}),"{name}:specificclockrefusal{error}");assert!(fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes),Default::default()).is_err());}
 assert!(failures.is_empty(),"{}",failures.join("; "));
}
