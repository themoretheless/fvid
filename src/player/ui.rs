use super::{
    Adjust, Player,
    captions::chapter_at,
    clock::{
        JUMP_LONG, buffered_fraction, clock, delay_step, end_clock, jump_size,
        position_from_digit, rate_fine, rate_osd,
    },
    playlist::{cycle_repeat, expand_inputs, repeat_osd},
    projection::{
        adjust_scalars, cropped_size, gpu_view_params, shown_insets, shown_size,
        stereo_dimensions, uv_window, video_rect,
    },
    skin as player_skin,
};
use crate::player_gpu::VideoCallback;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu;
use std::path::PathBuf;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

/// Which file the picker standing in front of the viewer was opened for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    /// A file to play, appended to the queue.
    Item,
    Folder,
    /// A file of cues to caption the item on screen.
    Subtitles,
}

// Palette from the design canvas: warm off-white type and one orange accent
// over the picture, with the control panel and the key messages on a dark
// backing at half opacity so the picture still shows through them.
pub(super) const WINDOW: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0a);
pub(super) const FRAME: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1d);
pub(super) const TEXT: Color32 = Color32::from_rgb(0xee, 0xeb, 0xe4);
pub(super) const MUTED: Color32 = Color32::from_rgb(0xa8, 0xa4, 0x9b);
pub(super) const DIM: Color32 = Color32::from_rgb(0x7a, 0x77, 0x70);
pub(super) const ERROR: Color32 = Color32::from_rgb(0xf0, 0x7a, 0x6a);
/// The dark backing of buttons.
pub(super) const CHIP: Color32 = Color32::from_rgba_premultiplied(6, 6, 6, 158);
pub(super) const CHIP_STRONG: Color32 = Color32::from_rgba_premultiplied(19, 19, 18, 20);
/// Corner of the control panel and the key messages.
pub(super) const PANEL_RADIUS: u8 = 14;

pub(super) const BUTTON: f32 = 44.0;
pub(super) const HIDE_AFTER: Duration = Duration::from_millis(2500);
/// How long a transient control message stays on screen.
pub(super) const OSD_AFTER: Duration = Duration::from_millis(1500);


/// Retargetable drawing-only motion; pointer targets and playback clocks stay exact.
pub(super) fn smooth_timeline_position(ctx: &egui::Context, id: egui::Id, target: f32) -> f32 {
    smooth_timeline_value(ctx, id, target, target, 0.080)
}

/// Exponential retargeting keeps the transition continuous when hover reverses.
pub(super) fn smooth_timeline_value(
    ctx: &egui::Context,
    id: egui::Id,
    target: f32,
    initial: f32,
    seconds: f32,
) -> f32 {
    let (now, first_dt) = ctx.input(|input| (input.time, input.predicted_dt));
    let value = ctx.data_mut(|data| {
        let (previous, last_time) = data
            .get_temp::<(f32, f64)>(id)
            .unwrap_or((initial, now - f64::from(first_dt)));
        // Use elapsed time rather than predicted frame time: hover and drag
        // transitions must run at the same speed on 60 Hz and 120 Hz displays.
        let elapsed = (now - last_time).max(0.0) as f32;
        // A paused player may not paint for seconds. That idle time predates
        // the new pointer target and must not fast-forward its transition.
        let dt = if elapsed > 0.100 {
            first_dt.clamp(1.0 / 240.0, 1.0 / 30.0)
        } else {
            elapsed
        };
        let next = previous + (target - previous) * (1.0 - (-dt / seconds).exp());
        let next = if (target - next).abs() < 0.0001 { target } else { next };
        data.insert_temp(id, (next, now));
        next
    });
    if value != target {
        ctx.request_repaint();
    }
    value
}

/// A round hit area with an icon painted by `draw`; returns whether it was clicked.
pub(super) fn round_button(
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

/// A crotched note: the stage of an item that is only sound.
pub(super) fn icon_note(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let head = s * 0.16;
    let stem = c.x + s * 0.22;
    painter.circle_filled(Pos2::new(stem - head * 1.6, c.y + s * 0.38), head, color);
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(stem, c.y - s * 0.42),
            Pos2::new(stem + s * 0.06, c.y + s * 0.38),
        ),
        CornerRadius::ZERO,
        color,
    );
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(stem + s * 0.06, c.y - s * 0.42),
            Pos2::new(stem + s * 0.3, c.y - s * 0.42 + s * 0.12),
        ),
        CornerRadius::ZERO,
        color,
    );
}

pub(super) fn icon_play(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let points = vec![
        Pos2::new(c.x - s * 0.36, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y),
        Pos2::new(c.x - s * 0.36, c.y + s * 0.5),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

pub(super) fn icon_pause(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let bar = Vec2::new(s * 0.3, s);
    for dx in [-s * 0.3, s * 0.3] {
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(c.x + dx, c.y), bar),
            CornerRadius::same(2),
            color,
        );
    }
}

pub(super) fn icon_restart(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let left = c.x - s * 0.45;
    painter.line_segment(
        [
            Pos2::new(left, c.y - s * 0.5),
            Pos2::new(left, c.y + s * 0.5),
        ],
        stroke,
    );
    let points = vec![
        Pos2::new(c.x + s * 0.5, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y + s * 0.5),
        Pos2::new(left + 4.0, c.y),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

pub(super) fn icon_fullscreen(painter: &egui::Painter, c: Pos2, s: f32, color: Color32, exit: bool) {
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


/// A control the keyboard asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Control {
    Spherical(bool),
    /// Up or down one volume step.
    Volume(bool),
    Mute,
    /// Next or previous VLC rate preset, or an absolute rate to return to.
    Rate(bool),
    /// VLC's `,` and `.`: nudge the rate by a tenth of its own name, inside
    /// the bounds the preset ladder reaches.
    RateFine(bool),
    Reset,
    /// A VLC jump size, forward or back.
    Jump(Duration, bool),
    Position(f32),
    /// One frame forward, or back to the frame before the one on screen.
    Frame(bool),
    Subtitles,
    /// Show the cues later (true) or earlier (false).
    SubtitleDelay(bool),
    /// Move the cue block up (true) or down (false).
    SubtitleMargin(bool),
    SubtitleSize(bool),
    /// Next or previous subtitle source of the open file.
    SubtitleTrack(bool),
    /// Next or previous item of the open list.
    Next,
    Previous,
    /// Cycle the end-of-item behaviour.
    Repeat,
    /// Turn VLC's Random on or off: the list walks in a drawn order.
    Shuffle,
    /// Press the A–B loop key: mark A, then B, then clear both.
    Loop,
    /// Let sound arrive later (true) or earlier (false).
    AudioDelay(bool),
    /// Next or previous audio track of the open file.
    AudioTrack(bool),
    /// Write the picture on screen out as a PNG file.
    Snapshot,
    /// Next (true) or previous chapter of the open file.
    Chapter(bool),
    /// Up or down one rung of VLC's Zoom menu.
    Zoom(bool),
    /// Forward or back along VLC's Crop menu.
    Crop(bool),
    /// Forward or back along VLC's aspect-ratio menu.
    Aspect(bool),
    /// Open or close the panel of what the open item is.
    Info,
    /// Name a file to caption the open item with, which VLC loads from its
    /// Media menu.
    LoadSubtitles,
    /// Open or close VLC's Ctrl+E dialog of picture settings.
    Effects,
}


/// Keys the player answers, mapped onto VLC's defaults: arrows jump, with
/// Shift, Alt and Control taking the longer ladder rungs; Control or Command
/// with up and down moves the volume; `m` mutes; `=` and `-` step through the
/// rate presets, `,` and `.` nudge the rate by a tenth between them; `\`
/// returns to normal speed; `e` advances one frame, or one
/// back with Shift; Home and End go to either end of the timeline; and a number
/// key jumps to that tenth of the file, and `n`, `p` and `r` walk the queue and
/// cycle what happens when an item ends, with `s` deciding whether that walk
/// follows a drawn order. `l` marks the A-B loop points, `j` and
/// `k` shift the audio clock, `a` walks the file's audio tracks, `b` its
/// subtitle tracks, and Shift+S writes the picture on screen out as a PNG.
/// `[` and `]` jump between the chapters the file names, and `z` and `Z` step
/// the picture along VLC's Zoom menu; a picture a magnification has grown past
/// the frame can then be dragged to the part of it the frame cannot show. `i`
/// opens the panel of what the item on screen is, and closes it again.
/// Consume view arrows before transport controls can interpret them as seeks.
pub(super) fn spherical_look_pressed(ctx: &egui::Context, enabled: bool) -> Option<(i32, i32, i32)> {
    if !enabled { return None; }
    ctx.input_mut(|input| {
        let modifiers = input.modifiers;
        if modifiers.ctrl || modifiers.command || modifiers.alt { return None; }
        let left = input.consume_key(modifiers, egui::Key::ArrowLeft);
        let right = input.consume_key(modifiers, egui::Key::ArrowRight);
        let up = input.consume_key(modifiers, egui::Key::ArrowUp);
        let down = input.consume_key(modifiers, egui::Key::ArrowDown);
        if !(left || right || up || down) { return None; }
        let horizontal = (i32::from(right) - i32::from(left)) * 5_000;
        let vertical = (i32::from(up) - i32::from(down)) * 5_000;
        Some(if modifiers.shift { (0, 0, horizontal) } else { (horizontal, vertical, 0) })
    })
}

pub(super) fn controls_pressed(ctx: &egui::Context) -> Vec<Control> {
    use egui::Key;
    let mut out = Vec::new();
    ctx.input(|input| {
        let key = |k: Key| input.key_pressed(k);
        let modifiers = input.modifiers;
        let held = modifiers.command || modifiers.ctrl;

        if held && key(Key::Num3) { out.push(Control::Spherical(modifiers.shift)); }
        if held {
            match (key(Key::ArrowUp), key(Key::ArrowDown)) {
                (true, _) => out.push(Control::Volume(true)),
                (_, true) => out.push(Control::Volume(false)),
                _ => {}
            }
        }
        let back = key(Key::ArrowLeft);
        let forward = key(Key::ArrowRight);
        if (back || forward) && !modifiers.alt {
            if held && !modifiers.shift {
                out.push(Control::Jump(JUMP_LONG, forward));
            } else {
                out.push(Control::Jump(
                    jump_size(modifiers.shift, modifiers.alt),
                    forward,
                ));
            }
        }
        if key(Key::M) {
            out.push(Control::Mute);
        }
        if key(Key::Equals) || key(Key::Plus) {
            out.push(Control::Rate(true));
        }
        if key(Key::Minus) {
            out.push(Control::Rate(false));
        }
        // VLC's fine pair: `,` and `.` nudge the rate by a tenth, completing
        // the preset ladder of `=` and `-`.
        if key(Key::Comma) {
            out.push(Control::RateFine(false));
        }
        if key(Key::Period) {
            out.push(Control::RateFine(true));
        }
        if key(Key::Backslash) {
            out.push(Control::Reset);
        }
        // VLC's Zoom menu, which it gives no key of its own: `z` steps up the
        // four rungs it lists, `Z` back down.
        if key(Key::Z) {
            out.push(Control::Zoom(!modifiers.shift));
        }
        // The shapes VLC lists for cropping, which it cycles through its `crop`
        // variable and fvid puts on one key: `c` walks the list, `C` back, and
        // the walk returns to the whole picture.
        if key(Key::C) {
            out.push(Control::Crop(!modifiers.shift));
        }
        // VLC's aspect-ratio menu, which it cycles through its `aspect-ratio`
        // variable and fvid puts on one key: `v` walks the ten shapes, `V`
        // back, and the walk returns to the shape the file declares. The older
        // player gives this menu `A`, which here lists the audio tracks.
        if key(Key::V) {
            out.push(Control::Aspect(!modifiers.shift));
        }
        // What the item is, which VLC opens as a window of its own on Ctrl+I and
        // fvid puts over the picture on `i`.
        if key(Key::I) {
            out.push(Control::Info);
        }
        // `[` and `]` walk the chapters of the open file.
        if key(Key::OpenBracket) {
            out.push(Control::Chapter(false));
        }
        if key(Key::CloseBracket) {
            out.push(Control::Chapter(true));
        }
        if key(Key::Home) {
            out.push(Control::Position(0.0));
        }
        if key(Key::End) {
            out.push(Control::Position(1.0));
        }
        if key(Key::E) {
            out.push(if held {
                Control::Effects
            } else {
                Control::Frame(!modifiers.shift)
            });
        }
        // `t` shows or hides the cues; with the command key held it is VLC's
        // "Load Subtitle File", which asks for a file to caption the item with.
        if key(Key::T) {
            out.push(if held {
                Control::LoadSubtitles
            } else {
                Control::Subtitles
            });
        }
        // `b` walks the subtitle sources of the open file, Shift back.
        if key(Key::B) {
            out.push(Control::SubtitleTrack(!modifiers.shift));
        }
        // VLC's playlist keys: `n` and `p` step through the queue, `r` cycles
        // what happens at the end of an item.
        if key(Key::N) {
            out.push(Control::Next);
        }
        if key(Key::P) {
            out.push(Control::Previous);
        }
        if key(Key::R) {
            out.push(Control::Repeat);
        }
        // VLC's `s` is Random: it joins the `n`, `p` and `r` trio by deciding
        // in what order the queue walks, not where it stops.
        if !modifiers.shift && key(Key::S) {
            out.push(Control::Shuffle);
        }
        // VLC's audio-delay pair: `k` lets sound arrive later, `j` earlier.
        if key(Key::J) {
            out.push(Control::AudioDelay(false));
        }
        if key(Key::K) {
            out.push(Control::AudioDelay(true));
        }
        // One key marks A, then B, then clears the pair.
        if key(Key::L) {
            out.push(Control::Loop);
        }
        // `a` walks the audio tracks of the open file, Shift back.
        if key(Key::A) {
            out.push(Control::AudioTrack(!modifiers.shift));
        }
        // VLC's snapshot key.
        if modifiers.shift && key(Key::S) {
            out.push(Control::Snapshot);
        }
        // VLC: `g` drops the cue delay, `h` raises it, so text comes later.
        if key(Key::G) {
            out.push(Control::SubtitleDelay(false));
        }
        if key(Key::H) {
            out.push(Control::SubtitleDelay(true));
        }
        if modifiers.alt {
            match (key(Key::ArrowUp), key(Key::ArrowDown)) {
                (true, _) => out.push(Control::SubtitleMargin(true)),
                (_, true) => out.push(Control::SubtitleMargin(false)),
                _ => {}
            }
        }
        if modifiers.alt {
            match (key(Key::Equals), key(Key::Minus)) {
                (true, _) => out.push(Control::SubtitleSize(true)),
                (_, true) => out.push(Control::SubtitleSize(false)),
                _ => {}
            }
        }
        for digit in 0..=9u8 {
            let key = match digit {
                0 => Key::Num0,
                1 => Key::Num1,
                2 => Key::Num2,
                3 => Key::Num3,
                4 => Key::Num4,
                5 => Key::Num5,
                6 => Key::Num6,
                7 => Key::Num7,
                8 => Key::Num8,
                _ => Key::Num9,
            };
            if !held && input.key_pressed(key)
                && let Some(fraction) = position_from_digit(digit)
            {
                out.push(Control::Position(fraction));
            }
        }
    });
    out
}

/// Draw the active cue over the video, preserving authored and wrapped rows.
pub(super) fn paint_subtitle_block(
    painter: &egui::Painter,
    line: String,
    font: FontId,
    frame: Rect,
    margin: f32,
    controls: Option<Rect>,
) -> (Pos2, std::sync::Arc<egui::Galley>) {
    let mut job = egui::text::LayoutJob::simple(line, font, Color32::WHITE, frame.width() * 0.86);
    job.halign = egui::Align::Center;
    let wrapped = painter.layout_job(job);
    // Controls paint after captions; keep their backing clear of the glyphs.
    let margin = controls.map_or(margin, |panel| {
        margin.max(frame.bottom() - panel.top() + 12.0)
    });
    // Center-aligned galley coordinates already straddle x=0.
    let at = Pos2::new(frame.center().x, frame.bottom() - margin - wrapped.size().y);
    for offset in [
        Vec2::new(-1.5, 0.0),
        Vec2::new(1.5, 0.0),
        Vec2::new(0.0, -1.5),
        Vec2::new(0.0, 1.5),
    ] {
        // A fallback color cannot recolor the layout's explicit white glyphs.
        painter.galley_with_override_text_color(at + offset, wrapped.clone(), Color32::BLACK);
    }
    painter.galley(at, wrapped.clone(), Color32::WHITE);
    (at, wrapped)
}

/// Space belongs to playback, even when a chrome button retains keyboard focus.
pub(super) fn play_pause_pressed(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Space))
}


impl Player {
    /// Carry one keyboard control through to playback state.
    pub(super) fn apply(&mut self, control: Control) {
        match control {
            Control::Spherical(cycle) => self.change_spherical(cycle),
            Control::Volume(up) => self.change_volume(up),
            Control::Mute => self.toggle_mute(),
            Control::Rate(up) => self.change_rate(up),
            Control::RateFine(up) => self.set_rate(rate_fine(self.rate_milli, up)),
            Control::Reset => self.set_rate(1_000),
            Control::Jump(size, forward) => self.jump(size, forward),
            Control::Chapter(forward) => self.walk_chapter(forward),
            Control::Position(fraction) => self.seek_fraction(fraction),
            Control::Frame(forward) => self.step_frame(forward),
            Control::Subtitles => {
                if self.cues.is_empty() {
                    self.show_osd("No subtitles");
                } else {
                    self.subtitle_shown = !self.subtitle_shown;
                    self.show_osd(if self.subtitle_shown {
                        "Subtitles on"
                    } else {
                        "Subtitles off"
                    });
                }
            }
            Control::SubtitleDelay(later) => {
                self.subtitle_delay_ms = delay_step(self.subtitle_delay_ms, later);
                self.show_osd(format!("Subtitle delay {} ms", self.subtitle_delay_ms));
            }
            Control::SubtitleMargin(upper) => {
                self.subtitle_margin =
                    (self.subtitle_margin + if upper { -16.0 } else { 16.0 }).clamp(8.0, 400.0);
                self.show_osd(format!("Subtitles at {:.0} px", self.subtitle_margin));
            }
            Control::SubtitleSize(larger) => {
                self.subtitle_font =
                    (self.subtitle_font + if larger { 2.0 } else { -2.0 }).clamp(10.0, 72.0);
                self.show_osd(format!("Subtitle size {:.0}", self.subtitle_font));
            }
            Control::SubtitleTrack(forward) => self.cycle_subtitle_track(forward),
            Control::Next => self.step_queue(false),
            Control::Previous => self.step_queue(true),
            Control::Repeat => {
                self.repeat = cycle_repeat(self.repeat);
                self.show_osd(repeat_osd(self.repeat));
            }
            Control::Shuffle => self.toggle_shuffle(),
            Control::Loop => self.press_loop(),
            Control::AudioDelay(later) => {
                self.audio_delay_ms = delay_step(self.audio_delay_ms, later);
                self.show_osd(format!("Audio delay {} ms", self.audio_delay_ms));
            }
            Control::AudioTrack(forward) => self.cycle_audio_track(forward),
            Control::Snapshot => self.take_snapshot(),
            Control::Zoom(up) => self.change_zoom(up),
            Control::Crop(forward) => self.change_crop(forward),
            Control::Aspect(forward) => self.change_aspect(forward),
            Control::Info => self.toggle_info(),
            Control::Effects => self.toggle_effects(),
            Control::LoadSubtitles => self.pick_subtitles(),
        }
    }

    pub(super) fn pick_file(&mut self, what: Pick) {
        if self.dialog.is_some() {
            return;
        }
        // Each list is what the player can take at that place: the picture
        // readers' own set of containers, the bare audio files the frame walkers
        // open, and the three cue formats a sidecar or a `--sub-file` is read
        // from. VLC's dialog offers sound on its own, and a list that hides the
        // files behind the open dialog's filter would be a worse picker than the
        // command line.
        let dialog = rfd::AsyncFileDialog::new();
        let dialog = match what {
            Pick::Item => dialog
                .add_filter("Playlists", &["m3u", "m3u8", "pls"])
                .add_filter("Media", &["y4m", "mp4", "mov", "m4v", "m4a", "webm", "mkv", "mka", "avi"])
                .add_filter(
                    "Audio",
                    &[
                        "mp3", "mp2", "mp1", "flac", "aac", "ac3", "wav", "w64", "aif", "aiff", "au", "snd",
                        "ogg", "wv", "wvc", "mid", "xm",
                    ],
                ),
            Pick::Folder => dialog,
            Pick::Subtitles => {
                dialog.add_filter("Subtitles", &["srt", "vtt", "ass", "ssa", "smi", "sami", "smil"])
            }
        };
        self.dialog = Some((what, if what == Pick::Folder {
            Box::pin(dialog.pick_folder())
        } else {
            Box::pin(dialog.pick_file())
        }));
    }

    /// Ask for a file to caption the item on screen with. An item that is not
    /// open has nothing to be captioned by, and says so rather than opening a
    /// picker whose answer would be dropped.
    pub(super) fn pick_subtitles(&mut self) {
        if !self.item_open() {
            self.show_osd("Nothing to caption");
            return;
        }
        self.pick_file(Pick::Subtitles);
    }

    pub(super) fn poll_dialog(&mut self) {
        let Some((what, dialog)) = &mut self.dialog else {
            return;
        };
        let mut cx = Context::from_waker(Waker::noop());
        if let Poll::Ready(handle) = dialog.as_mut().poll(&mut cx) {
            let what = *what;
            self.dialog = None;
            if let Some(handle) = handle {
                let path = handle.path().to_path_buf();
                match what {
                    Pick::Item | Pick::Folder => self.open_queue(vec![path]),
                    Pick::Subtitles => self.add_subtitle_file(path),
                }
            }
        }
    }
}

impl eframe::App for Player {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_open(ctx);
        self.prefetch_next();
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if play_pause_pressed(ctx) {
            self.toggle_pause();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.command) {
            self.pick_file(if ctx.input(|i| i.modifiers.shift) { Pick::Folder } else { Pick::Item });
        }
        if let Some((yaw, pitch, roll)) = spherical_look_pressed(ctx, self.spherical.enabled) {
            self.look_spherical(yaw, pitch, roll);
        }
        for control in controls_pressed(ctx) {
            self.apply(control);
        }
        // A settings dialog left open over a paused frame moves sliders, and a
        // moved slider has to reach the picture the same way a moved crop does.
        // One mouse notch is one VLC volume step; smaller trackpad ticks are
        // ignored rather than accumulated into a slide.
        if let Some(delta) = ctx.input(|i| Some(i.smooth_scroll_delta.y))
            && delta.abs() >= 24.0
        {
            self.change_volume(delta > 0.0);
        }
        if ctx
            .input(|i| i.pointer.is_moving() || i.pointer.any_pressed() || i.pointer.any_released())
        {
            self.activity = Instant::now();
        }
        // A drop of several files is a playlist, the way VLC queues them up.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_queue(expand_inputs(&dropped));
        }
        if self.dialog.is_some() {
            self.poll_dialog();
            if self.dialog.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        self.present(ctx);
        if self.should_quit() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
        if self.playback.is_some() && !self.paused && !self.ended {
            self.playback_activity
                .get_or_insert_with(fvid_platform::PlaybackActivity::new);
        } else {
            self.playback_activity = None;
        }
        // Wake up once to let the controls fade after the pointer rests.
        if self.controls_visible() && !self.paused {
            ctx.request_repaint_after(HIDE_AFTER.saturating_sub(self.activity.elapsed()));
        }
        // And once more to take a key message down when its time is up.
        if let Some((_, since)) = &self.osd
            && since.elapsed() < OSD_AFTER
        {
            ctx.request_repaint_after(OSD_AFTER.saturating_sub(since.elapsed()));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let panel = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(WINDOW));
        panel.show(ui, |ui| {
            let frame = ui.max_rect();
            let painter = ui.painter().with_clip_rect(frame);
            let (skin_text, skin_accent, skin_panel) =
                (self.skin.text, self.skin.played, self.skin.panel);
            painter.rect_filled(frame, CornerRadius::ZERO, FRAME);

            // The picture, cropped to VLC's shape, drawn at the shape VLC's
            // aspect menu or its container gives it, fitted inside the frame and
            // then magnified the way its Zoom menu does, the window keeping its
            // size. Planar frames are converted to RGB by the GPU shader while
            // drawing, which is handed the crop as texture coordinates and the
            // grade the picture kept for it.
            if let Some((planes, serial, grade, rotation)) = &self.video {
                let (w, h) = (planes.width, planes.height);
                let raw_dims = if *rotation == 90 || *rotation == 270 { [h, w] } else { [w, h] };
                let view_dims = stereo_dimensions(raw_dims, self.stereo3d);
                let source = Vec2::new(view_dims[0] as f32, view_dims[1] as f32);
                let insets =
                    shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let cropped = cropped_size(source, insets);
                let size = shown_size(
                    cropped,
                    self.pixel_aspect,
                    self.aspect,
                    frame.size(),
                );
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    VideoCallback {
                        frame: planes.clone(),
                        serial: *serial,
                        window: uv_window(source, insets),
                        adjust: adjust_scalars(&self.adjust),
                        // The grade this picture kept its planes for, which the
                        // shader reads out of the table the CPU would have read;
                        // see [`Grade::shader_look`] for what reaches here.
                        grade: grade.clone(),
                        rotation: *rotation,
                        view: gpu_view_params(
                            self.stereo3d,
                            self.spherical,
                            cropped.x / cropped.y.max(1.0),
                        ),
                    },
                ));
            } else if let Some((planes, serial, grade, rotation)) = &self.video_packed {
                let (w, h) = (planes.frame.width, planes.frame.height);
                let raw_dims = if *rotation == 90 || *rotation == 270 { [h, w] } else { [w, h] };
                let view_dims = stereo_dimensions(raw_dims, self.stereo3d);
                let source = Vec2::new(view_dims[0] as f32, view_dims[1] as f32);
                let insets = shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let cropped = cropped_size(source, insets);
                let size = shown_size(cropped, self.pixel_aspect, self.aspect, frame.size());
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect,
                    crate::player_gpu::PackedVideoCallback {
                        frame: planes.clone(), serial: *serial, window: uv_window(source, insets),
                        adjust: adjust_scalars(&self.adjust), grade: grade.clone(), rotation: *rotation,
                        view: gpu_view_params(self.stereo3d, self.spherical, cropped.x / cropped.y.max(1.0)),
                    },
                ));
            } else if let Some((rgb, dimensions)) = &self.rgb_frame {
                let source = Vec2::new(dimensions[0] as f32, dimensions[1] as f32);
                let insets = shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let size = shown_size(cropped_size(source, insets), self.pixel_aspect, self.aspect, frame.size());
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect,
                    crate::player_gpu::RgbVideoCallback {
                        frame: rgb.clone(), size: *dimensions, window: uv_window(source, insets),
                        adjust: adjust_scalars(&self.adjust),
                        view: crate::player_gpu::IDENTITY_VIEW,
                    }));
            } else if let Some(texture) = &self.texture {
                let source = texture.size_vec2();
                let insets =
                    shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let size = shown_size(
                    cropped_size(source, insets),
                    self.pixel_aspect,
                    self.aspect,
                    frame.size(),
                );
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                let [u0, v0, u1, v1] = uv_window(source, insets);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::new(u0, v0), Pos2::new(u1, v1)),
                    Color32::WHITE,
                );
            }

            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            if let Some((surface, serial)) = &self.video_surface {
                let (w, h) = (surface.surface.width(), surface.surface.height());
                let raw_dims = if surface.rotation == 90 || surface.rotation == 270 { [h, w] } else { [w, h] };
                let view_dims = stereo_dimensions(raw_dims, self.stereo3d);
                let source = Vec2::new(view_dims[0] as f32, view_dims[1] as f32);
                let insets = shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let cropped = cropped_size(source, insets);
                let size = shown_size(cropped, self.pixel_aspect, self.aspect, frame.size());
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect,
                    crate::player_gpu::SurfaceVideoCallback {
                        frame: surface.surface.clone(), rotation: surface.rotation, colour: surface.colour, serial: *serial,
                        window: uv_window(source, insets), adjust: adjust_scalars(&self.adjust), grade: surface.grade.clone(),
                        view: gpu_view_params(self.stereo3d, self.spherical, cropped.x / cropped.y.max(1.0)),
                    }));
            }

            if self.opening.is_some() || (self.playback.is_some() && self.buffering && self.error.is_none()) {
                ctx.request_repaint_after(Duration::from_millis(16));
                ui.put(Rect::from_center_size(frame.center() - Vec2::new(0.0, 52.0), Vec2::splat(24.0)), egui::Spinner::new());
                let loading = if let Some(bytes) = self.network_downloaded {
                    format!("Загрузка {}\nHTTP: {:.1} МБ", self.name, bytes as f64 / 1048576.0)
                } else {
                    format!("Загрузка {}\n{} · {} с\nБуфер: {} кадров · Загружено: {:.1} МБ", self.name,
                        self.opening_stage, self.loading_since.map_or(0, |since| since.elapsed().as_secs()),
                        self.playback.as_ref().map_or_else(|| self.loading_buffer.as_ref().map_or(0, |counter| counter.load(std::sync::atomic::Ordering::Relaxed)), |playback| playback.filled()),
                        self.spool.as_ref().map_or(0.0, |spool| spool.copied() as f64 / 1048576.0))
                };
                painter.text(frame.center(), Align2::CENTER_CENTER, loading,
                    FontId::proportional(18.0), TEXT);
            }
            // An item that is only sound has no picture to show, so the stage
            // says what it is instead of looking like a failure.
            if self.playback.is_none() && self.item_open() {
                icon_note(
                    &painter,
                    Pos2::new(frame.center().x, frame.center().y - 70.0),
                    64.0,
                    DIM,
                );
            }

            // VLC's Ctrl+E dialog of picture settings: its switch, its five
            // sliders with its ranges and labels, and its button that puts the
            // defaults back. VLC keeps the window apart from the video; here
            // it stands over the picture, and the sliders write the bundle the
            // draw paths read every repaint.
            if self.effects {
                let adjust = &mut self.adjust;
                egui::Window::new("Adjustments and Effects")
                    .open(&mut self.effects)
                    .default_pos(frame.center() + Vec2::new(-120.0, -140.0))
                    .show(&ctx, |ui| {
                        ui.checkbox(&mut adjust.on, "Enable");
                        ui.add_enabled_ui(adjust.on, |ui| {
                            ui.add(
                                egui::Slider::new(&mut adjust.brightness, 0.0..=2.0)
                                    .text("Brightness"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.gamma, 0.01..=10.0)
                                    .logarithmic(true)
                                    .text("Gamma"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.saturation, 0.0..=3.0)
                                    .text("Colour saturation"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.contrast, 0.0..=2.0).text("Contrast"),
                            );
                            ui.add(egui::Slider::new(&mut adjust.hue, -180.0..=180.0).text("Tone"));
                            if ui.button("Restore Default Values").clicked() {
                                *adjust = Adjust {
                                    on: adjust.on,
                                    ..Adjust::default()
                                };
                            }
                        });
                    });
            }

            // Clicking the picture toggles playback; the buttons below take priority
            // because they are interacted with later in the same frame. A picture a
            // magnification has grown past the frame is dragged to the part of it
            // the frame cannot show, the way VLC pans a zoomed one; egui stops
            // counting a press that moved as a click, so the two answers do not
            // fight over the same gesture.
            let surface = ui.interact(frame, ui.id().with("surface"), Sense::click_and_drag());
            if surface.dragged() && self.zoom_milli > 1_000 {
                self.pan += surface.drag_delta();
            } else if surface.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            } else if surface.clicked() {
                if self.item_open() {
                    self.toggle_pause();
                } else {
                    self.pick_file(Pick::Item);
                }
            }

            let visible = self.controls_visible();
            if visible {
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            } else {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }

            // Centre: one large play button while idle.
            if self.opening.is_none()
                && !(self.playback.is_some() && self.buffering && self.presented.is_none())
                && (!self.item_open() || self.paused || self.ended)
            {
                let center = frame.center();
                let size = 88.0_f32.min(frame.height() * 0.4);
                let clicked = round_button(ui, "big-play", center, size, CHIP, |p, c, color| {
                    icon_play(p, Pos2::new(c.x + size * 0.04, c.y), size * 0.34, color);
                });
                if clicked {
                    if self.item_open() {
                        self.toggle_pause();
                    } else {
                        self.pick_file(Pick::Item);
                    }
                }
                if !self.item_open() {
                    painter.text(
                        Pos2::new(center.x, center.y + size / 2.0 + 24.0),
                        Align2::CENTER_TOP,
                        "Open or drop a Y4M, MP4, MOV or Matroska file",
                        FontId::proportional(13.0),
                        MUTED,
                    );
                }
            }

            // A file carried over the window lights the frame up before it is let go.
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                painter.rect_stroke(
                    frame.shrink(12.0),
                    CornerRadius::same(20),
                    Stroke::new(2.0, skin_accent),
                    egui::StrokeKind::Inside,
                );
            }

            let pad = 28.0_f32.min(frame.width() * 0.05);

            let panel_bottom = frame.bottom() - 20.0;
            let row_y = panel_bottom - 8.0 - BUTTON / 2.0;
            let bar_y = row_y - BUTTON / 2.0 - if self.skin.diagram { 34.0 } else { 12.0 };
            let clock_y = bar_y - 18.0;
            let panel = Rect::from_min_max(
                Pos2::new(frame.left() + pad, clock_y - 22.0),
                Pos2::new(frame.right() - pad, panel_bottom),
            );

            // Captions and key messages stay on screen while the panel hides, so
            // a key pressed during playback still answers without a mouse move.
            // Captions go first, so a key message can still be read over them.
            if let Some(line) = self
                .timeline()
                .0
                .and_then(|at| self.subtitle_line(at))
                .map(str::to_owned)
            {
                let font = FontId::proportional(self.subtitle_font);
                paint_subtitle_block(&painter, line, font, frame, self.subtitle_margin, visible.then_some(panel));
            }
            // The key message sits in the corner opposite the title.
            if let Some((message, since)) = &self.osd
                && since.elapsed() < OSD_AFTER
            {
                let galley =
                    painter.layout_no_wrap(message.clone(), FontId::proportional(15.0), skin_text);
                let size = galley.size() + Vec2::new(32.0, 20.0);
                let right = frame.right() - pad;
                let backing =
                    Rect::from_min_size(Pos2::new(right - size.x, frame.top() + 24.0), size);
                painter.rect_filled(backing, CornerRadius::same(10), skin_panel);
                painter.galley(backing.min + Vec2::new(16.0, 10.0), galley, skin_text);
            }

            // Keep cached coverage visible when controls fade, at the window edge.
            let pixel = 1.0 / ctx.pixels_per_point();
            let bottom = (frame.bottom() / pixel).floor() * pixel;
            let (elapsed, total) = self.timeline();
            let decoded_range = elapsed.zip(total).filter(|(_, total)| !total.is_zero())
                .and_then(|(elapsed, total)| self.playback.as_ref().map(|playback| {
                    let near = (elapsed.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0);
                    [near, buffered_fraction(near, playback.filled(), self.period, Some(total))]
                }));
            // Independent rows: active decode queue, RAM cache, disk spool.
            // One physical pixel per row, with one dark pixel between rows.
            let rows = [
                (decoded_range.into_iter().collect::<Vec<_>>(), Color32::from_rgb(0x26, 0x8b, 0xff)),
                (self.storage_ranges(true), Color32::from_rgb(0x42, 0xd3, 0x92)),
                (self.storage_ranges(false), Color32::from_rgb(0xff, 0xb4, 0x54)),
            ];
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(frame.left(), bottom - 5.0 * pixel), Pos2::new(frame.right(), bottom)),
                CornerRadius::ZERO, Color32::from_rgb(12, 14, 18));
            for (row, (ranges, color)) in rows.into_iter().enumerate() {
                let y = bottom - row as f32 * 2.0 * pixel;
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(frame.left(), y - pixel), Pos2::new(frame.right(), y)),
                    CornerRadius::ZERO, color.gamma_multiply(0.18));
                for [near, far] in ranges {
                    let left = frame.left() + frame.width() * near.clamp(0.0, 1.0);
                    let right = frame.left() + frame.width() * far.clamp(0.0, 1.0);
                    if right > left {
                        painter.rect_filled(
                            Rect::from_min_max(Pos2::new(left, y - pixel), Pos2::new(right, y)),
                            CornerRadius::ZERO,
                            color,
                        );
                    }
                }
            }

            // Top: title block.
            // Keep the title block clear of the macOS traffic lights over the hidden title bar.
            let title_left = if fullscreen { pad } else { pad.max(96.0) };
            let top = Pos2::new(frame.left() + title_left, frame.top() + 24.0);
            let preparing_seek = self.playback.as_ref().is_some_and(|p| p.preparing_seek());
            if let Some(playback) = self.playback.as_ref().filter(|_| preparing_seek) {
                ctx.request_repaint_after(Duration::from_millis(100));
                let progress: f32 = self.duration.map_or(0.0, |total|
                    playback.prepared_frame_ranges(total).iter().map(|[a,b]| b-a).sum());
                painter.text(Pos2::new(top.x, top.y + 24.0),
                    Align2::LEFT_TOP, format!("Подготовка быстрой перемотки: {:.0}%", progress.clamp(0.0, 1.0) * 100.0),
                    FontId::proportional(12.0), skin_text);
            }

            if !visible {
                return;
            }

            if self.item_open() {
                painter.text(
                    top,
                    Align2::LEFT_TOP,
                    &self.name,
                    FontId::proportional(17.0),
                    skin_text,
                );
                // Use the top panel's metadata row for preparation progress.
                // Restore the media details once preparation has finished.
                if !preparing_seek {
                    painter.text(
                        Pos2::new(top.x, top.y + 24.0),
                        Align2::LEFT_TOP,
                        self.subtitle(),
                        FontId::proportional(13.0),
                        MUTED,
                    );
                }
            } else {
                painter.text(
                    top,
                    Align2::LEFT_TOP,
                    "FVid",
                    FontId::proportional(17.0),
                    skin_text,
                );
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
            // What the item is, in the title's corner under the title, in the
            // subtitle line's type. An item that cannot be opened has nothing to
            // list here, so the panel and the error never share the corner.
            if self.info {
                let font = FontId::proportional(13.0);
                for (nth, line) in self.info_lines().into_iter().enumerate() {
                    painter.text(
                        Pos2::new(top.x, top.y + 48.0 + 18.0 * nth as f32),
                        Align2::LEFT_TOP,
                        line,
                        font.clone(),
                        skin_text,
                    );
                }
            }
            // Bottom: one panel holding the clocks, the progress line under
            // them and the control row, on a backing the picture shows through.
            // The panel takes the pointer itself, so a click between its
            // buttons does not fall through to the picture and pause it.
            ui.interact(panel, ui.id().with("panel"), Sense::click());
            painter.rect_filled(panel, CornerRadius::same(PANEL_RADIUS), self.skin.panel);
            let hit = Rect::from_min_max(
                Pos2::new(panel.left() + 22.0, bar_y - 12.0),
                Pos2::new(panel.right() - 22.0, bar_y + 12.0),
            );
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
            // Where the picture actually stands, kept apart from the fraction
            // the line may be showing under a drag: the loaded span is measured
            // from the picture, not from the pointer.
            let played = fraction;
            // The pictures already in hand, drawn between the played part and
            // the part the source has not delivered, so one line answers both
            // questions a stall raises: how far ahead the player has got, and
            // whether the answer is shrinking.
            let loaded = match (played, self.playback.as_ref()) {
                (Some(played), Some(playback)) => Some(buffered_fraction(
                    played,
                    playback.filled(),
                    self.period,
                    total,
                )),
                _ => None,
            };
            // The line takes clicks and drags on a taller hit area; the seek
            // itself happens on release so a drag decodes only once. Where the
            // pointer rests on it, the line thickens and names the moment.
            let mut hover = None;
            let mut timeline_active = false;
            if fraction.is_some() && self.seekable {
                let seek = ui.interact(hit, ui.id().with("seek"), Sense::click_and_drag());
                // Keep the expansion alive even when a drag leaves the hit
                // area, or a diagram label takes hover ownership.
                timeline_active = seek.is_pointer_button_down_on()
                    || self.scrub.is_some()
                    || ui.rect_contains_pointer(hit);
                let at = |pos: Pos2| ((pos.x - hit.left()) / hit.width()).clamp(0.0, 1.0);
                if seek.dragged()
                    && let Some(pos) = seek.interact_pointer_pos()
                {
                    self.scrub = Some(at(pos));
                }
                if (seek.drag_stopped() || seek.clicked())
                    && let Some(pos) = seek.interact_pointer_pos()
                {
                    let target = at(pos);
                    self.scrub = None;
                    self.seek_fraction(target);
                    fraction = Some(target);
                }
                if timeline_active {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    hover = self.scrub.or_else(|| ctx.pointer_hover_pos().map(at));
                }
                if let Some(scrub) = self.scrub {
                    fraction = Some(scrub);
                }
            }
            let preview_id = ui.id().with("timeline-last-preview");
            if let Some(target) = hover {
                ctx.data_mut(|data| data.insert_temp(preview_id, target));
            }
            let expansion = smooth_timeline_value(
                &ctx,
                ui.id().with("timeline-expansion"),
                if timeline_active { 1.0 } else { 0.0 },
                0.0,
                0.120,
            );
            let thick = if self.skin.diagram {
                20.0 + 4.0 * expansion
            } else {
                4.0 + 6.0 * expansion
            };
            // Animate only the drawing; seeks and preview timestamps keep the
            // exact pointer target. A short transition also follows a drag.
            let fraction = fraction.map(|target| {
                smooth_timeline_position(
                    &ctx,
                    ui.id().with("timeline-position"),
                    target,
                )
            });
            let bar = Rect::from_min_max(
                Pos2::new(hit.left(), bar_y - thick / 2.0),
                Pos2::new(hit.right(), bar_y + thick / 2.0),
            );
            let round = if self.skin.diagram {
                CornerRadius::ZERO
            } else {
                CornerRadius::same((thick / 2.0) as u8)
            };
            if self.skin.diagram {
                painter.line_segment(
                    [bar.left_center(), bar.right_center()],
                    Stroke::new(1.0 + 2.0 * expansion, self.skin.track),
                );
            } else {
                painter.rect_filled(bar, round, self.skin.track);
            }
            let swap_ranges = self.swap_ranges();
            if let (Some(played), Some(loaded)) = (played, loaded) {
                let from = bar.left() + bar.width() * played;
                let actual_to = bar.left() + bar.width() * loaded;
                let to = if !self.skin.diagram && actual_to > from {
                    // Keep a tiny queue visible past the 7 px playhead. The
                    // timeline fraction itself remains exact for seeking.
                    actual_to.max(from + 16.0).min(bar.right())
                } else { actual_to };
                if to > from {
                    let span =
                        Rect::from_min_max(Pos2::new(from, bar.min.y), Pos2::new(to, bar.max.y));
                    if self.skin.diagram {
                        painter.rect_filled(span, 0.0, self.skin.panel);
                        player_skin::pattern(&painter, span, self.skin.buffer, true);
                    } else {
                        painter.rect_filled(span, CornerRadius::same(2), self.skin.buffer);
                    }
                }
            }
            if let Some(fraction) = fraction {
                let x = bar.left() + bar.width() * fraction;
                painter.rect_filled(
                    Rect::from_min_max(bar.min, Pos2::new(x, bar.max.y)),
                    round,
                    self.skin.played,
                );
            }
            // Chapter starts cut the line, the way the canvas marks them.
            if let Some(total) = total.filter(|t| *t > Duration::ZERO) {
                for mark in self.chapters.iter().filter(|m| m.start > Duration::ZERO) {
                    let x =
                        bar.left() + bar.width() * (mark.start.as_secs_f32() / total.as_secs_f32());
                    if x < bar.right() {
                        painter.rect_filled(
                            Rect::from_min_max(
                                Pos2::new(x - 1.0, bar.top()),
                                Pos2::new(x + 1.0, bar.bottom()),
                            ),
                            CornerRadius::ZERO,
                            WINDOW,
                        );
                    }
                }
            }
            if let Some(fraction) = fraction.filter(|_| !self.skin.diagram) {
                painter.circle_filled(
                    Pos2::new(bar.left() + bar.width() * fraction, bar_y),
                    5.0 + 3.0 * expansion,
                    skin_text,
                );
            }
            if self.skin.diagram {
                let played_end = fraction.unwrap_or(0.0);
                let buffer_end = loaded.unwrap_or(played_end).max(played_end);
                let covered = swap_ranges.iter().find(|range| range[1] >= buffer_end).copied();
                let [swap_start, swap_end] = covered.map_or([buffer_end, buffer_end], |span| {
                    [span[0].max(buffer_end), span[1].max(buffer_end)]
                });
                let regions = [
                    (0.0, played_end, "проиграно"),
                    (played_end, buffer_end, "буфер"),
                    (buffer_end, swap_start, "ещё не загружено"),
                    (swap_start, swap_end, "swap"),
                    (swap_end, 1.0, "ещё не загружено"),
                ];
                for (region_index, (near, far, label)) in regions.into_iter().enumerate() {
                    if total.is_none() || far <= near {
                        continue;
                    }
                    let left = bar.left() + bar.width() * near;
                    let right = bar.left() + bar.width() * far;
                    let galley = painter.layout_no_wrap(
                        label.into(),
                        FontId::monospace(11.0),
                        self.skin.text,
                    );
                    if right - left >= galley.size().x + 4.0 {
                        painter.galley(
                            Pos2::new((left + right - galley.size().x) / 2.0, bar.bottom() + 5.0),
                            galley,
                            self.skin.text,
                        );
                    }
                    let area = Rect::from_min_max(
                        Pos2::new(left, bar.top()),
                        Pos2::new(right.max(left + 3.0), bar.bottom()),
                    );
                    ui.interact(
                        area,
                        ui.id().with(("skin-region", region_index)),
                        Sense::hover(),
                    )
                    .on_hover_text(label);
                }
                // Keep endpoint labels apart when the real buffer spans only a few pixels.
                let mut last_clock_x = bar.left();
                for at in [played_end, buffer_end, swap_start, swap_end, 1.0] {
                    let x = bar.left() + bar.width() * at;
                    painter.line_segment(
                        [Pos2::new(x, bar.top()), Pos2::new(x, bar.bottom())],
                        Stroke::new(1.0, self.skin.text),
                    );
                    if at < 1.0 && x - last_clock_x > 60.0 && bar.right() - x > 60.0 {
                        if let Some(total) = total {
                            painter.text(
                                Pos2::new(x, clock_y),
                                Align2::CENTER_CENTER,
                                clock(Duration::from_millis(
                                    (total.as_secs_f64() * f64::from(at) * 1000.0).round() as u64,
                                )),
                                FontId::monospace(13.0),
                                self.skin.text,
                            );
                            last_clock_x = x;
                        }
                    }
                }
            }
            // The tip over the panel: the moment under the pointer and the
            // chapter it falls in.
            let preview = hover.or_else(|| {
                (expansion > 0.0)
                    .then(|| ctx.data(|data| data.get_temp::<f32>(preview_id)))
                    .flatten()
            });
            if let (Some(hover), Some(total)) = (preview, total) {
                // Move the timestamp and its label together during retargeting.
                // The actual seek still uses the exact pointer coordinate.
                let preview_position = smooth_timeline_position(
                    &ctx,
                    ui.id().with("timeline-preview-position"),
                    hover,
                );
                let moment = total.mul_f32(preview_position);
                let mut label = clock(moment);
                if let Some((_, title)) = chapter_at(&self.chapters, moment)
                    && !title.is_empty()
                {
                    label = format!("{label} \u{b7} {title}");
                }
                let galley = painter.layout_no_wrap(label, FontId::monospace(13.0), skin_text);
                let size = galley.size() + Vec2::new(16.0, 8.0);
                if !self.skin.diagram {
                    // Follow the same animated position as the timestamp.
                    // Pointer hit testing and the actual seek remain exact.
                    painter.circle_filled(
                        Pos2::new(bar.left() + bar.width() * preview_position, bar_y),
                        3.0 * expansion,
                        skin_text.gamma_multiply(0.55 * expansion),
                    );
                }
                let x = (bar.left() + bar.width() * preview_position - size.x / 2.0)
                    .clamp(frame.left() + 8.0, frame.right() - 8.0 - size.x);
                let tip = Rect::from_min_size(
                    Pos2::new(x, panel.top() - 8.0 - size.y + 4.0 * (1.0 - expansion)),
                    size,
                );
                painter.rect_filled(tip, CornerRadius::same(6), skin_panel.gamma_multiply(expansion));
                painter.galley(
                    tip.min + Vec2::new(8.0, 4.0),
                    galley,
                    skin_text.gamma_multiply(expansion),
                );
            }
            // Clocks over the two ends of the line; the right one flips between
            // the whole length and what is left of it.
            if let Some(elapsed) = elapsed {
                let font = FontId::monospace(13.0);
                painter.text(
                    Pos2::new(hit.left(), clock_y),
                    Align2::LEFT_CENTER,
                    clock(elapsed),
                    font.clone(),
                    skin_text,
                );
                if let Some(total) = total {
                    let shown = painter.text(
                        Pos2::new(hit.right(), clock_y),
                        Align2::RIGHT_CENTER,
                        end_clock(elapsed, total, self.remaining),
                        font,
                        MUTED,
                    );
                    let flip =
                        ui.interact(shown.expand(6.0), ui.id().with("end-clock"), Sense::click());
                    if flip.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if flip.clicked() {
                        self.remaining = !self.remaining;
                    }
                }
            }

            // Left group: play/pause, restart, the chapter playing.
            let mut x = panel.left() + 8.0 + BUTTON / 2.0;
            let playing = self.item_open() && !self.paused && !self.ended;
            let clicked = round_button(
                ui,
                "play",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    if playing {
                        icon_pause(p, c, 16.0, color);
                    } else {
                        icon_play(p, Pos2::new(c.x + 1.0, c.y), 16.0, color);
                    }
                },
            );
            if clicked {
                if self.item_open() {
                    self.toggle_pause();
                } else {
                    self.pick_file(Pick::Item);
                }
            }
            x += BUTTON + 4.0;
            if round_button(
                ui,
                "restart",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    icon_restart(p, c, 14.0, color);
                },
            ) {
                self.restart();
            }
            x += BUTTON / 2.0 + 12.0;
            if let Some((nth, title)) = elapsed.and_then(|at| chapter_at(&self.chapters, at)) {
                let end = painter.text(
                    Pos2::new(x, row_y),
                    Align2::LEFT_CENTER,
                    format!("Chapter {nth} of {}", self.chapters.len()),
                    FontId::proportional(13.0),
                    MUTED,
                );
                if !title.is_empty() {
                    painter.text(
                        Pos2::new(end.right() + 10.0, row_y),
                        Align2::LEFT_CENTER,
                        title,
                        FontId::proportional(13.0),
                        skin_text,
                    );
                }
            }

            // Right group: rate, open, fullscreen.
            let mut x = panel.right() - 8.0 - BUTTON / 2.0;
            if round_button(
                ui,
                "fullscreen",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    icon_fullscreen(p, c, 16.0, color, fullscreen);
                },
            ) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }
            x -= BUTTON + 4.0;
            let label = "Open…";
            let font = FontId::proportional(13.0);
            let width = painter
                .layout_no_wrap(label.to_owned(), font.clone(), skin_text)
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
            painter.text(
                open_rect.center(),
                Align2::CENTER_CENTER,
                label,
                font,
                skin_text,
            );
            if open.clicked() {
                self.pick_file(Pick::Item);
            }
            open.context_menu(|ui| {
                if ui.button("Open file or playlist…").clicked() {
                    self.pick_file(Pick::Item);
                    ui.close();
                }
                if ui.button("Open folder…").clicked() {
                    self.pick_file(Pick::Folder);
                    ui.close();
                }
            });
            // A rate other than the recorded one stays in sight in the accent,
            // and a click on it goes back to 1x, as the `\` key does.
            if self.rate_milli != 1_000 {
                let font = FontId::monospace(14.0);
                let label = rate_osd(self.rate_milli);
                let width = painter
                    .layout_no_wrap(label.clone(), font.clone(), skin_accent)
                    .size()
                    .x
                    + 24.0;
                let rate_rect = Rect::from_center_size(
                    Pos2::new(open_rect.left() - 4.0 - width / 2.0, row_y),
                    Vec2::new(width, BUTTON),
                );
                let rate = ui.interact(rate_rect, ui.id().with("rate"), Sense::click());
                if rate.hovered() {
                    painter.rect_filled(rate_rect, CornerRadius::same(10), CHIP_STRONG);
                }
                painter.text(
                    rate_rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    font,
                    skin_accent,
                );
                if rate.clicked() {
                    self.set_rate(1_000);
                }
            }
        });
    }
}
