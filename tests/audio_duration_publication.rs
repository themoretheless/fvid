#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::Path,
    time::{Duration, Instant},
};
fn source(bytes: Vec<u8>) -> fvid::playback_webm_audio::WebmAudioReader<Cursor<Vec<u8>>> {
    fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
        .unwrap()
}
#[test]
fn completed_source_duration_is_published_to_the_live_handle() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-duration-publication");
    let mut wrong = Vec::new();
    for (name, millis) in [("control", 60), ("tail", 20), ("empty", 0)] {
        let bytes = std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut direct = source(bytes.clone());
        let mut decoder = direct.make_decoder().unwrap();
        let mut frames = 0;
        let initial = direct.duration();
        assert_eq!(
            initial,
            Some(Duration::from_millis(if name == "empty" { 0 } else { 60 }))
        );
        while let Some(p) = direct.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(p) = direct
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                assert!(
                    p.data
                        .chunks_exact(4)
                        .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
                );
                frames += p.data.len() / 4;
            }
        }
        direct.validate_eof().unwrap();
        assert_eq!(frames, millis as usize * 48);
        let wanted = Some(Duration::from_millis(millis));
        assert_eq!(direct.duration(), wanted);
        let mut playback =
            fvid::audio_thread::AudioPlayback::start(Box::new(source(bytes)), || {
                Box::new(fvid::audio::NullBackend::default())
            });
        assert_eq!(playback.duration(), initial);
        for generation in 0..3 {
            if generation == 1 {
                playback.rewind();
            }
            if generation == 2 {
                playback.seek(Duration::ZERO);
            }
            playback.play();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "worker EOF timed out");
                match playback.poll() {
                    Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => break,
                    Some(fvid::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if playback.duration() != wanted {
                wrong.push(format!(
                    "{name} generation{generation}: handle {:?}, completed source {wanted:?}",
                    playback.duration()
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "worker retained initial duration snapshot: {wrong:?}"
    );
}
