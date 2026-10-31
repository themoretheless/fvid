//! Relative navigation originates at the active clock,not a picture's future end.
use super::*;
use std::io::Cursor;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/player-jump-clock")
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
            "jump stalled {:?} {:?}",
            p.interval,
            p.seek_target
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn exact_mp4_long_picture_controls_seek_and_eof() {
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
            (i, i as i64 * 2000, 2000)
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
    for (at, start) in [(3300, 2000), (1300, 0), (10000, 8000)] {
        assert!(r.seek_raw(Duration::from_millis(at)).unwrap().is_some());
        assert_eq!(r.frame_interval(), Some((start, start + 2000, 1000)));
    }
}
fn jump_case(forward: bool, paused: bool) {
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
    let base = if forward { 0 } else { 4000 };
    if !forward {
        p.seek_time(Duration::from_millis(base));
        wait(&mut p, &ctx, |p| !p.seek_preview && !p.buffering);
        assert_eq!(p.interval, Some((4000, 6000, 1000)));
    }
    let until = Instant::now() + Duration::from_secs(1);
    while p.presentation_clock().unwrap() < Duration::from_millis(base + 300) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    if paused {
        p.toggle_pause();
        assert!(p.paused);
    }
    let clock = p.presentation_clock().unwrap();
    assert!(
        clock >= Duration::from_millis(base + 300) && clock < Duration::from_millis(base + 500),
        "bad control phase{clock:?}"
    );
    let delta = JUMP_VERY_SHORT;
    assert_eq!(delta, Duration::from_secs(3));
    let expected = if forward {
        clock + delta
    } else {
        clock - delta
    };
    p.apply(Control::Jump(delta, forward));
    let target = p.seek_target.unwrap();
    assert!(
        target.abs_diff(expected) < Duration::from_millis(15),
        "forward{forward} paused{paused} jump started at future picture end:clock{clock:?} expected{expected:?} target{target:?}"
    );
    wait(&mut p, &ctx, |p| !p.seek_preview && !p.buffering);
    assert_eq!(p.paused, paused);
    let start = if forward { 2000 } else { 0 };
    assert_eq!(p.interval, Some((start, start + 2000, 1000)));
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let rgb = reference
        .seek_raw(expected)
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
    assert!(p.error.is_none());
}
#[test]
fn forward_three_second_jump_uses_live_clock() {
    jump_case(true, false);
}
#[test]
fn backward_three_second_jump_uses_live_clock() {
    jump_case(false, false);
}
#[test]
fn paused_three_second_jump_uses_held_clock() {
    jump_case(true, true);
}

#[test]
fn relative_navigation_clamps_at_native_eof_and_zero_without_changing_pause() {
    let ctx = egui::Context::default();
    let mut p = Player {
        queue: vec![root().join("movie.mp4")],
        no_audio: true,
        asynchronous_open: false,
        start_paused: true,
        ..Default::default()
    };
    p.play_index(0);
    wait(&mut p, &ctx, |p| {
        p.presented.is_some() && !p.buffering && !p.seek_preview
    });
    let first = display_rgb(p.presented.as_ref().unwrap()).unwrap();
    p.apply(Control::Jump(JUMP_LONG, true));
    assert_eq!(p.seek_target, Some(Duration::from_secs(10)));
    wait(&mut p, &ctx, |p| !p.seek_preview && !p.buffering);
    assert_eq!(p.interval, Some((8000, 10000, 1000)));
    assert!(p.paused);
    assert!(p.error.is_none());
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let last = reference
        .seek_raw(Duration::from_secs(10))
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), last);
    p.apply(Control::Jump(JUMP_LONG, false));
    assert_eq!(p.seek_target, Some(Duration::ZERO));
    wait(&mut p, &ctx, |p| !p.seek_preview && !p.buffering);
    assert_eq!(p.interval, Some((0, 2000, 1000)));
    assert!(p.paused);
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), first);
    assert!(p.error.is_none());
}
