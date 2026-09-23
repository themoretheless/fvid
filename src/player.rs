//! Window presentation for FVid's own video reader.
//!
//! The chrome follows the minimalist player design: a near-black window, the
//! picture in a rounded dark frame, a title block top-left, one large play
//! button in the middle while paused, and a thin progress line with round
//! controls along the bottom. Controls fade out while the video plays and
//! the pointer rests; any movement brings them back.
use crate::{
    playback_native::{NativeReader, Planar8},
    playback_thread::{Event, Frame, Pixels, Playback},
    player_gpu::VideoCallback,
};
use eframe::egui_wgpu;
use std::sync::Arc;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::{
    fs::File,
    future::Future,
    io::BufReader,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

type Dialog = Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>;

/// Open an empty player or a supported local Y4M, MP4/AVC or WebM/VP9/AV1 file.
pub fn run(path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Player::default();
    if let Some(path) = path {
        app.open(path)?;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_title("FVid")
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_fullsize_content_view(true)
            .with_drag_and_drop(true),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "FVid",
        options,
        Box::new(move |cc| {
            if let Some(state) = cc.wgpu_render_state.as_ref() {
                crate::player_gpu::install(state);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

// Palette from the design canvas. Every overlay element sits at 50% opacity
// over the picture; only the window and frame backgrounds are opaque.
const WINDOW: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x10);
const FRAME: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1d);
const TEXT: Color32 = Color32::from_rgba_premultiplied(0x7a, 0x7a, 0x79, 128);
const MUTED: Color32 = Color32::from_rgba_premultiplied(0x51, 0x51, 0x53, 128);
const DIM: Color32 = Color32::from_rgba_premultiplied(0x3e, 0x3e, 0x41, 128);
const ACCENT: Color32 = Color32::from_rgba_premultiplied(0x74, 0x71, 0x6b, 128);
const ERROR: Color32 = Color32::from_rgba_premultiplied(0x70, 0x45, 0x3d, 128);
const TRACK: Color32 = Color32::from_rgba_premultiplied(23, 23, 23, 23);
const CHIP: Color32 = Color32::from_rgba_premultiplied(8, 8, 8, 8);
const CHIP_STRONG: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);

const BUTTON: f32 = 44.0;
const HIDE_AFTER: Duration = Duration::from_millis(2500);

/// How far a frame may lead the audio clock before presentation waits. The
/// audio clock only advances in whole device buffers, and ITU-R BT.1359 puts
/// the perceptibility of audio lagging picture at roughly ten frames, so this
/// stays above the stutter a tighter gate would cause at every rate.
pub const AUDIO_SYNC_SLACK: f64 = 0.05;

struct Player {
    /// The decoding thread for the open file.
    playback: Option<Playback>,
    /// Audio playback thread for A/V sync (when audio track is present).
    audio: Option<crate::audio_thread::AudioPlayback>,
    /// Audio reached its end or failed, so it no longer gates presentation.
    audio_ended: bool,
    playback_activity: Option<fvid_platform::PlaybackActivity>,
    duration: Option<Duration>,
    seekable: bool,
    hardware: bool,
    /// Geometry, period and timeline interval of the frame on screen.
    dimensions: [usize; 2],
    period: Duration,
    interval: Option<(u128, u128, u32)>,
    /// The next decoded frame, waiting for its presentation deadline.
    queued: Option<Frame>,
    buffering: bool,
    seek_preview: bool,
    seek_target: Option<Duration>,
    /// Frame on screen when it is drawn by the GPU shader (planar).
    video: Option<(Arc<Planar8>, u64)>,
    /// Frame on screen when it arrived as packed RGB (Y4M, WebM).
    texture: Option<egui::TextureHandle>,
    name: String,
    error: Option<String>,
    paused: bool,
    ended: bool,
    deadline: Instant,
    presentation_stats: Option<(Instant, u32)>,
    starved_polls: u32,
    /// Last pointer movement or click; drives the controls fade-out.
    activity: Instant,
    /// An open file picker, polled once per frame so the event loop never nests.
    dialog: Option<Dialog>,
    /// Fraction of the timeline under a drag on the progress line, shown until release.
    scrub: Option<f32>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            playback: None,
            audio: None,
            audio_ended: false,
            playback_activity: None,
            duration: None,
            seekable: false,
            hardware: false,
            dimensions: [0; 2],
            period: Duration::ZERO,
            interval: None,
            queued: None,
            buffering: true,
            seek_preview: false,
            seek_target: None,
            video: None,
            texture: None,
            name: String::new(),
            error: None,
            paused: false,
            ended: false,
            deadline: Instant::now(),
            presentation_stats: std::env::var_os("FVID_PLAYER_STATS").map(|_| (Instant::now(), 0)),
            starved_polls: 0,
            activity: Instant::now(),
            dialog: None,
            scrub: None,
        }
    }
}

impl Player {
    fn open(&mut self, path: PathBuf) -> crate::Result<()> {
        let mut reader = NativeReader::without_memory_limit(BufReader::new(File::open(&path)?))?;
        if !reader.read_frame()? {
            return Err(crate::invalid("video has no frames"));
        }
        if let Ok(seconds) = std::env::var("FVID_PLAYER_START_SECONDS") {
            let seconds = seconds
                .parse::<f64>()
                .map_err(|_| crate::invalid("invalid player start time"))?;
            let target = Duration::try_from_secs_f64(seconds)
                .map_err(|_| crate::invalid("invalid player start time"))?;
            reader.seek(target)?;
        }
        self.duration = reader.duration();
        if self.presentation_stats.is_some() {
            eprintln!("player timeline: duration={:?}", self.duration);
        }
        self.seekable = reader.seekable();
        self.hardware = reader.hardware_accelerated();
        self.dimensions = reader.dimensions();
        self.period = reader.frame_period();
        self.interval = None;
        // Drop the old thread before starting the new one.
        self.playback = None;
        self.audio = None;
        self.audio_ended = false;
        self.queued = None;
        self.buffering = true;
        self.seek_preview = false;
        self.seek_target = None;
        self.playback = Some(Playback::start(reader));
        // Try to start audio playback if the file has an audio track.
        self.try_start_audio(&path);
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
        self.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.error = None;
        self.paused = false;
        self.ended = false;
        Ok(())
    }

    /// Open the file's audio track, if any, and start decoding it on its own
    /// thread. Both containers are probed because the video reader accepts
    /// either and reports no codec information of its own.
    fn try_start_audio(&mut self, path: &PathBuf) {
        let mp4 = File::open(path).ok().and_then(|file| {
            crate::playback_mp4_audio::Mp4AudioReader::open(
                BufReader::new(file),
                crate::container::mp4::Limits::default(),
            )
            .ok()
            .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
        });
        let stream = mp4.or_else(|| {
            File::open(path)
                .ok()
                .and_then(|file| {
                    crate::playback_webm_audio::WebmAudioReader::open(
                        BufReader::new(file),
                        crate::container::webm::Limits::default(),
                    )
                    .ok()
                })
                .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
        });
        let Some(stream) = stream else {
            return;
        };
        self.audio = Some(crate::audio_thread::AudioPlayback::start(
            stream,
            || Box::new(crate::audio::PlatformBackend::new()),
        ));
    }

    fn try_open(&mut self, path: PathBuf) {
        if let Err(error) = self.open(path.clone()) {
            let message = format!("{}: {error}", path.display());
            eprintln!("{message}");
            self.error = Some(message);
        }
    }

    fn pick_file(&mut self) {
        if self.dialog.is_some() {
            return;
        }
        let dialog = rfd::AsyncFileDialog::new()
            .add_filter("Video", &["y4m", "mp4", "mov", "webm", "mkv"])
            .pick_file();
        self.dialog = Some(Box::pin(dialog));
    }

    fn poll_dialog(&mut self) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mut cx = Context::from_waker(Waker::noop());
        if let Poll::Ready(handle) = dialog.as_mut().poll(&mut cx) {
            self.dialog = None;
            if let Some(handle) = handle {
                self.try_open(handle.path().to_path_buf());
            }
        }
    }

    fn toggle_pause(&mut self) {
        let Some(playback) = &self.playback else {
            return;
        };
        if self.ended {
            self.restart();
            return;
        }
        self.paused = !self.paused;
        if self.paused {
            playback.pause();
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        } else {
            playback.play();
            if let Some(audio) = &self.audio {
                audio.play();
            }
            self.deadline = Instant::now();
            if let Some((_, count)) = &mut self.presentation_stats {
                *count = 0;
            }
        }
    }

    fn restart(&mut self) {
        if let Some(playback) = &mut self.playback {
            playback.rewind();
            if let Some(audio) = &mut self.audio {
                audio.rewind();
            }
            self.audio_ended = false;
            self.queued = None;
            self.buffering = true;
            self.seek_preview = false;
            self.seek_target = None;
            self.ended = false;
            self.paused = false;
            self.error = None;
            self.deadline = Instant::now();
            if let Some((_, count)) = &mut self.presentation_stats {
                *count = 0;
            }
        }
    }

    /// Jump to `fraction` of the known duration; playback state is kept.
    fn seek_fraction(&mut self, fraction: f32) {
        let (Some(playback), Some(total)) = (&mut self.playback, self.duration) else {
            return;
        };
        let target = total.mul_f32(fraction.clamp(0.0, 1.0));
        playback.seek(target);
        if let Some(audio) = &mut self.audio {
            audio.seek(target);
        }
        self.audio_ended = false;
        self.seek_target = Some(target);
        self.seek_preview = true;
        self.queued = None;
        self.buffering = true;
        self.ended = false;
        self.error = None;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
    }

    /// Check if a frame is ready for presentation based on A/V sync.
    /// Returns true if the frame can be shown now (audio clock has reached frame PTS).
    fn frame_ready_for_sync(&self, frame: &Frame) -> bool {
        let Some(audio) = &self.audio else {
            return true;
        };
        if self.audio_ended {
            return true;
        }
        let Some((pts, timescale)) = frame.pts else {
            return true;
        };
        if timescale == 0 {
            return true;
        }
        let audio_pos = audio.position();
        let frame_seconds = pts as f64 / timescale as f64;
        let audio_seconds = audio_pos.as_secs_f64();
        audio_seconds >= frame_seconds - AUDIO_SYNC_SLACK
    }

    /// Drain audio-thread events. Audio that has run out or failed must stop
    /// gating presentation, otherwise the picture freezes at the last clock.
    fn poll_audio(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        let generation = audio.generation();
        while let Some(event) = audio.poll() {
            match event {
                crate::audio_thread::AudioEvent::Started => {}
                crate::audio_thread::AudioEvent::Ended(at) => {
                    if at >= generation {
                        self.audio_ended = true;
                    }
                }
                crate::audio_thread::AudioEvent::Error(error) => {
                    self.audio_ended = true;
                    eprintln!("audio: {error}");
                }
            }
        }
    }

    /// Take decoded frames from the thread and show the one whose time has come.
    fn present(&mut self, ctx: &egui::Context) {
        self.poll_audio();
        if self.paused && !self.seek_preview {
            return;
        }
        let Some(playback) = &self.playback else {
            return;
        };
        let generation = playback.generation();
        while self.queued.is_none() {
            match playback.poll() {
                Some(Event::Frame(frame)) if frame.generation < generation => continue,
                Some(Event::Frame(frame)) => self.queued = Some(frame),
                Some(Event::Ended(at)) => {
                    if at >= generation {
                        self.ended = true;
                    }
                }
                Some(Event::Error(error)) => {
                    self.error = Some(error);
                    self.seek_target = None;
                    self.ended = true;
                }
                None => break,
            }
        }
        let now = Instant::now();
        if self.buffering
            && let Some(frame) = &self.queued
        {
            self.deadline = if self.paused {
                now
            } else if self.seek_preview {
                now + frame.period.min(Duration::from_millis(33))
            } else {
                now + crate::playback_thread::startup_buffer(frame.period)
            };
            self.buffering = false;
            // Sound and picture have to start from the same instant: the audio
            // thread was held paused across the preroll, so this is the first
            // moment its clock can run. Starting it any earlier leaves the
            // picture behind by the preroll for the whole file, and the sync
            // gate below can only ever hold video back, never hand it time.
            if !self.paused
                && let Some(audio) = &self.audio
            {
                audio.play();
            }
        }
        if self.queued.is_some() {
            // Clock and vsync jitter must not postpone a ready frame by an
            // entire refresh. The media clock still advances by the exact PTS interval.
            let tolerance = (self.period / 8).min(Duration::from_millis(1));
            let frame = self.queued.as_ref().unwrap();
            let time_ready = now + tolerance >= self.deadline;
            let av_ready = self.frame_ready_for_sync(frame);
            if time_ready && av_ready {
                let frame = self.queued.take().unwrap();
                self.seek_preview = false;
                self.seek_target = None;
                match &frame.pixels {
                    Pixels::Planar(planes) => {
                        self.video = Some((planes.clone(), frame.serial));
                        self.texture = None;
                    }
                    Pixels::Rgb(rgb) => {
                        let image = egui::ColorImage::from_rgb(frame.dimensions, rgb);
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(ctx.load_texture(
                                "video",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        self.video = None;
                    }
                }
                if let Some((start, count)) = &mut self.presentation_stats {
                    if *count == 0 {
                        *start = Instant::now();
                        self.starved_polls = 0;
                    }
                    *count += 1;
                    if *count == 301 {
                        eprintln!(
                            "player presentation: 300 intervals in {:?}, {:.2} fps, visible={:?}, focused={:?}, starved_polls={}",
                            start.elapsed(),
                            300.0 / start.elapsed().as_secs_f64(),
                            ctx.input(|i| i.viewport().visible()),
                            ctx.input(|i| i.viewport().focused),
                            self.starved_polls
                        );
                        self.starved_polls = 0;
                        *count = 1;
                        *start = Instant::now();
                    }
                }
                self.dimensions = frame.dimensions;
                self.period = frame.period;
                self.interval = frame.interval;
                // Keep the cadence while the decoder keeps up; when it falls
                // behind, show frames as they arrive instead of piling up debt.
                self.deadline = if self.deadline + frame.period < now {
                    now
                } else {
                    self.deadline + frame.period
                };
                ctx.request_repaint_after(self.deadline.saturating_duration_since(Instant::now()));
            } else {
                ctx.request_repaint_after(self.deadline.saturating_duration_since(now));
            }
        } else if (!self.paused || self.seek_preview) && !self.ended {
            if now >= self.deadline && self.presentation_stats.is_some() {
                self.starved_polls = self.starved_polls.saturating_add(1);
            }
            // Waiting on the decoder: check again soon.
            ctx.request_repaint_after(Duration::from_millis(8));
        }
        if !self.paused && !self.ended {
            // Let vsync pace rendering; a timer at the presentation deadline
            // can wake too late to submit that frame for the next refresh.
            ctx.request_repaint();
        }
    }

    /// Elapsed time at the end of the frame on screen, and the total when known.
    fn timeline(&self) -> (Option<Duration>, Option<Duration>) {
        if self.playback.is_none() {
            return (None, None);
        }
        if let Some(target) = self.seek_target {
            return (Some(target), self.duration);
        }
        let elapsed = self
            .interval
            .filter(|(_, _, scale)| *scale > 0)
            .map(|(_, end, scale)| {
                let nanos = end * 1_000_000_000 / u128::from(scale);
                Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
            });
        (elapsed, self.duration)
    }

    fn subtitle(&self) -> String {
        if self.playback.is_none() {
            return String::new();
        }
        let [w, h] = self.dimensions;
        let period = self.period.as_secs_f64();
        let fps = if period > 0.0 { 1.0 / period } else { 0.0 };
        let fps = if (fps - fps.round()).abs() < 0.05 {
            format!("{}", fps.round() as u32)
        } else {
            format!("{fps:.2}")
        };
        let decoder = if self.hardware { " · VideoToolbox" } else { "" };
        format!("{w}×{h} · {fps} fps{decoder}")
    }

    fn controls_visible(&self) -> bool {
        self.playback.is_none()
            || self.paused
            || self.ended
            || self.error.is_some()
            || self.activity.elapsed() < HIDE_AFTER
    }
}

fn clock(time: Duration) -> String {
    let total = time.as_secs();
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A round hit area with an icon painted by `draw`; returns whether it was clicked.
fn round_button(
    ui: &mut egui::Ui,
    id: &str,
    center: Pos2,
    size: f32,
    background: Color32,
    draw: impl FnOnce(&egui::Painter, Pos2, Color32),
) -> bool {
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() {
        CHIP_STRONG
    } else {
        background
    };
    if fill != Color32::TRANSPARENT {
        painter.circle_filled(center, size / 2.0, fill);
    }
    draw(painter, center, TEXT);
    response.clicked()
}

fn icon_play(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let points = vec![
        Pos2::new(c.x - s * 0.36, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y),
        Pos2::new(c.x - s * 0.36, c.y + s * 0.5),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn icon_pause(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let bar = Vec2::new(s * 0.3, s);
    for dx in [-s * 0.3, s * 0.3] {
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(c.x + dx, c.y), bar),
            CornerRadius::same(2),
            color,
        );
    }
}

fn icon_restart(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let left = c.x - s * 0.45;
    painter.line_segment(
        [Pos2::new(left, c.y - s * 0.5), Pos2::new(left, c.y + s * 0.5)],
        stroke,
    );
    let points = vec![
        Pos2::new(c.x + s * 0.5, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y + s * 0.5),
        Pos2::new(left + 4.0, c.y),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn icon_fullscreen(painter: &egui::Painter, c: Pos2, s: f32, color: Color32, exit: bool) {
    let stroke = Stroke::new(2.0, color);
    let h = s / 2.0;
    let arm = s * 0.3;
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = Pos2::new(c.x + sx * h, c.y + sy * h);
        let (a, b) = if exit {
            let inner = Pos2::new(c.x + sx * (h - arm), c.y + sy * (h - arm));
            (
                [inner, Pos2::new(inner.x, corner.y)],
                [inner, Pos2::new(corner.x, inner.y)],
            )
        } else {
            (
                [corner, Pos2::new(corner.x, corner.y - sy * arm)],
                [corner, Pos2::new(corner.x - sx * arm, corner.y)],
            )
        };
        painter.line_segment(a, stroke);
        painter.line_segment(b, stroke);
    }
}

fn icon_close(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let h = s / 2.0;
    painter.line_segment(
        [Pos2::new(c.x - h, c.y - h), Pos2::new(c.x + h, c.y + h)],
        stroke,
    );
    painter.line_segment(
        [Pos2::new(c.x - h, c.y + h), Pos2::new(c.x + h, c.y - h)],
        stroke,
    );
}

impl eframe::App for Player {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_pause();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.command) {
            self.pick_file();
        }
        if ctx.input(|i| i.pointer.is_moving() || i.pointer.any_pressed() || i.pointer.any_released())
        {
            self.activity = Instant::now();
        }
        if let Some(path) =
            ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()))
        {
            self.try_open(path);
        }
        if self.dialog.is_some() {
            self.poll_dialog();
            if self.dialog.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        self.present(ctx);
        if self.playback.is_some() && !self.paused && !self.ended {
            self.playback_activity.get_or_insert_with(fvid_platform::PlaybackActivity::new);
        } else {
            self.playback_activity = None;
        }
        // Wake up once to let the controls fade after the pointer rests.
        if self.controls_visible() && !self.paused {
            ctx.request_repaint_after(HIDE_AFTER.saturating_sub(self.activity.elapsed()));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let panel = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(WINDOW));
        panel.show(ui, |ui| {
            let frame = ui.max_rect();
            let painter = ui.painter().with_clip_rect(frame);
            painter.rect_filled(frame, CornerRadius::ZERO, FRAME);

            // The picture, aspect-fitted inside the frame. Planar frames are
            // converted to RGB by the GPU shader while drawing.
            if let Some((planes, serial)) = &self.video {
                let size = Vec2::new(planes.width as f32, planes.height as f32);
                let scale = (frame.width() / size.x)
                    .min(frame.height() / size.y)
                    .max(0.0);
                let rect = Rect::from_center_size(frame.center(), size * scale);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    VideoCallback {
                        frame: planes.clone(),
                        serial: *serial,
                    },
                ));
            } else if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let scale = (frame.width() / size.x)
                    .min(frame.height() / size.y)
                    .max(0.0);
                let rect = Rect::from_center_size(frame.center(), size * scale);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            // Clicking the picture toggles playback; the buttons below take priority
            // because they are interacted with later in the same frame.
            let surface = ui.interact(frame, ui.id().with("surface"), Sense::click());
            if surface.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            } else if surface.clicked() {
                if self.playback.is_some() {
                    self.toggle_pause();
                } else {
                    self.pick_file();
                }
            }

            let visible = self.controls_visible();
            if visible {
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            } else {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }

            // Centre: one large play button while idle.
            if self.playback.is_none() || self.paused || self.ended {
                let center = frame.center();
                let size = 88.0_f32.min(frame.height() * 0.4);
                let clicked = round_button(ui, "big-play", center, size, CHIP, |p, c, color| {
                    icon_play(p, Pos2::new(c.x + size * 0.04, c.y), size * 0.34, color);
                });
                if clicked {
                    if self.playback.is_some() {
                        self.toggle_pause();
                    } else {
                        self.pick_file();
                    }
                }
                if self.playback.is_none() {
                    painter.text(
                        Pos2::new(center.x, center.y + size / 2.0 + 24.0),
                        Align2::CENTER_TOP,
                        "Open or drop a Y4M, MP4 or WebM video",
                        FontId::proportional(13.0),
                        MUTED,
                    );
                }
            }

            if !visible {
                return;
            }

            let pad = 28.0_f32.min(frame.width() * 0.05);

            // Top: title block and close.
            // Keep the title block clear of the macOS traffic lights over the hidden title bar.
            let title_left = if fullscreen { pad } else { pad.max(96.0) };
            let top = Pos2::new(frame.left() + title_left, frame.top() + 24.0);
            if self.playback.is_some() {
                painter.text(top, Align2::LEFT_TOP, &self.name, FontId::proportional(17.0), TEXT);
                painter.text(
                    Pos2::new(top.x, top.y + 24.0),
                    Align2::LEFT_TOP,
                    self.subtitle(),
                    FontId::proportional(13.0),
                    MUTED,
                );
            } else {
                painter.text(top, Align2::LEFT_TOP, "FVid", FontId::proportional(17.0), TEXT);
            }
            if let Some(error) = &self.error {
                painter.text(
                    Pos2::new(top.x, top.y + 48.0),
                    Align2::LEFT_TOP,
                    error,
                    FontId::proportional(13.0),
                    ERROR,
                );
            }
            let close_at = Pos2::new(frame.right() - pad - BUTTON / 2.0, frame.top() + 24.0 + BUTTON / 2.0);
            if round_button(ui, "close", close_at, BUTTON, CHIP, |p, c, color| {
                icon_close(p, c, 12.0, color);
            }) {
                if fullscreen {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }

            // Bottom: progress line, then the control row.
            let row_y = frame.bottom() - 22.0 - BUTTON / 2.0;
            let bar_y = row_y - BUTTON / 2.0 - 14.0;
            let bar = Rect::from_min_max(
                Pos2::new(frame.left() + pad, bar_y - 2.0),
                Pos2::new(frame.right() - pad, bar_y + 2.0),
            );
            painter.rect_filled(bar, CornerRadius::same(2), TRACK);
            let (elapsed, total) = self.timeline();
            let mut fraction = match (elapsed, total) {
                (Some(e), Some(t)) if t > Duration::ZERO => {
                    Some(if self.ended && self.error.is_none() {
                        1.0
                    } else {
                        (e.as_secs_f32() / t.as_secs_f32()).clamp(0.0, 1.0)
                    })
                }
                _ => None,
            };
            // The line takes clicks and drags on a taller hit area; the seek
            // itself happens on release so a drag decodes only once.
            if fraction.is_some() && self.seekable {
                let hit = Rect::from_min_max(
                    Pos2::new(bar.left(), bar_y - 12.0),
                    Pos2::new(bar.right(), bar_y + 12.0),
                );
                let seek = ui.interact(hit, ui.id().with("seek"), Sense::click_and_drag());
                let at = |pos: Pos2| ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                if seek.dragged() {
                    if let Some(pos) = seek.interact_pointer_pos() {
                        self.scrub = Some(at(pos));
                    }
                }
                if seek.drag_stopped() || seek.clicked() {
                    if let Some(pos) = seek.interact_pointer_pos() {
                        let target = at(pos);
                        self.scrub = None;
                        self.seek_fraction(target);
                        fraction = Some(target);
                    }
                }
                if seek.hovered() || self.scrub.is_some() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if let Some(scrub) = self.scrub {
                    fraction = Some(scrub);
                }
            }
            if let Some(fraction) = fraction {
                let x = bar.left() + bar.width() * fraction;
                painter.rect_filled(
                    Rect::from_min_max(bar.min, Pos2::new(x, bar.max.y)),
                    CornerRadius::same(2),
                    ACCENT,
                );
                painter.circle_filled(Pos2::new(x, bar_y), 7.0, TEXT);
            }

            // Left group: play/pause, restart, time.
            let mut x = frame.left() + pad + BUTTON / 2.0;
            let playing = self.playback.is_some() && !self.paused && !self.ended;
            let clicked = round_button(ui, "play", Pos2::new(x, row_y), BUTTON, Color32::TRANSPARENT, |p, c, color| {
                if playing {
                    icon_pause(p, c, 16.0, color);
                } else {
                    icon_play(p, Pos2::new(c.x + 1.0, c.y), 16.0, color);
                }
            });
            if clicked {
                if self.playback.is_some() {
                    self.toggle_pause();
                } else {
                    self.pick_file();
                }
            }
            x += BUTTON + 4.0;
            if round_button(ui, "restart", Pos2::new(x, row_y), BUTTON, Color32::TRANSPARENT, |p, c, color| {
                icon_restart(p, c, 14.0, color);
            }) {
                self.restart();
            }
            x += BUTTON / 2.0 + 16.0;
            if let Some(elapsed) = elapsed {
                let font = FontId::monospace(13.0);
                let end = painter.text(Pos2::new(x, row_y), Align2::LEFT_CENTER, clock(elapsed), font.clone(), TEXT);
                if let Some(total) = total {
                    painter.text(
                        Pos2::new(end.right() + 6.0, row_y),
                        Align2::LEFT_CENTER,
                        format!("/ {}", clock(total)),
                        font,
                        DIM,
                    );
                }
            }

            // Right group: open, fullscreen.
            let mut x = frame.right() - pad - BUTTON / 2.0;
            if round_button(ui, "fullscreen", Pos2::new(x, row_y), BUTTON, Color32::TRANSPARENT, |p, c, color| {
                icon_fullscreen(p, c, 16.0, color, fullscreen);
            }) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }
            x -= BUTTON + 4.0;
            let label = "Open…";
            let font = FontId::proportional(13.0);
            let width = painter
                .layout_no_wrap(label.to_owned(), font.clone(), TEXT)
                .size()
                .x
                + 24.0;
            let open_rect = Rect::from_center_size(
                Pos2::new(x + BUTTON / 2.0 - width / 2.0, row_y),
                Vec2::new(width, BUTTON),
            );
            let open = ui.interact(open_rect, ui.id().with("open"), Sense::click());
            if open.hovered() {
                painter.rect_filled(open_rect, CornerRadius::same(22), CHIP_STRONG);
            }
            painter.text(open_rect.center(), Align2::CENTER_CENTER, label, font, TEXT);
            if open.clicked() {
                self.pick_file();
            }
        });
    }
}
