#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio_thread::{AudioEvent,AudioPlayback},playback_mp4_audio::Mp4AudioReader};
use std::{io::Cursor,path::Path,sync::{Arc,Mutex},time::{Duration,Instant}};
#[test]
fn active_seek_refuses_the_invalid_ms_adpcm_predictor() {
    let data=std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-adpcm-subtick-seek/invalid-ms-predictor.mp4")).unwrap();
    let mut video=fvid::playback_native::NativeReader::software(Cursor::new(data.clone()),32<<20).unwrap();
    for _ in 0..6 {assert_eq!(video.read_frame_raw().unwrap().unwrap().into_rgb(32<<20).unwrap(),vec![130;16*16*3]);}
    assert!(video.read_frame_raw().unwrap().is_none());
    for target in [Duration::from_micros(16250),Duration::from_nanos(16_250_001)] {
        let stream=Mp4AudioReader::open(Cursor::new(data.clone()),Default::default()).unwrap();
        let trace=Arc::new(Mutex::new(transitions::Trace::default()));let shared=trace.clone();
        let mut worker=AudioPlayback::start(Box::new(stream),move||Box::new(transitions::Capture::new(shared)));
        worker.seek(target);worker.play();let deadline=Instant::now()+Duration::from_secs(3);
        'events:loop {
            assert!(Instant::now()<deadline,"invalid MS predictor seek did not terminate");
            while let Some(event)=worker.poll(){match event {
                AudioEvent::Error(e)=>{assert!(e.contains("MS ADPCM predictor index exceeds coefficient table"),"{e}");assert!(trace.lock().unwrap().packets.is_empty());break 'events;}
                AudioEvent::Ended(_)=>panic!("invalid MS predictor became successful EOF"),_=>{}
            }}
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
