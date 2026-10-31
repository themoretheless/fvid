//! A live silent rate change preserves media phase within the current picture.
use super::*;
use std::io::Cursor;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-rate-clock")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn tick(p: &mut Player, ctx: &egui::Context) {
    let mut o = ctx.run_ui(Default::default(), |ui| p.present(ui.ctx()));
    o.textures_delta.clear();
}
fn wait(p: &mut Player, ctx: &egui::Context, pred: impl Fn(&Player) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !pred(p) {
        assert!(
            Instant::now() < until,
            "rate test stalled {:?} ended{}",
            p.interval,
            p.ended
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn exact_slow_mp4_pixels_intervals_seek_and_eof_control() {
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
    let first = r
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert!(r.seek_raw(Duration::from_millis(450)).unwrap().is_some());
    assert_eq!(r.frame_interval(), Some((300, 600, 1000)));
    assert_eq!(
        r.seek_raw(Duration::ZERO)
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        first
    );
    assert!(r.seek_raw(Duration::from_millis(1500)).unwrap().is_some());
    assert_eq!(r.frame_interval(), Some((1200, 1500, 1000)));
}
fn rate_transition(rate: u32) {
    let ctx = egui::Context::default();
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        ..Default::default()
    };
    p.play_index(0);
    wait(&mut p, &ctx, |p| p.presented.is_some());
    assert_eq!(p.interval, Some((0, 300, 1000)));
    let until = Instant::now() + Duration::from_secs(1);
    while p.presentation_clock().unwrap() < Duration::from_millis(75) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = p.presentation_clock().unwrap();
    let transition = Instant::now();
    p.set_rate(rate);
    let after = p.presentation_clock().unwrap();
    assert!(
        after.abs_diff(before) < Duration::from_millis(10),
        "rate{rate} jumped media clock from{before:?} to{after:?}"
    );
    let expected = paced_period(Duration::from_millis(300).saturating_sub(before), rate);
    while transition.elapsed() + Duration::from_millis(20) < expected {
        tick(&mut p, &ctx);
        assert_eq!(
            p.interval,
            Some((0, 300, 1000)),
            "rate{rate} spent current picture early"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    wait(&mut p, &ctx, |p| {
        p.interval.is_some_and(|(s, _, _)| s >= 300)
    });
    assert!(transition.elapsed() + Duration::from_millis(20) >= expected);
    assert_eq!(p.interval, Some((300, 600, 1000)));
    assert!(!p.ended);
    // Preserve the chosen rate through a real native seek and natural EOF.
    p.seek_time(Duration::from_millis(1250));
    wait(&mut p, &ctx, |p| !p.seek_preview && !p.buffering);
    assert_eq!(p.rate_milli, rate);
    assert_eq!(p.interval, Some((1200, 1500, 1000)));
    wait(&mut p, &ctx, |p| p.ended);
    assert!(p.error.is_none());
    assert!(!p.buffering);
}

#[test]
fn faster_live_rate_change_keeps_phase_and_real_seek_eof() {
    rate_transition(2000);
}
#[test]
fn slower_live_rate_change_keeps_phase_and_real_seek_eof() {
    rate_transition(500);
}
#[test]
fn silent_pause_resume_holds_phase_and_remaining_picture_even_with_paused_rate_change() {
    let ctx = egui::Context::default();
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: false,
        ..Default::default()
    };
    p.play_index(0);
    wait(&mut p, &ctx, |p| p.presented.is_some());
    let until = Instant::now() + Duration::from_secs(1);
    while p.presentation_clock().unwrap() < Duration::from_millis(75) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = p.presentation_clock().unwrap();
    p.toggle_pause();
    assert!(p.paused);
    let paused = p.presentation_clock().unwrap();
    assert!(
        paused.abs_diff(before) < Duration::from_millis(10),
        "pause rewound media phase from{before:?} to{paused:?}"
    );
    p.set_rate(500);
    std::thread::sleep(Duration::from_millis(30));
    tick(&mut p, &ctx);
    assert_eq!(p.presentation_clock().unwrap(), paused);
    assert_eq!(p.interval, Some((0, 300, 1000)));
    let resumed = Instant::now();
    p.toggle_pause();
    assert!(!p.paused);
    assert!(p.presentation_clock().unwrap().abs_diff(paused) < Duration::from_millis(10));
    let remaining = paced_period(Duration::from_millis(300) - paused, 500);
    while resumed.elapsed() + Duration::from_millis(20) < remaining {
        tick(&mut p, &ctx);
        assert_eq!(
            p.interval,
            Some((0, 300, 1000)),
            "resume spent the held picture early"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    wait(&mut p, &ctx, |p| {
        p.interval.is_some_and(|(s, _, _)| s >= 300)
    });
    assert_eq!(p.interval, Some((300, 600, 1000)));
    assert!(resumed.elapsed() + Duration::from_millis(20) >= remaining);
    p.seek_time(Duration::from_millis(1250));
    wait(&mut p, &ctx, |p| !p.buffering && !p.seek_preview);
    assert_eq!(p.interval, Some((1200, 1500, 1000)));
    wait(&mut p, &ctx, |p| p.ended);
    assert!(p.error.is_none());
}
