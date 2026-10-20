#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
#[test]
fn automatic_pcm_choice_skips_unsupported_default_width_without_reordering_manual_tracks() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-pcm-choice");
    let mut wrong=Vec::new();
    for (name,chosen) in [("valid-first",0),("integer-default",1),("float-default",1),("opus-missing-setup",1),("aac-missing-setup",1),("vorbis-missing-setup",1)] {
        let media=std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let expected=std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let automatic=fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(media.clone()),Default::default());
        if let Ok(automatic)=&automatic {
            assert_eq!(automatic.audio_tracks().len(),2);
            if automatic.audio_track_index()!=chosen {wrong.push(format!("{name}: index{}",automatic.audio_track_index()));}
        } else {wrong.push(format!("{name}: {}",automatic.as_ref().err().unwrap()));}
        let good=fvid::playback_webm_audio::WebmAudioReader::open_at(Cursor::new(media.clone()),Default::default(),chosen).unwrap();
        assert_eq!(good.audio_track_index(),chosen);assert!(good.make_decoder().is_ok());
        let intervals:Vec<_>=expected.chunks_exact(640).enumerate().map(|(i,b)|(Duration::from_millis(i as u64*20),b.to_vec())).collect();
        capture::worker_check(Box::new(good),&intervals);
        if let Ok(automatic)=automatic && automatic.audio_track_index()==chosen {capture::worker_check(Box::new(automatic),&intervals);}
        if chosen==1 {
            let error=match fvid::playback_webm_audio::WebmAudioReader::open_at(Cursor::new(media),Default::default(),0) {
                Ok(bad)=>{assert_eq!(bad.audio_track_index(),0);bad.make_decoder().err().expect("unsupported manual setup accepted")},
                Err(error)=>error,
            };
            let needle=match name {"integer-default"=>"272","float-default"=>"288","opus-missing-setup"=>"OpusHead","aac-missing-setup"=>"shorter than two",_=>"Xiph header"};
            assert!(error.to_string().contains(needle),"{name}: {error}");
        }
    }
    assert!(wrong.is_empty(),"unsupported default PCM prevented valid automatic fallback: {wrong:?}");
}
