use fvid::{
    color::{
        ColourDescription, DisplayTarget, Grade, HdrMetadata, Interpolation, Log, Lut, Lut3d,
        Settings, ToneMap, Transfer,
    },
    playback_native::{NativeReader, planar8_to_rgb},
    playback_thread::{Event, Frame, Pixels, Playback, queue_depth},
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
    // Oversized frames narrow the queue to one slot; admission separately
    // rejects a frame whose own payload exceeds the budget.
    assert_eq!(queue_depth(Duration::from_millis(20), rgb(16384, 16384)), 1);
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
    assert!(matches!(preview.pixels, Pixels::Planar(..)));
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
fn y4m_bytes() -> Vec<u8> {
    let mut bytes = b"YUV4MPEG2 W2 H2 F60:1 Ip C420jpeg\n".to_vec();
    for _ in 0..100 {
        bytes.extend_from_slice(b"FRAME\n\x10\x20\x30\x40\x80\x80");
    }
    bytes
}

fn y4m() -> NativeReader<Cursor<Vec<u8>>> {
    let mut reader = NativeReader::without_memory_limit(Cursor::new(y4m_bytes())).unwrap();
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

/// A picture with nothing done to its colour is packed only where the reader
/// left it that way, which for an MP4 is its first picture and planes after
/// that. Planes that still owe a grade the shader carries are graded here too,
/// because the window reads that table and a test asking what was shown has to.
fn packed(pixels: Pixels, budget: usize) -> Vec<u8> {
    match pixels {
        Pixels::Rgb(rgb) => rgb,
        Pixels::Packed(planes, grade) => {
            let mut rgb = Vec::new();
            planes
                .to_rgb(&mut rgb, budget)
                .expect("source-depth conversion");
            if let Some(grade) = grade {
                grade.apply(&mut rgb);
            }
            rgb
        }
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        Pixels::Surface(frame) => {
            let planes = fvid::playback_native::surface_to_packed(&frame.surface, frame.colour)
                .unwrap()
                .rotated(frame.rotation)
                .unwrap();
            let mut rgb = Vec::new();
            planes.to_rgb(&mut rgb, budget).unwrap();
            if let Some(grade) = frame.grade {
                grade.apply(&mut rgb);
            }
            rgb
        }
        Pixels::Planar(planes, grade) => {
            let mut rgb = Vec::new();
            planar8_to_rgb(&planes, &mut rgb, budget).expect("converted");
            if let Some(grade) = grade {
                grade.apply(&mut rgb);
            }
            rgb
        }
    }
}

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
        for _ in 0..height / 2 {
            for cx in 0..width / 2 {
                bytes.push(16 + (cx * 14) as u8);
            }
        }
        for cy in 0..height / 2 {
            for _ in 0..width / 2 {
                bytes.push(16 + (cy * 28) as u8);
            }
        }
    }
    bytes
}

/// BT.709 codes shown as the desktop's own, as a source would state them.
fn recurve_signal() -> ColourDescription {
    ColourDescription {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    }
}

/// BT.709 codes shown as the desktop's own: a curve and nothing else.
fn recurve() -> Grade {
    Grade::new(
        recurve_signal(),
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
    // The pictures after the first arrive as planes. A grade that is one table
    // the fragment shader can bind travels with them rather than copying the
    // whole picture into RGB on this thread, and the table it travels with is
    // the one asked for above.
    let mut owed = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while owed < 3 {
        assert!(Instant::now() < deadline, "playback stalled");
        match player.poll() {
            Some(Event::Frame(frame)) => {
                let Pixels::Planar(_, grade) = frame.pixels else {
                    panic!("a plane picture was made to lose its planes");
                };
                let grade = grade.expect("the picture kept no grade for the shader");
                assert!(grade.is_shader_look());
                owed += 1;
            }
            Some(Event::Error(error)) => panic!("{error}"),
            Some(Event::Ended(_)) => panic!("the stream ended early"),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

/// The other half of the plane route: a grade the fragment shader cannot bind is
/// this thread's own work, because two lookups read one after the other need the
/// picture in RGB. A per-channel LUT after the plan folds into the one table the
/// shader reads and so still travels with the planes; a grid after it does not,
/// and the picture is converted and graded here instead. Held to the bytes, not
/// just to the shape it arrives in, because a route that quietly dropped the
/// second lookup would still hand over RGB.
#[test]
fn a_grade_the_shader_cannot_carry_is_applied_on_the_thread() {
    let budget = NativeReader::without_memory_limit(Cursor::new(y4m_bytes()))
        .unwrap()
        .rgb_budget();
    let kept = Grade::new(
        recurve_signal(),
        &HdrMetadata::default(),
        Settings::default(),
        Some(Lut::from_cube("LUT_1D_SIZE 2\n0.0 1.0 0.0\n1.0 0.0 1.0\n").unwrap()),
    );
    assert!(kept.is_shader_look(), "a per-channel LUT is one table");
    let owed = Grade::new(
        recurve_signal(),
        &HdrMetadata::default(),
        Settings::default(),
        Some(Lut::Three(Lut3d::from_fn(2, |rgb| {
            [1.0 - rgb[0], rgb[1], rgb[2]]
        }))),
    );
    assert!(!owed.is_shader_look(), "two grids are not one lookup");

    // The 1D chain folds into the table the shader reads, so that grade still
    // rides with the planes instead of converting them here.
    let mut riding = Playback::start(y4m(), Some(kept));
    let Pixels::Rgb(_) = first_frame(&mut riding).pixels else {
        panic!("a stream's first picture is the RGB its reader left");
    };
    let frame = first_frame(&mut riding);
    let Pixels::Planar(planes, grade) = frame.pixels else {
        panic!("a folded 1D LUT converted the picture instead of riding with it");
    };
    assert_eq!(
        planes.y,
        [0x10, 0x20, 0x30, 0x40],
        "the planes were touched"
    );
    assert!(
        grade
            .expect("no table rode with the planes")
            .is_shader_look(),
        "the riding grade is not one lookup"
    );

    // The renderer now carries conversion plus an authored look as two stages.
    // Verify both stages against CPU pixels while preserving subsequent planes.
    let mut plain = Playback::start(y4m(), None);
    let mut shown = Playback::start(y4m(), Some(owed.clone()));
    for frame in 0..3 {
        let mut want = packed(first_frame(&mut plain).pixels, budget);
        owed.apply(&mut want);
        let after = first_frame(&mut shown);
        if frame > 0 {
            let Pixels::Planar(_, grade) = &after.pixels else {
                panic!("frame {frame}: a GPU-compatible chain lost its planes");
            };
            assert!(grade.as_ref().is_some_and(|g| g.is_gpu_grade()));
        }
        assert_eq!(
            packed(after.pixels, budget),
            want,
            "frame {frame} was not graded both ways"
        );
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
                    matches!(frame.pixels, Pixels::Planar(..)),
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
                matches!(untouched.pixels, Pixels::Planar(..) | Pixels::Packed(..)),
                "an ungraded picture was made to lose its planes"
            );
        }
        let untouched = packed(untouched.pixels, budget);
        let graded = first_frame(&mut shown);
        // A tone map folds channels, so this grade has no byte tables and reaches
        // the window as the one grid the fragment shader samples: the picture it
        // travels in is whatever route the thread took, packed for the first
        // picture and planes with the grid riding on them after that.
        match &graded.pixels {
            Pixels::Rgb(_) => {}
            Pixels::Planar(_, grade) | Pixels::Packed(_, grade) => assert!(
                grade.as_ref().is_some_and(|grade| grade.is_gpu_grade()),
                "a plane picture went to the shader with no table on it"
            ),
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            Pixels::Surface(frame) => assert!(
                frame
                    .grade
                    .as_ref()
                    .is_some_and(|grade| grade.is_gpu_grade())
            ),
        }
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

/// The other half of a grade. A 1D cube keeps the channels apart, so the thread
/// is handed three 256-entry tables and paints by indexing them. The tables the
/// curve-only test above walks come out of the colour conversion alone, and the
/// 3D cube that follows this one mixes channels and takes the float route, so
/// nothing until now had put a viewer's own cube in front of this branch. Two
/// frames of the synthetic stream put 253 of the 256 codes each table is
/// indexed by in front of it, over 3 072 channels, and every one of those
/// channels is held to what the cube's own text says by an independent linear
/// walk of its nine nodes.
#[test]
fn a_per_channel_cube_paints_the_shown_pictures_from_byte_tables() {
    // Nine nodes, each channel its own shape: red lifted off black, green opened
    // through the mids, blue turned end for end. A table sent to the wrong
    // channel cannot survive that, and neither can one that never got built.
    const ONE: &str = "LUT_1D_SIZE 9
0.00 0.00 1.00
0.10 0.18 0.87
0.20 0.36 0.75
0.32 0.53 0.62
0.45 0.68 0.50
0.58 0.80 0.37
0.72 0.89 0.25
0.86 0.95 0.12
1.00 1.00 0.00
";
    let nodes: Vec<[f64; 3]> = ONE
        .lines()
        .filter(|line| {
            line.as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
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
    assert_eq!(nodes.len(), 9);
    let lookup = |code: f64, channel: usize| -> f64 {
        let pos = code.clamp(0.0, 1.0) * 8.0;
        let low = pos.floor() as usize;
        let high = (low + 1).min(8);
        let frac = pos - f64::from(low as u32);
        nodes[low][channel] * (1.0 - frac) + nodes[high][channel] * frac
    };
    let signal = ColourDescription {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
    let settings = Settings::video(DisplayTarget::sdr(240.0));
    assert!(
        Grade::new(signal, &HdrMetadata::default(), settings, None).is_identity(),
        "these settings already move this stream on their own, so a cube would not be the only change"
    );
    let grade = Grade::new(
        signal,
        &HdrMetadata::default(),
        settings,
        Some(Lut::from_cube(ONE).expect("a written 1D cube is a cube")),
    );
    assert!(
        !grade.is_identity(),
        "a cube that inverts blue claims to change nothing"
    );
    let reader = graded_from(stream());
    let budget = reader.rgb_budget();
    let mut plain = Playback::start(graded_from(stream()), None);
    let mut shown = Playback::start(graded_from(stream()), Some(grade));
    let mut channels = 0usize;
    let mut walked = [0usize; 256];
    for frame in 0..2 {
        let untouched = packed(first_frame(&mut plain).pixels, budget);
        let graded = packed(first_frame(&mut shown).pixels, budget);
        assert_eq!(untouched.len(), graded.len());
        assert_ne!(untouched, graded, "the 1D cube reached no pixels");
        for (pixel, out) in untouched
            .as_chunks::<3>()
            .0
            .iter()
            .zip(graded.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let code = pixel[channel];
                walked[usize::from(code)] += 1;
                let want = (lookup(f64::from(code) / 255.0, channel) * 255.0).round() as i16;
                let step = (i16::from(out[channel]) - want).abs();
                assert!(
                    step <= 1,
                    "frame {frame} channel {channel}: {code} shown as {}, the cube's own text says {want}",
                    out[channel]
                );
                channels += 1;
            }
        }
    }
    let codes = walked.iter().filter(|hits| **hits > 0).count();
    assert!(codes >= 250, "only {codes} input codes met the tables");
    assert!(
        channels >= 3 * 1_000,
        "only {channels} channels were compared against the cube's text"
    );
}

/// A camera log on the thread that shows it. S-Log3 is decoded from Sony's own
/// published numbers, written out here rather than borrowed, and put through
/// BT.709's opto-electronic transfer from the standard's own constants, so the
/// picture the thread shows is held to arithmetic the standard states instead
/// of to the curve that produced it. The synthetic ramp stays above Sony's
/// black code and out of the toe, which the profile's own tests walk.
///
/// A grade is a baked grid, so an input between two nodes is legitimately a
/// step towards one of them: the comparison runs at the nodes only, where the
/// grid carries the curve exactly and one code is the whole rounding. A grid of
/// 86 puts a node on every third code, which leaves 1 032 of this ramp's
/// 3 072 channels to check over 63 distinct nodes. 33 of those 63 sit two
/// codes or more away from what sRGB's exponent makes of the same light,
/// sixteen apart at the widest, so a wrong output curve cannot hide in the
/// rounding.
#[test]
fn a_camera_log_unfolds_in_the_picture_the_thread_shows() {
    let slog3 = |signal: f64| -> f64 {
        let cv = signal * 1023.0;
        if cv > 171.210_294_7 {
            10f64.powf((cv - 420.0) / 261.5) * 0.19 - 0.01
        } else {
            (cv - 95.0) * 0.011_25 / (171.210_294_7 - 95.0)
        }
    };
    let bt709 = |light: f64| -> f64 {
        if light <= 0.018 {
            4.5 * light
        } else {
            1.099 * light.powf(0.45) - 0.099
        }
    };
    let srgb = |light: f64| -> f64 {
        if light <= 0.003_130_8 {
            12.92 * light
        } else {
            1.055 * light.powf(1.0 / 2.4) - 0.055
        }
    };
    // 32x16, 4:2:0, neutral chroma, two frames of a luma ramp between Sony's
    // black code and the top of legal range.
    fn log_stream() -> Vec<u8> {
        let (width, height) = (32usize, 16usize);
        let mut bytes = b"YUV4MPEG2 W32 H16 F60:1 Ip C420jpeg\n".to_vec();
        for frame in 0..2u8 {
            bytes.extend_from_slice(b"FRAME\n");
            for y in 0..height {
                for x in 0..width {
                    bytes.push(60 + (((x * 10 + y * 3) as u32 + u32::from(frame) * 5) % 161) as u8);
                }
            }
            for _ in 0..height / 2 {
                for _ in 0..width / 2 {
                    bytes.push(128);
                }
            }
            for _ in 0..height / 2 {
                for _ in 0..width / 2 {
                    bytes.push(128);
                }
            }
        }
        bytes
    }
    fn graded(bytes: Vec<u8>) -> NativeReader<Cursor<Vec<u8>>> {
        let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes)).unwrap();
        assert!(reader.read_frame().unwrap());
        reader
    }
    // The file names BT.709 as its own gamut, so the log curve is the only
    // thing the grade has to do: no primaries, no matrix, no tone map.
    let signal = ColourDescription {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
    let mut settings = Settings::video(DisplayTarget::sdr(240.0));
    settings.log = Some(Log::SLog3);
    settings.size = 86;
    let grade = Grade::new(signal, &HdrMetadata::default(), settings, None);
    assert!(
        !grade.is_identity(),
        "a log curve asked for in place of the file's own transfer claims to change nothing"
    );
    let budget = graded(log_stream()).rgb_budget();
    let mut plain = Playback::start(graded(log_stream()), None);
    let mut shown = Playback::start(graded(log_stream()), Some(grade));
    let mut checked = 0usize;
    let mut walked = 0usize;
    for frame in 0..2 {
        let untouched = packed(first_frame(&mut plain).pixels, budget);
        let unfolded = packed(first_frame(&mut shown).pixels, budget);
        assert_eq!(untouched.len(), unfolded.len());
        assert_ne!(untouched, unfolded, "the log reached no pixels");
        let mut walk: Vec<(u8, u8)> = Vec::new();
        for (pixel, out) in untouched
            .as_chunks::<3>()
            .0
            .iter()
            .zip(unfolded.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let input = pixel[channel];
                let code = f64::from(input) / 255.0;
                assert!(
                    code * 1023.0 > 171.210_294_7,
                    "the ramp reached S-Log3's linear toe at code {input}"
                );
                walk.push((input, out[channel]));
                walked += 1;
                // A code between two nodes is where the grid is allowed to step,
                // so only the nodes carry the standard's arithmetic exactly.
                if input % 3 != 0 {
                    continue;
                }
                let want709 = (bt709(slog3(code)) * 255.0).clamp(0.0, 255.0).round() as i16;
                let want_srgb = (srgb(slog3(code)) * 255.0).clamp(0.0, 255.0).round() as i16;
                let shown = i16::from(out[channel]);
                assert!(
                    (shown - want709).abs() <= 1,
                    "frame {frame} channel {channel}: node {input} shown as {shown}, BT.709's own arithmetic says {want709} and sRGB's says {want_srgb}",
                );
                checked += 1;
            }
        }
        for pair in walk.windows(2) {
            if pair[0].0 < pair[1].0 {
                assert!(
                    pair[0].1 <= pair[1].1,
                    "the log turned {} -> {} and {} -> {}, which is not one direction",
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1
                );
            }
        }
    }
    assert!(
        checked >= 1_000,
        "only {checked} of {walked} channels met a node of the grid"
    );
}
