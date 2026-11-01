//! A real empty-queue read stall is distinct from a missed UI repaint.
use super::*;
use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/player-io-clock")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn tick(p: &mut Player, ctx: &egui::Context) {
    let mut out = ctx.run_ui(Default::default(), |ui| p.present(ui.ctx()));
    out.textures_delta.clear();
}
#[derive(Default)]
struct State {
    armed: bool,
    blocked: bool,
    released: bool,
}
#[derive(Default)]
struct Gate {
    state: Mutex<State>,
    wake: Condvar,
}
impl Gate {
    fn arm(&self) {
        self.state.lock().unwrap().armed = true;
    }
    fn blocked(&self) -> bool {
        self.state.lock().unwrap().blocked
    }
    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.wake.notify_all();
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct Input {
    cursor: Cursor<Vec<u8>>,
    gate: Arc<Gate>,
    range: std::ops::Range<u64>,
}
impl Input {
    fn wait(&self) -> io::Result<()> {
        if !self.range.contains(&self.cursor.position()) {
            return Ok(());
        }
        let mut s = self.gate.state.lock().unwrap();
        while s.armed && !s.released {
            s.blocked = true;
            self.gate.wake.notify_all();
            let (next, timeout) = self
                .gate
                .wake
                .wait_timeout(s, Duration::from_secs(10))
                .unwrap();
            s = next;
            if timeout.timed_out() && !s.released {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "test input gate was not released",
                ));
            }
        }
        Ok(())
    }
}
impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        self.wait()?;
        let n = out.len().min(1);
        self.cursor.read(&mut out[..n])
    }
}
impl BufRead for Input {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.wait()?;
        let n = (self.cursor.position() as usize).min(self.cursor.get_ref().len());
        let end = (n + 1).min(self.cursor.get_ref().len());
        Ok(&self.cursor.get_ref()[n..end])
    }
    fn consume(&mut self, n: usize) {
        self.cursor.consume(n);
    }
}
impl Seek for Input {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(from)
    }
}
fn open(stop: Option<Duration>) -> (Player, Arc<Gate>) {
    let file = bytes("movie.mp4");
    let parsed =
        crate::container::mp4::Mp4Reader::open(Cursor::new(file.clone()), Default::default())
            .unwrap();
    let sample = parsed.tracks()[0].samples.get(2).unwrap();
    let gate = Arc::new(Gate::default());
    let input = Input {
        cursor: Cursor::new(file),
        gate: gate.clone(),
        range: sample.offset..sample.offset + u64::from(sample.size),
    };
    let mut reader = NativeReader::software(input, 32 << 20).unwrap();
    let first = reader.read_frame_raw().unwrap().unwrap();
    assert_eq!(reader.frame_interval(), Some((0, 300, 1000)));
    gate.arm();
    let period = reader.frame_period();
    let signal = reader.colour();
    let hdr = reader.hdr();
    let playback = crate::playback_thread::Playback::start_from_frame(reader, first, None);
    let p = Player {
        playback: Some(playback),
        duration: Some(Duration::from_millis(1500)),
        seekable: true,
        no_audio: true,
        asynchronous_open: false,
        queue: vec![root().join("movie.mp4")],
        opened: Some(root().join("movie.mp4")),
        buffering: true,
        stop_time: stop,
        period,
        signal,
        hdr,
        ..Default::default()
    };
    (p, gate)
}
fn first(p: &mut Player, g: &Gate, ctx: &egui::Context) {
    let until = Instant::now() + Duration::from_secs(4);
    while p.presented.is_none() || !g.blocked() {
        assert!(
            Instant::now() < until,
            "next-picture lookahead read did not actually block"
        );
        assert!(p.error.is_none(), "{:?}", p.error);
        tick(p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((0, 300, 1000)));
    assert!(!p.buffering);
    assert_eq!(p.playback.as_ref().unwrap().filled(), 0);
    assert!(p.queued.is_none());
}
#[test]
fn exact_mp4_io_stall_source_pixels_pts_seek_and_eof_control() {
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

#[test]
fn real_read_stall_freezes_media_time_then_resumes_without_rewinding() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(None);
    let _release = Release(g.clone());
    first(&mut p, &g, &ctx);
    let until = Instant::now() + Duration::from_secs(3);
    let mut previous = p.presentation_clock().unwrap();
    while !p.buffering {
        assert!(
            Instant::now() < until,
            "actual input stall never became buffering"
        );
        assert!(g.blocked());
        assert!(!p.ended && p.error.is_none());
        tick(&mut p, &ctx);
        let current = p.presentation_clock().unwrap();
        assert!(
            current >= previous,
            "buffering rewound the media clock:{previous:?}->{current:?}"
        );
        previous = current;
        std::thread::sleep(Duration::from_millis(1));
    }
    let held = p.presentation_clock().unwrap();
    std::thread::sleep(Duration::from_millis(80));
    tick(&mut p, &ctx);
    assert!(p.buffering && g.blocked());
    assert_eq!(
        p.presentation_clock(),
        Some(held),
        "silent media clock advances while actual input is buffered"
    );
    g.release();
    let until = Instant::now() + Duration::from_secs(3);
    while p.buffering || p.interval.unwrap().0 == 0 {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        p.presentation_clock().unwrap() >= held,
        "input recovery rewound the held phase"
    );
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let at = Duration::from_millis(p.interval.unwrap().0 as u64);
    let rgb = reference
        .seek_raw(at)
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
    while !p.ended {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((1200, 1500, 1000)));
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
    assert!(!p.buffering);
}
#[test]
fn actual_missing_input_does_not_spend_a_later_media_stop_before_recovery() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(Some(Duration::from_millis(450)));
    let _release = Release(g.clone());
    first(&mut p, &g, &ctx);
    let until = Instant::now() + Duration::from_secs(3);
    while !p.buffering {
        assert!(Instant::now() < until);
        assert!(g.blocked());
        tick(&mut p, &ctx);
        assert!(
            !p.ended,
            "stop450ms consumed while the actual next-picture lookahead packet is blocked and only0..300ms exists:clock{:?}",
            p.presentation_clock()
        );
        assert!(p.error.is_none());
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(80));
    tick(&mut p, &ctx);
    assert!(!p.ended && p.stop_time.is_some());
    g.release();
    let until = Instant::now() + Duration::from_secs(3);
    while !p.ended {
        assert!(
            Instant::now() < until,
            "recovered input never reaches stop450ms"
        );
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(450)));
    assert_eq!(p.interval, Some((300, 600, 1000)));
    assert!(p.stop_time.is_none());
    assert!(!p.buffering);
}

fn reach_buffering(p: &mut Player, g: &Gate, ctx: &egui::Context) {
    first(p, g, ctx);
    let until = Instant::now() + Duration::from_secs(3);
    while !p.buffering {
        assert!(Instant::now() < until);
        assert!(g.blocked() && !p.ended && p.error.is_none());
        tick(p, ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(g.blocked());
    assert_eq!(p.playback.as_ref().unwrap().filled(), 0);
    assert!(p.queued.is_none());
}
fn recover(p: &mut Player, g: &Gate, ctx: &egui::Context, held: Duration) {
    let until = Instant::now() + Duration::from_secs(3);
    let mut previous = held;
    g.release();
    while p.buffering || p.interval.unwrap().0 == 0 {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(p, ctx);
        let current = p.presentation_clock().unwrap();
        assert!(
            current >= previous,
            "input recovery clock rewound before next picture:{previous:?}->{current:?},interval{:?},buffering{}",
            p.interval,
            p.buffering
        );
        previous = current;
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn input_recovery_clock_never_rewinds_before_the_next_picture_is_shown() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(None);
    let _release = Release(g.clone());
    reach_buffering(&mut p, &g, &ctx);
    let held = p.presentation_clock().unwrap();
    recover(&mut p, &g, &ctx, held);
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let at = Duration::from_millis(p.interval.unwrap().0 as u64);
    let rgb = reference
        .seek_raw(at)
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
}
#[test]
fn seek_while_input_waits_keeps_the_new_target_through_preroll_and_eof() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(None);
    let _release = Release(g.clone());
    reach_buffering(&mut p, &g, &ctx);
    p.seek_time(Duration::from_millis(1200));
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1200)));
    g.release();
    let until = Instant::now() + Duration::from_secs(3);
    while p.seek_preview || p.buffering {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((1200, 1500, 1000)));
    assert!(
        p.presentation_clock()
            .unwrap()
            .abs_diff(Duration::from_millis(1200))
            < Duration::from_millis(20)
    );
    let mut reference = NativeReader::software(Cursor::new(bytes("movie.mp4")), 32 << 20).unwrap();
    let rgb = reference
        .seek_raw(Duration::from_millis(1200))
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    assert_eq!(display_rgb(p.presented.as_ref().unwrap()).unwrap(), rgb);
    while !p.ended {
        assert!(Instant::now() < until);
        tick(&mut p, &ctx);
        assert!(p.error.is_none());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
}
#[test]
fn pause_and_rate_change_during_read_wait_keep_recovery_phase_and_remaining_hold() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(None);
    let _release = Release(g.clone());
    reach_buffering(&mut p, &g, &ctx);
    p.toggle_pause();
    let held = p.presentation_clock().unwrap();
    p.set_rate(500);
    std::thread::sleep(Duration::from_millis(80));
    tick(&mut p, &ctx);
    assert!(p.paused && p.buffering && g.blocked());
    assert_eq!(p.presentation_clock(), Some(held));
    p.toggle_pause();
    recover(&mut p, &g, &ctx, held);
    assert_eq!(p.interval, Some((300, 600, 1000)));
    let phase = p.presentation_clock().unwrap();
    assert!(phase >= held && phase < Duration::from_millis(600));
    let remaining = paced_period(Duration::from_millis(600) - phase, 500);
    let began = Instant::now();
    let until = Instant::now() + Duration::from_secs(4);
    while p.interval == Some((300, 600, 1000)) {
        assert!(Instant::now() < until);
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        began.elapsed() + Duration::from_millis(15) >= remaining,
        "recovered picture lost its remaining half-rate duration"
    );
    assert!(began.elapsed() <= remaining + Duration::from_millis(100));
    assert_eq!(p.interval, Some((600, 900, 1000)));
    p.seek_time(Duration::from_millis(1200));
    while !p.ended {
        assert!(Instant::now() < until);
        assert!(p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.presentation_clock(), Some(Duration::from_millis(1500)));
}

#[test]
fn short_input_wait_before_buffering_keeps_the_existing_wall_clock_catchup() {
    let ctx = egui::Context::default();
    let (mut p, g) = open(None);
    let _release = Release(g.clone());
    first(&mut p, &g, &ctx);
    let phase = p.presentation_clock().unwrap();
    let began = Instant::now();
    while phase + began.elapsed() < Duration::from_millis(400) {
        assert!(g.blocked() && !p.buffering && !p.ended);
        tick(&mut p, &ctx);
        assert!(p.error.is_none());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!p.buffering);
    g.release();
    let until = Instant::now() + Duration::from_secs(3);
    while p.interval.unwrap().0 == 0 {
        assert!(Instant::now() < until);
        assert!(!p.buffering && p.error.is_none());
        tick(&mut p, &ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(p.interval, Some((300, 600, 1000)));
    let expected = phase + began.elapsed();
    assert!(
        p.presentation_clock().unwrap().abs_diff(expected) < Duration::from_millis(15),
        "brief input delay lost wall-clock catchup:actual{:?} expected{expected:?}",
        p.presentation_clock()
    );
}
