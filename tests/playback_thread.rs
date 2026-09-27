use fvid::{
    playback_native::NativeReader,
    playback_thread::{Event, Playback, queue_depth},
};
use std::{
    io::Cursor,
    thread,
    time::{Duration, Instant},
};

/// A queue is a span of presentation time, not a count of pictures: 20 ms of
/// period at 60 Hz buys ten of them, and a slideshow buys two rather than the
/// five seconds a fixed count would hold.
#[test]
fn depth_follows_the_presentation_clock() {
    assert_eq!(queue_depth(Duration::from_millis(20), 1 << 20), 10);
    assert_eq!(queue_depth(Duration::from_millis(100), 1 << 20), 2);
    // A very fast source asks for more pictures than the queue is willing to keep.
    assert_eq!(queue_depth(Duration::from_millis(4), 1 << 20), 24);
    // A clock the container never stated leaves the queue at its widest.
    assert_eq!(queue_depth(Duration::ZERO, 1 << 20), 24);
}

/// Full frames cost memory, and the reader's own budget only ever covered the
/// buffers inside it, so the queue bounds the count the clock asks for itself.
#[test]
fn depth_stays_inside_its_memory_budget() {
    let rgb = |w: usize, h: usize| w * h * 3;
    // 1080p fits the 200 ms the clock wants within the budget: 62 MiB of 64.
    assert_eq!(queue_depth(Duration::from_millis(20), rgb(1920, 1080)), 10);
    // 4K would cost 237 MiB at that depth, so the budget narrows it to 47.
    assert_eq!(queue_depth(Duration::from_millis(20), rgb(3840, 2160)), 2);
    // However large the picture, the window is never left with nothing to show.
    assert_eq!(queue_depth(Duration::from_millis(20), rgb(16384, 16384)), 2);
}

/// A picture that reports no size cannot overdraw the budget, so only the clock bounds it.
#[test]
fn depth_ignores_a_frame_that_costs_nothing() {
    assert_eq!(queue_depth(Duration::from_millis(20), 0), 10);
}

/// The window reads the queue as a buffer level: it has to rise to the depth
/// the source was given, never past it, and fall back to empty when the pictures
/// are taken out and nothing more is coming.
#[test]
fn the_fill_reading_tracks_the_queue() {
    let player = playback();
    // A 2x2 picture costs nothing, so this source is bound by time alone:
    // 200 ms of a 60 Hz clock is twelve pictures.
    assert_eq!(
        player.depth(),
        queue_depth(Duration::from_micros(16_667), 2 * 2 * 3)
    );
    assert_eq!(player.depth(), 12);

    let filled_up = Instant::now() + Duration::from_secs(5);
    while player.filled() < player.depth() && Instant::now() < filled_up {
        assert!(
            player.filled() <= player.depth(),
            "queue reported more than it holds"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(player.filled(), player.depth(), "the queue never filled");

    // Once the stream has run out and the window has taken everything, nothing
    // is left to raise the level again, so it has to read empty.
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match player.poll() {
            Some(Event::Ended(_)) => break,
            Some(Event::Error(error)) => panic!("{error}"),
            None if Instant::now() > until => panic!("the stream never ended"),
            None => thread::sleep(Duration::from_micros(200)),
            Some(Event::Frame(_)) => {}
        }
    }
    while player.poll().is_some() {}
    assert_eq!(
        player.filled(),
        0,
        "took everything and still reports a load"
    );
}

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
