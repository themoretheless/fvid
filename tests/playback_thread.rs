use fvid::{
    playback_native::NativeReader,
    playback_thread::{Event, Playback},
};
use std::{
    io::Cursor,
    thread,
    time::{Duration, Instant},
};

fn playback() -> Playback {
    let mut bytes = b"YUV4MPEG2 W2 H2 F60:1 Ip C420jpeg\n".to_vec();
    for _ in 0..100 {
        bytes.extend_from_slice(b"FRAME\n\x10\x20\x30\x40\x80\x80");
    }
    let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes)).unwrap();
    assert!(reader.read_frame().unwrap());
    Playback::start(reader)
}

#[test]
fn backpressure_preserves_every_frame_and_rewind_generation() {
    let mut player = playback();
    // Let both bounded channels fill before consuming anything.
    thread::sleep(Duration::from_millis(50));
    for generation in 0..2 {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut count = 0;
        loop {
            assert!(Instant::now() < deadline, "playback worker stalled");
            match player.poll() {
                Some(Event::Frame(frame)) if frame.generation == generation => count += 1,
                Some(Event::Ended(at)) if at == generation => break,
                Some(Event::Error(error)) => panic!("{error}"),
                _ => thread::sleep(Duration::from_micros(100)),
            }
        }
        assert_eq!(count, 100);
        if generation == 0 {
            player.rewind();
        }
    }
}

#[test]
fn drop_with_full_channels_stops_both_workers() {
    let (done, result) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let player = playback();
        thread::sleep(Duration::from_millis(50));
        drop(player);
        done.send(()).unwrap();
    });
    result
        .recv_timeout(Duration::from_secs(5))
        .expect("drop deadlocked with full channels");
}

#[test]
fn paused_seek_discards_old_generation_and_delivers_one_planar_preview() {
    use fvid::playback_thread::Pixels;
    let data = include_bytes!("fixtures/hevc/main-ipb.mp4");
    let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
    assert!(reader.read_frame().unwrap());
    let mut player = Playback::start(reader);
    player.pause();
    player.seek(Duration::from_millis(350));
    let generation = player.generation();
    let deadline = Instant::now() + Duration::from_secs(5);
    let preview = loop {
        assert!(Instant::now() < deadline, "seek stalled");
        match player.poll() {
            Some(Event::Frame(frame)) if frame.generation == generation => break frame,
            Some(Event::Error(error)) => panic!("{error}"),
            _ => thread::sleep(Duration::from_millis(1)),
        }
    };
    assert!(matches!(preview.pixels, Pixels::Planar(_)));
    let (start, end, scale) = preview.interval.unwrap();
    assert!(start * 1000 <= 350 * u128::from(scale));
    assert!(end * 1000 > 350 * u128::from(scale));
    thread::sleep(Duration::from_millis(30));
    while let Some(event) = player.poll() {
        if let Event::Frame(frame) = event {
            assert!(frame.generation < generation, "paused preview advanced");
        }
    }
    player.play();
    loop {
        assert!(Instant::now() < deadline, "resume stalled");
        match player.poll() {
            Some(Event::Frame(frame)) if frame.generation == generation => {
                assert_eq!(frame.interval.unwrap().0, end);
                break;
            }
            Some(Event::Error(error)) => panic!("{error}"),
            _ => thread::sleep(Duration::from_millis(1)),
        }
    }
}
