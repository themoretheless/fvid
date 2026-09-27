use fvid::{
    color::{
        ColourDescription, DisplayTarget, Grade, HdrMetadata, Interpolation, Lut, Settings,
        ToneMap, Transfer,
    },
    playback_native::{NativeReader, planar8_to_rgb},
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

/// A graded picture is always packed RGB on this thread; one with nothing done
/// to its colour is packed only where the reader left it that way, which for an
/// MP4 is its first picture and planes after that.
fn packed(pixels: Pixels, budget: usize) -> Vec<u8> {
    match pixels {
        Pixels::Rgb(rgb) => rgb,
        Pixels::Planar(planes) => {
            let mut rgb = Vec::new();
            planar8_to_rgb(&planes, &mut rgb, budget).expect("converted");
            rgb
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

/// A real HDR10 clip on the thread that shows it, which is where a grade has to
/// land for a viewer to see anything of it. The file is Main10 HEVC that states
/// its light nowhere but in-band SEI, so the peak the grade compresses from comes
/// out of the coding; asked blind, the same settings have only the panel's own
/// 100 cd/m² to work against and their curve becomes clipping. Measured on this
/// clip's first two pictures, codes at the ceiling per channel: 835/631/428 and
/// 800/595/413 against the master's stated peak, versus 957/665/580 and
/// 950/645/568 blind — the shoulder holds highlights back on decoded pixels, not
/// only on the code values the grade's own tests hand it.
#[test]
fn a_real_hdr10_picture_is_tone_mapped_by_the_thread_that_shows_it() {
    fn hdr10(data: &[u8]) -> NativeReader<Cursor<Vec<u8>>> {
        let mut reader = NativeReader::without_memory_limit(Cursor::new(data.to_vec())).unwrap();
        assert!(reader.read_frame().unwrap());
        reader
    }
    fn at_the_ceiling(rgb: &[u8]) -> [usize; 3] {
        let mut counts = [0usize; 3];
        for pixel in rgb.as_chunks::<3>().0 {
            for (channel, code) in pixel.iter().enumerate() {
                if *code == 255 {
                    counts[channel] += 1;
                }
            }
        }
        counts
    }
    let data = include_bytes!("fixtures/hevc/hdr10.mp4").to_vec();
    let reader = hdr10(&data);
    let (signal, hdr) = (reader.colour(), reader.hdr());
    assert!(signal.is_hdr());
    assert_eq!(hdr.content_light(100.0).max_cll, 1_000.0);
    let settings = Settings::video(DisplayTarget::sdr(100.0));
    let stated = Grade::new(signal, &hdr, settings, None);
    let blind = Grade::new(signal, &HdrMetadata::default(), settings, None);
    assert_eq!(stated.plan().tone_map, Some(ToneMap::Mobius));
    assert_eq!(blind.plan().tone_map, Some(ToneMap::Clip));

    let plain_reader = hdr10(&data);
    let budget = plain_reader.rgb_budget();
    let mut plain = Playback::start(plain_reader, None);
    let mut shown = Playback::start(hdr10(&data), Some(stated));
    let mut burnt = Playback::start(hdr10(&data), Some(blind));

    for frame in 0..2 {
        let untouched = first_frame(&mut plain);
        if frame > 0 {
            assert!(
                matches!(untouched.pixels, Pixels::Planar(_)),
                "an ungraded picture was made to lose its planes"
            );
        }
        let untouched = packed(untouched.pixels, budget);
        let graded = first_frame(&mut shown);
        assert!(
            matches!(graded.pixels, Pixels::Rgb(_)),
            "a graded picture stayed planes"
        );
        let graded = packed(graded.pixels, budget);
        let clipped = packed(first_frame(&mut burnt).pixels, budget);
        assert_ne!(untouched, graded, "the grade reached no pixels");
        let rolled = at_the_ceiling(&graded);
        let burnt = at_the_ceiling(&clipped);
        for channel in 0..3 {
            assert!(
                rolled[channel] < burnt[channel],
                "frame {frame} channel {channel}: the master's own peak left {rolled:?} at the ceiling, blind clipping {burnt:?}"
            );
        }
    }
}

/// A cube read off disk and applied by the thread that shows the pictures, then
/// checked against the file's own text. The premise is measured first: with no
/// cube these settings are the identity on this stream, so every code a graded
/// picture differs by is the cube's doing — and that difference is recomputed
/// here by interpolating node values parsed straight out of the `.cube`, with
/// neither the crate's parser nor its sampler. The stream carries a hundred and
/// ninety-two colours rather than four, so the comparison walks cells across the
/// cube instead of one point on its diagonal, and both routes are compared: the
/// packed first picture and the planes after it, because a grade reaches a viewer
/// through whichever one the decoder happened to leave. Measured on this stream:
/// 3 072 channels over five hundred and seventy-two distinct colours, all of them
/// within one code of what the file's own numbers interpolate to.
#[test]
fn a_cube_on_disk_regrades_the_pictures_the_thread_shows() {
    const SIZE: usize = 17;
    const CUBE: &str = include_str!("fixtures/lut/grade-17.cube");
    fn graded_from(bytes: Vec<u8>) -> NativeReader<Cursor<Vec<u8>>> {
        let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes)).unwrap();
        assert!(reader.read_frame().unwrap());
        reader
    }
    // 32x16, 4:2:0, two frames: luma runs across the width while both chroma
    // planes run down their own axes, so no two chroma cells share a colour.
    fn stream() -> Vec<u8> {
        let (width, height) = (32usize, 16usize);
        let mut bytes = b"YUV4MPEG2 W32 H16 F60:1 Ip C420jpeg\n".to_vec();
        for _ in 0..2 {
            bytes.extend_from_slice(b"FRAME\n");
            for y in 0..height {
                for x in 0..width {
                    bytes.push(16 + (x * 7 + y * 3) as u8 % 220);
                }
            }
            for cy in 0..height / 2 {
                for cx in 0..width / 2 {
                    bytes.push(16 + (cx * 14) as u8);
                }
            }
            for cy in 0..height / 2 {
                for cx in 0..width / 2 {
                    bytes.push(16 + (cy * 28) as u8);
                }
            }
        }
        bytes
    }
    // The file lists red fastest, then green, then blue.
    let nodes: Vec<[f64; 3]> = CUBE
        .lines()
        .filter(|line| {
            line.as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'-' || *byte == b'.')
        })
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            [
                parts[0].parse().expect("red"),
                parts[1].parse().expect("green"),
                parts[2].parse().expect("blue"),
            ]
        })
        .collect();
    assert_eq!(nodes.len(), SIZE.pow(3));
    let lookup = |code: [f64; 3]| -> [f64; 3] {
        let mut low = [0usize; 3];
        let mut frac = [0.0f64; 3];
        for axis in 0..3 {
            let pos = code[axis].clamp(0.0, 1.0) * f64::from(SIZE as u32 - 1);
            let index = pos.floor() as usize;
            let index = index.min(SIZE - 2);
            low[axis] = index;
            frac[axis] = pos - f64::from(index as u32);
        }
        let mut out = [0.0f64; 3];
        for blue in 0..2 {
            for green in 0..2 {
                for red in 0..2 {
                    let weight = [red, green, blue]
                        .iter()
                        .zip(&frac)
                        .fold(1.0, |product, (corner, f)| {
                            product * if *corner == 0 { 1.0 - f } else { *f }
                        });
                    let node = nodes
                        [(low[0] + red) + (low[1] + green) * SIZE + (low[2] + blue) * SIZE * SIZE];
                    for channel in 0..3 {
                        out[channel] += weight * node[channel];
                    }
                }
            }
        }
        out
    };
    let reader = graded_from(stream());
    let budget = reader.rgb_budget();
    // The stream states no colour of its own, so the grade is told what the bytes
    // mean the way the rest of this file tells it, and the settings below are the
    // ones already proven to ask for nothing.
    let signal = ColourDescription {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
    let mut settings = Settings::video(DisplayTarget::sdr(240.0));
    settings.interpolation = Interpolation::Trilinear;
    assert!(
        Grade::new(signal, &HdrMetadata::default(), settings, None).is_identity(),
        "these settings already move this stream on their own, so a cube would not be the only change"
    );
    let grade = Grade::new(
        signal,
        &HdrMetadata::default(),
        settings,
        Some(Lut::from_cube(CUBE).expect("a written cube is a cube")),
    );
    assert!(
        !grade.is_identity(),
        "a cube of this size claims to change nothing"
    );
    let mut plain = Playback::start(graded_from(stream()), None);
    let mut shown = Playback::start(graded_from(stream()), Some(grade));
    let mut colours = std::collections::HashSet::new();
    let mut channels = 0usize;
    for frame in 0..2 {
        let untouched = packed(first_frame(&mut plain).pixels, budget);
        let graded = packed(first_frame(&mut shown).pixels, budget);
        assert_eq!(untouched.len(), graded.len());
        assert_ne!(untouched, graded, "the cube reached no pixels");
        for (pixel, out) in untouched
            .as_chunks::<3>()
            .0
            .iter()
            .zip(graded.as_chunks::<3>().0)
        {
            colours.insert(*pixel);
            let want = lookup([
                f64::from(pixel[0]) / 255.0,
                f64::from(pixel[1]) / 255.0,
                f64::from(pixel[2]) / 255.0,
            ]);
            for channel in 0..3 {
                let expected = (want[channel] * 255.0).round() as i16;
                let step = (i16::from(out[channel]) - expected).abs();
                assert!(
                    step <= 1,
                    "frame {frame} pixel {pixel:?} channel {channel}: shown {}, the cube's own text says {expected}",
                    out[channel]
                );
                channels += 1;
            }
        }
    }
    assert!(
        colours.len() >= 500,
        "only {} colours reached the cube, which does not walk it",
        colours.len()
    );
    assert!(
        channels >= 3 * 1024,
        "only {channels} channels were compared against the cube's text"
    );
}
