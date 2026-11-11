//! Native chapter transport uses the live position.
use super::*;
use std::io::Cursor;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-chapter-clock")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn tick(p: &mut Player, ctx: &egui::Context) {
    let mut o = ctx.run_ui(Default::default(), |ui| p.present(ui.ctx()));
    o.textures_delta.clear();
}
fn generation(p: &Player) -> u64 {
    p.playback.as_ref().unwrap().generation()
}
fn open(ctx: &egui::Context) -> Player {
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        ..Default::default()
    };
    p.play_index(0);
    let until = Instant::now() + Duration::from_secs(5);
    while p.presented.is_none() || p.buffering || p.seek_preview {
        assert!(Instant::now() < until);
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(&mut p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((0, 300, 1000)));
    assert!(p.seekable);
    p
}
fn settle(p: &mut Player, ctx: &egui::Context, expected: u64) {
    let until = Instant::now() + Duration::from_secs(2);
    while (p.seek_preview || p.buffering) && generation(p) <= expected + 1 && Instant::now() < until
    {
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !p.seek_preview && !p.buffering,
        "chapter seek did not settle:g{} expected{expected} interval{:?} target{:?}",
        generation(p),
        p.interval,
        p.seek_target
    );
    assert_eq!(generation(p), expected);
    assert!(!p.ended);
}
#[test]
fn exact_mp4_chapter_source_metadata_pixels_seek_and_eof_control() {
    let movie =
        crate::container::mp4::Mp4Reader::open(Cursor::new(bytes("movie.mp4")), Default::default())
            .unwrap();
    let expected = vec![
        (0, "Opening"),
        (100, "First interior"),
        (200, "Second interior"),
        (600, "Later"),
        (1200, "Final"),
    ];
    assert_eq!(
        movie
            .chapters()
            .iter()
            .map(|c| (c.start_ns / 1_000_000, c.title.as_str()))
            .collect::<Vec<_>>(),
        expected
    );
    let owned =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes("movie.mp4")), Default::default())
            .unwrap();
    assert_eq!(
        owned
            .chapters()
            .iter()
            .map(|c| (c.start_ns / 1_000_000, c.title.as_str()))
            .collect::<Vec<_>>(),
        expected
    );
    let mut r = crate::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("movie.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..5 {
        let f = r.read_frame().unwrap().unwrap();
        assert_eq!(
            (f.sample_index, f.presentation_time.ticks, f.duration.ticks),
            (i, i as i64 * 300, 300)
        );
        let yuv: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(yuv, bytes(&format!("frame-{}.yuv16", i % 3)));
    }
    assert!(r.read_frame().unwrap().is_none());
    let mut r = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    for (at, start) in [(300, 300), (1200, 1200), (1500, 1200)] {
        assert!(r.seek_raw(Duration::from_millis(at)).unwrap().is_some());
        assert_eq!(r.frame_interval(), Some((start, start + 300, 1000)));
    }
}

fn chapter_case(at: u64, forward: bool, paused: bool, pending: bool, target: u64) {
    let ctx = egui::Context::default();
    let mut p = open(&ctx);
    assert_eq!(
        p.chapters
            .iter()
            .map(|c| c.start.as_millis())
            .collect::<Vec<_>>(),
        vec![0, 100, 200, 600, 1200]
    );
    p.seek_time(Duration::from_millis(at));
    let first = generation(&p);
    if !pending {
        settle(&mut p, &ctx, first);
    }
    if paused {
        p.toggle_pause();
        assert!(p.paused);
    }
    let origin = p.presentation_clock().unwrap();
    assert!(
        origin >= Duration::from_millis(at)
            && origin
                < Duration::from_millis(match at {
                    75 => 100,
                    175 => 200,
                    450 => 600,
                    _ => panic!("unexpected probe phase"),
                }),
        "transport probe left the intended chapter interval: {origin:?}"
    );
    p.apply(Control::Chapter(forward));
    assert_eq!(
        p.seek_target,
        Some(Duration::from_millis(target)),
        "chapter origin {origin:?},forward{forward},paused{paused},pending{pending}"
    );
    let requested = generation(&p);
    assert_eq!(requested, first + 1);
    settle(&mut p, &ctx, requested);
    assert_eq!(p.paused, paused);
    assert!(
        p.presentation_clock()
            .unwrap()
            .abs_diff(Duration::from_millis(target))
            < Duration::from_millis(20)
    );
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let rgb = reference
        .seek_raw(Duration::from_millis(target))
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
    assert_eq!(
        p.interval,
        Some((
            u128::from(target / 300 * 300),
            u128::from(target / 300 * 300 + 300),
            1000
        ))
    );
    // Native final chapter remains playable and true source EOF is reachable.
    p.seek_time(Duration::from_millis(1200));
    let last = generation(&p);
    settle(&mut p, &ctx, last);
    if paused {
        p.toggle_pause();
    }
    let until = Instant::now() + Duration::from_secs(3);
    while !p.ended {
        assert!(Instant::now() < until);
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(p.error.is_none());
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
}
#[test]
fn next_chapter_inside_current_picture_is_not_skipped() {
    chapter_case(75, true, false, false, 100);
}
#[test]
fn previous_chapter_does_not_move_forward_within_picture() {
    chapter_case(175, false, false, false, 100);
}
#[test]
fn paused_previous_chapter_uses_retained_media_phase() {
    chapter_case(175, false, true, false, 100);
}
#[test]
fn pending_seek_is_the_origin_of_the_next_chapter() {
    chapter_case(175, true, false, true, 200);
}
#[test]
fn pending_seek_is_the_origin_of_the_previous_chapter() {
    chapter_case(175, false, false, true, 100);
}

#[test]
fn next_chapter_at_next_picture_start_is_not_skipped() {
    chapter_case(450, true, false, false, 600);
}
