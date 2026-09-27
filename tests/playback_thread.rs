use fvid::{
    color::{ColourDescription, DisplayTarget, Grade, HdrMetadata, Settings, Transfer},
    playback_native::NativeReader,
    playback_thread::{Event, Frame, Pixels, Playback},
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
    Playback::start(reader, None)
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
    let data = include_bytes!("fixtures/hevc/main-ipb.mp4");
    let mut reader = NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
    assert!(reader.read_frame().unwrap());
    let mut player = Playback::start(reader, None);
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

/// Two by two, a hundred times: the first picture reaches the converter as the
/// packed RGB the reader left behind and every picture after it as planes, so
/// one stream walks both routes.
fn y4m() -> NativeReader<Cursor<Vec<u8>>> {
    let mut bytes = b"YUV4MPEG2 W2 H2 F60:1 Ip C420jpeg\n".to_vec();
    for _ in 0..100 {
        bytes.extend_from_slice(b"FRAME\n\x10\x20\x30\x40\x80\x80");
    }
    let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes)).unwrap();
    assert!(reader.read_frame().unwrap());
    reader
}

fn first_frame(player: &mut Playback) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "playback stalled");
        match player.poll() {
            Some(Event::Frame(frame)) => return frame,
            Some(Event::Error(error)) => panic!("{error}"),
            Some(Event::Ended(_)) => panic!("the stream ended before its first frame"),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

/// BT.709 codes shown as the desktop's own: a curve and nothing else.
fn recurve() -> Grade {
    Grade::new(
        ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        },
        &HdrMetadata::default(),
        Settings::default(),
        None,
    )
}

#[test]
fn a_grade_reaches_the_pictures_a_stream_hands_over_on_both_routes() {
    let Pixels::Rgb(plain) = first_frame(&mut Playback::start(y4m(), None)).pixels else {
        panic!("a stream's first picture is the RGB its reader left");
    };
    let mut player = Playback::start(y4m(), Some(recurve()));
    let Pixels::Rgb(shown) = first_frame(&mut player).pixels else {
        panic!("a graded picture is packed RGB");
    };
    assert_eq!(plain.len(), shown.len());
    assert_ne!(plain, shown, "the grade changed nothing");
    for (before, after) in plain.iter().zip(&shown) {
        let linear = Transfer::Bt709.eotf(f32::from(*before) / 255.0).unwrap();
        let want = (Transfer::Srgb.oetf(linear).unwrap() * 255.0).round() as i16;
        let step = (i16::from(*after) - want).abs();
        assert!(step <= 1, "{before} -> {after}, not {want}");
    }
    // The pictures after the first arrive as planes, and a plane picture has to
    // become packed RGB for a grade to be applied to it on this thread.
    let mut graded = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while graded < 3 {
        assert!(Instant::now() < deadline, "playback stalled");
        match player.poll() {
            Some(Event::Frame(frame)) => match frame.pixels {
                Pixels::Rgb(_) => graded += 1,
                Pixels::Planar(_) => panic!("a graded picture stayed planes"),
            },
            Some(Event::Error(error)) => panic!("{error}"),
            Some(Event::Ended(_)) => panic!("the stream ended early"),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

#[test]
fn a_grade_that_would_change_nothing_leaves_a_picture_as_planes() {
    // BT.709 codes written out as BT.709 codes on their own panel: the lookup
    // a caller gets for asking for nothing is its own input, and copying planes
    // into RGB to prove that would be the work this thread exists to avoid.
    let same = Grade::new(
        ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        },
        &HdrMetadata::default(),
        Settings::video(DisplayTarget::sdr(240.0)),
        None,
    );
    assert!(same.is_identity());
    let mut player = Playback::start(y4m(), Some(same));
    assert!(matches!(first_frame(&mut player).pixels, Pixels::Rgb(_)));
    let mut planes = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while planes < 3 {
        assert!(Instant::now() < deadline, "playback stalled");
        match player.poll() {
            Some(Event::Frame(frame)) => {
                assert!(
                    matches!(frame.pixels, Pixels::Planar(_)),
                    "an identity grade converted the picture"
                );
                planes += 1;
            }
            Some(Event::Error(error)) => panic!("{error}"),
            Some(Event::Ended(_)) => panic!("the stream ended early"),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}
