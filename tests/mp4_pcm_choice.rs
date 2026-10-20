#![cfg(feature="player")]
use fvid::audio::AudioStream;
use std::{io::Cursor,path::Path,time::Duration};
#[path="support/audio_capture.rs"] mod capture;
#[test]
fn automatic_pcm_choice_skips_unsupported_movie_width_without_reordering_manual_tracks() {
    let base=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-pcm-choice");
    let mut wrong=Vec::new();
    for (name,chosen) in [("valid-movie",0),("unsupported-movie",1),("both-movie",1)] {
        let media=std::fs::read(base.join(format!("{name}.mp4"))).unwrap();
        let expected=std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap();
        let mut video=fvid::playback_native::NativeReader::software(Cursor::new(media.clone()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let automatic=fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(media.clone()),Default::default()).unwrap();
        assert_eq!(automatic.audio_tracks().len(),2);
        if automatic.audio_track_index()!=chosen {wrong.push((name,automatic.audio_track_index()));}
        let good=fvid::playback_mp4_audio::Mp4AudioReader::open_at(Cursor::new(media.clone()),Default::default(),chosen).unwrap();
        assert_eq!(good.audio_track_index(),chosen);assert!(good.make_decoder().is_ok());
        let intervals:Vec<_>=expected.chunks_exact(640).enumerate().map(|(i,b)|(Duration::from_millis(i as u64*20),b.to_vec())).collect();
        capture::worker_check(Box::new(good),&intervals);
        if automatic.audio_track_index()==chosen {capture::worker_check(Box::new(automatic),&intervals);}
        if chosen==1 {
            let bad=fvid::playback_mp4_audio::Mp4AudioReader::open_at(Cursor::new(media),Default::default(),0).unwrap();
            assert_eq!(bad.audio_track_index(),0);assert!(bad.make_decoder().is_err());
        }
    }
    assert!(wrong.is_empty(),"unsupported default PCM prevented valid automatic fallback: {wrong:?}");
}
