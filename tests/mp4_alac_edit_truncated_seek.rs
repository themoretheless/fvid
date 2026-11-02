#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio_thread::{AudioEvent,AudioPlayback},playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,path::Path,sync::{Arc,Mutex},time::{Duration,Instant}};
#[test]
fn active_edit_seek_validates_the_complete_alac_coded_body() {
    let data=std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-alac-edit-subtick-seek/active-short-body.mp4")).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(data.clone()),32<<20).unwrap();
    for _ in 0..3 {assert_eq!(video.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}
    assert!(video.read_frame_raw().unwrap().is_none());
    for target in [Duration::ZERO,Duration::from_nanos(500001)] {
        let stream=Mp4AudioReader::open(Cursor::new(data.clone()),Default::default()).unwrap();
        let trace=Arc::new(Mutex::new(transitions::Trace::default()));let shared=trace.clone();
        let mut worker=AudioPlayback::start(Box::new(stream),move||Box::new(transitions::Capture::new(shared)));
        worker.seek(target);worker.play();let deadline=Instant::now()+Duration::from_secs(3);
        'events:loop {
            assert!(Instant::now()<deadline,"truncated ALAC active edit seek did not terminate");
            while let Some(event)=worker.poll(){match event {
                AudioEvent::Error(e)=>{assert!(e.contains("ALAC frame is short of its samples"),"{e}");assert!(trace.lock().unwrap().packets.is_empty());break 'events;}
                AudioEvent::Ended(_)=>panic!("truncated ALAC body became successful EOF"),_=>{}
            }}
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
