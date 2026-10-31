//! A missed repaint must not clip the live clock to the stale picture endpoint.
use super::*;
use std::io::Cursor;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-late-clock")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn tick(p: &mut Player, ctx: &egui::Context) {
    let mut o = ctx.run_ui(Default::default(), |ui| p.present(ui.ctx()));
    o.textures_delta.clear();
}
fn open(stop: Option<Duration>, ctx: &egui::Context) -> Player {
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        bounds: PlayBounds { start: None, stop },
        ..Default::default()
    };
    p.play_index(0);
    let until = Instant::now() + Duration::from_secs(5);
    while p.presented.is_none() {
        assert!(Instant::now() < until);
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(&mut p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((0, 300, 1000)));
    while p.playback.as_ref().unwrap().filled() < 2
        || p.presentation_clock().unwrap() < Duration::from_millis(75)
    {
        assert!(
            Instant::now() < until,
            "native prefetch/initial clock failed"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    p
}
fn stall(p: &Player) -> Duration {
    let before = p.presentation_clock().unwrap();
    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(800));
    before + start.elapsed()
}
#[test]
fn exact_mp4_late_callback_source_pixels_seek_and_eof_control() {
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
    for (at, start) in [(850, 600), (0, 0), (1500, 1200)] {
        assert!(r.seek_raw(Duration::from_millis(at)).unwrap().is_some());
        assert_eq!(r.frame_interval(), Some((start, start + 300, 1000)));
    }
}
#[test]
fn silent_clock_advances_past_stale_picture_end_during_missed_refresh() {
    let ctx = egui::Context::default();
    let p = open(None, &ctx);
    let expected = stall(&p);
    assert_eq!(p.interval, Some((0, 300, 1000)));
    let actual = p.presentation_clock().unwrap();
    assert!(
        actual.abs_diff(expected) < Duration::from_millis(15),
        "missed refresh clipped live clock:{actual:?} expected{expected:?}"
    );
    assert!(actual > Duration::from_millis(800));
}
#[test]
fn late_callback_consumes_passed_stop_bound_without_presenting_future_picture() {
    let ctx = egui::Context::default();
    let bound = Duration::from_millis(450);
    let mut p = open(Some(bound), &ctx);
    let first = display_rgb(p.presented.as_ref().unwrap()).unwrap();
    let elapsed = stall(&p);
    assert!(elapsed > Duration::from_millis(800));
    tick(&mut p, &ctx);
    assert!(
        p.ended,
        "passed stop bound stalled:clock{:?} old{:?} queued{:?}",
        p.presentation_clock(),
        p.interval,
        p.queued.as_ref().and_then(|f| f.interval)
    );
    assert_eq!(p.interval, Some((0, 300, 1000)));
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), first);
    assert!(p.stop_time.is_none());
    assert!(!p.buffering && !p.seek_preview);
    assert_eq!(p.presentation_clock(), Some(bound));
    std::thread::sleep(Duration::from_millis(30));
    tick(&mut p, &ctx);
    assert_eq!(p.presentation_clock(), Some(bound));
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), first);
    p.seek_time(Duration::ZERO);
    let until = Instant::now() + Duration::from_secs(5);
    while p.seek_preview || p.buffering {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!p.ended);
    assert!(p.presentation_clock().unwrap() < Duration::from_millis(20));
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), first);
}
#[test]
fn pausing_and_resuming_overdue_picture_preserves_phase_at_new_rate() {
    let ctx = egui::Context::default();
    let mut p = open(None, &ctx);
    let expected = stall(&p);
    let at = p.presentation_clock().unwrap();
    assert!(
        at.abs_diff(expected) < Duration::from_millis(15),
        "overdue phase lost before pause:{at:?} expected{expected:?}"
    );
    p.toggle_pause();
    let held = p.presentation_clock().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    p.set_rate(500);
    tick(&mut p, &ctx);
    assert_eq!(p.presentation_clock().unwrap(), held);
    p.toggle_pause();
    assert!(
        p.presentation_clock().unwrap().abs_diff(held) < Duration::from_millis(15),
        "resume discarded overdue phase"
    );
    tick(&mut p, &ctx);
    assert!(p.error.is_none());
    let (start, end, scale) = p.interval.unwrap();
    let clock = p.presentation_clock().unwrap();
    assert!(SelfClock::in_interval(clock, start, end, scale));
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let rgb = reference
        .seek_raw(Duration::from_millis(start as u64))
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
    p.seek_time(Duration::from_millis(1500));
    let until = Instant::now() + Duration::from_secs(5);
    while !p.ended {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
    std::thread::sleep(Duration::from_millis(30));
    tick(&mut p, &ctx);
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
}
struct SelfClock;
impl SelfClock {
    fn in_interval(clock: Duration, start: u128, end: u128, scale: u32) -> bool {
        let a = Player::interval_time(start, scale).unwrap();
        let b = Player::interval_time(end, scale).unwrap();
        clock + Duration::from_millis(5) >= a && clock <= b + Duration::from_millis(5)
    }
}

#[test]
fn manual_picture_navigation_leaves_zero_stop_before_first_picture() {
    let ctx = egui::Context::default();
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        bounds: PlayBounds {
            start: None,
            stop: Some(Duration::ZERO),
        },
        ..Default::default()
    };
    p.play_index(0);
    tick(&mut p, &ctx);
    assert!(p.ended);
    assert!(p.presented.is_none());
    p.apply(Control::Frame(false));
    let until = Instant::now() + Duration::from_secs(5);
    while p.presented.is_none() || p.step != 0 || p.buffering {
        assert!(Instant::now() < until);
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((0, 300, 1000)));
    assert!(p.paused);
    p.apply(Control::Frame(true));
    while p.step != 0 || p.buffering {
        assert!(Instant::now() < until);
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((300, 600, 1000)));
    assert!(p.paused);
    assert_eq!(
        p.presentation_clock(),
        Some(Duration::from_millis(300)),
        "stale terminal clock survived explicit picture navigation"
    );
}
