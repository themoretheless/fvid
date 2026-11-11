use super::{
    HIDE_AFTER, Player,
    config::{colour_line, light_line},
    playlist::playlist_osd,
    projection::prepare_view_frame,
};
use crate::playback_thread::{Event, Frame};
use crate::subtitles;
use eframe::egui;
use std::time::{Duration, Instant};

/// Volume ceiling, matching VLC's slider at 200% of the recorded level.
pub(super) const VOLUME_MAX: u32 = 2_000;
/// One VLC volume tick, 5%.
pub(super) const VOLUME_STEP: u32 = 50;
/// Rates VLC exposes, in thousandths, from 0.25x to 4x.
pub(super) const RATES: [u32; 7] = [250, 500, 1_000, 1_500, 2_000, 3_000, 4_000];

/// Move a volume level by one step, staying inside the slider range.
pub(super) fn volume_step(current: u32, up: bool) -> u32 {
    if up {
        (current + VOLUME_STEP).min(VOLUME_MAX)
    } else {
        current.saturating_sub(VOLUME_STEP)
    }
}

/// Move to the neighbouring VLC rate preset, clamped at both ends. A level that
/// is not a preset (from `--rate`, say) snaps to the next one rather than
/// inventing an in-between speed.
pub(super) fn rate_step(current: u32, up: bool) -> u32 {
    if up {
        RATES
            .iter()
            .copied()
            .find(|r| *r > current)
            .unwrap_or(*RATES.last().unwrap())
    } else {
        RATES
            .iter()
            .rev()
            .copied()
            .find(|r| *r < current)
            .unwrap_or(RATES[0])
    }
}

/// VLC's fine pair: the rate one tenth faster or slower than the current one.
/// The ladder's bounds hold it, so a fine step never asks the stretching clock
/// for a speed the preset keys cannot step back from.
pub(super) fn rate_fine(current: u32, up: bool) -> u32 {
    (i64::from(current) + if up { 100 } else { -100 })
        .clamp(i64::from(RATES[0]), i64::from(*RATES.last().unwrap())) as u32
}

/// Frame cadence at a playback rate: showing each picture for shorter means the
/// timeline advances `milli`/1000 times faster.
pub(super) fn paced_period(period: Duration, milli: u32) -> Duration {
    if milli == 1_000 {
        return period;
    }
    Duration::try_from_secs_f64(period.as_secs_f64() * 1_000.0 / f64::from(milli)).unwrap_or(period)
}

/// Keep the presentation clock running across a late repaint. Resetting it
/// to the repaint time turns missed refreshes into permanent slow motion.
pub(super) fn next_frame_deadline(
    deadline: Instant,
    period: Duration,
    now: Instant,
    stepping: bool,
) -> Instant {
    if stepping { now } else { deadline + period }
}

/// The control line for a volume change, muted or not.
pub(super) fn volume_osd(milli: u32, muted: bool) -> String {
    if muted {
        return "Muted".to_owned();
    }
    format!("Volume {}%", milli.div_euclid(10))
}

/// A thousandths level spoken as a multiple with no trailing zeros: `2000` is
/// `2×`, `250` is `0.25×`.
pub(super) fn times(milli: u32) -> String {
    let text = format!("{:.2}", f64::from(milli) / 1000.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}\u{d7}")
}

/// The control line for a rate change, as a multiple rather than a percent.
pub(super) fn rate_osd(milli: u32) -> String {
    times(milli)
}

/// VLC's absolute position keys: `0` starts the file, `9` jumps to 90%.
pub(super) fn position_from_digit(digit: u8) -> Option<f32> {
    if digit > 9 {
        return None;
    }
    Some(f32::from(digit) / 10.0)
}

/// How far a frame may lead the audio clock before presentation waits. The
/// audio clock only advances in whole device buffers, and ITU-R BT.1359 puts
/// the perceptibility of audio lagging picture at roughly ten frames, so this
/// stays above the stutter a tighter gate would cause at every rate.
pub const AUDIO_SYNC_SLACK: f64 = 0.05;

pub(super) fn clock(time: Duration) -> String {
    let total = time.as_secs();
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The clock at the right end of the progress line: the whole length, or what
/// is left of it behind a minus sign, the two VLC flips between on a click.
pub(super) fn end_clock(elapsed: Duration, total: Duration, remaining: bool) -> String {
    if remaining {
        format!("\u{2212}{}", clock(total.saturating_sub(elapsed)))
    } else {
        clock(total)
    }
}

/// How many frames a second of the picture lasts for, written whole whenever it
/// comes within a twentieth of a frame of one: 41.7 ms per frame reads as `24`,
/// not `24.00`, and only a rate standing further off keeps its decimals.
pub(super) fn fps_text(period: Duration) -> String {
    let seconds = period.as_secs_f64();
    if seconds <= 0.0 {
        return "0".to_owned();
    }
    let fps = 1.0 / seconds;
    if (fps - fps.round()).abs() < 0.05 {
        format!("{}", fps.round() as u32)
    } else {
        format!("{fps:.2}")
    }
}

/// How far along the progress line the loaded pictures reach: the point already
/// shown, plus the pictures the queue is holding ahead of it, as a share of the
/// item. An item with no stated length cannot carry a share of it, so the
/// segment stops where the picture is.
///
/// The span is measured in time rather than in frames because a queue full of
/// pictures means nothing on its own: twelve of them are a fifth of a second at
/// 60 fps and a twelfth of a second at six, and what a viewer wants to know is
/// how far ahead of the stall the player has got.
pub(super) fn buffered_fraction(played: f32, frames: usize, period: Duration, total: Option<Duration>) -> f32 {
    let played = played.clamp(0.0, 1.0);
    let Some(total) = total.filter(|t| *t > Duration::ZERO) else {
        return played;
    };
    let ahead = period.saturating_mul(frames as u32).as_secs_f32() / total.as_secs_f32();
    (played + ahead).clamp(0.0, 1.0)
}

/// The buffer as the panel says it: the seconds of picture in hand and how full
/// the queue is, so a mount delivering too few bytes and a decoder too slow to
/// use them read differently on screen.
pub(super) fn buffer_text(frames: usize, depth: usize, period: Duration) -> String {
    let seconds = period.saturating_mul(frames as u32).as_secs_f64();
    format!("Buffer {seconds:.1} s · {frames}/{depth}")
}

/// How far the local copy of a slow source runs ahead of the picture, in the
/// picture's own units. The window on disk is a fixed span of bytes, so only
/// the item's bitrate can say what it is worth, and a copier that has fallen
/// behind the playhead shows as the shrinking lead it is rather than as a
/// share of the file, which a window can never finish.
pub(super) fn spool_text(ahead: u64, bytes: u64, total: Duration) -> String {
    let seconds = ahead as f64 * total.as_secs_f64() / bytes.max(1) as f64;
    format!("Spool {seconds:.1} s ahead")
}

/// The local copy of a slow source as the two ends of a share of the item, near
/// edge first. A window is a stretch of the file rather than a prefix of it -
/// it follows the reader and hands the bytes behind it back to the mount - so
/// both edges move, and the line has to show where the copy sits and not just
/// how much of it there is. An item of no stated size has no share to draw.
pub(super) fn cached_time_ranges(
    packets: &[(u64, u64, Duration, Duration)],
    bytes: &[(u64, u64)],
    total: Duration,
) -> Vec<[f32; 2]> {
    let mut intervals: Vec<_> = packets
        .iter()
        .filter(|(start, end, _, _)| bytes.iter().any(|(a, b)| a <= start && end <= b))
        .map(|(_, _, start, end)| (*start, *end))
        .collect();
    intervals.sort_unstable();
    let mut merged: Vec<(Duration, Duration)> = Vec::new();
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1.saturating_add(Duration::from_micros(1))) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
        .into_iter()
        .map(|(start, end)| {
            [
                (start.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0),
                (end.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0),
            ]
        })
        .collect()
}

#[cfg(test)]
pub(super) fn covered_span(covered: (u64, u64), total: u64) -> [f32; 2] {
    if total == 0 {
        return [0.0, 0.0];
    }
    let total = total as f32;
    [
        (covered.0 as f32 / total).clamp(0.0, 1.0),
        (covered.1 as f32 / total).clamp(0.0, 1.0),
    ]
}

/// A file's size in the units an information panel counts in: the largest one
/// that still leaves a whole number, with a decimal place only while the number
/// has a single digit, so that 9 MB and 9.4 MB do not read as one size. Bytes
/// are grouped by a thousand, the way the system names a disk's capacity.
pub(super) fn byte_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// The rate the whole item adds up to: every byte of it counted once and spread
/// over its timeline, which is what a container's length and its size state
/// together. `None` without a length to spread them over, since a rate of one of
/// the two alone would be a guess.
pub(super) fn bitrate_text(bytes: u64, duration: Duration) -> Option<String> {
    let micros = duration.as_micros();
    if micros == 0 {
        return None;
    }
    let bits = u128::from(bytes) * 8 * 1_000_000 / micros;
    Some(if bits >= 1_000_000 {
        format!("{:.1} Mb/s", bits as f64 / 1_000_000.0)
    } else {
        format!("{:.0} kb/s", bits as f64 / 1_000.0)
    })
}

/// VLC's jump ladder: very short, short, medium, long.
pub(super) const JUMP_VERY_SHORT: Duration = Duration::from_secs(3);
pub(super) const JUMP_SHORT: Duration = Duration::from_secs(10);
pub(super) const JUMP_MEDIUM: Duration = Duration::from_secs(60);
pub(super) const JUMP_LONG: Duration = Duration::from_secs(300);

/// The size of a plain arrow jump for these modifiers. Control belongs to the
/// volume here, so it is handled by the caller before this is consulted.
pub(super) fn jump_size(shift: bool, alt: bool) -> Duration {
    if shift {
        JUMP_SHORT
    } else if alt {
        JUMP_MEDIUM
    } else {
        JUMP_VERY_SHORT
    }
}

/// Shift the cue timeline by one delay step, bounded at ten seconds either
/// way as VLC bounds it.
pub(super) fn delay_step(current: i64, later: bool) -> i64 {
    let step = i64::try_from(subtitles::DELAY_STEP.as_millis()).unwrap_or(50);
    (current + if later { step } else { -step }).clamp(-10_000, 10_000)
}

/// What the A–B key does with the position it was pressed at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LoopMark {
    /// First press: A is now, and the region is not closed yet.
    MarkedA(Duration),
    /// Second press: B is now, so the picture will return to A once it gets here.
    MarkedB(Duration),
    /// Third press: both marks go away and playback carries on normally.
    Cleared,
}

/// VLC's single loop key presses A, then B, then clears the pair. A mark behind
/// the one already set would loop backwards, so it starts a new region instead.
pub(super) fn loop_press(now: Duration, a: Option<Duration>, b: Option<Duration>) -> LoopMark {
    match b {
        Some(_) => LoopMark::Cleared,
        None => match a {
            // A mark before the one already set cannot close the region, so it
            // starts a new one instead of looping backwards.
            Some(start) if now > start => LoopMark::MarkedB(now),
            _ => LoopMark::MarkedA(now),
        },
    }
}

/// Where to rewind after showing a frame, if the open A–B region has run out.
pub(super) fn loop_rewind(now: Duration, a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) if now >= b => Some(a),
        _ => None,
    }
}

pub(super) fn loop_osd(mark: LoopMark) -> String {
    match mark {
        LoopMark::MarkedA(at) => format!("Loop A {}", clock(at)),
        LoopMark::MarkedB(at) => format!("Loop A to B {}", clock(at)),
        LoopMark::Cleared => "Loop cleared".to_owned(),
    }
}

/// The audio clock as the picture sees it. A positive delay lets sound arrive
/// later, so the gate treats audio as being that much further ahead.
pub(super) fn delayed_clock(position: Duration, delay_ms: i64) -> Duration {
    let shift = Duration::from_millis(delay_ms.unsigned_abs());
    if delay_ms >= 0 {
        position.saturating_add(shift)
    } else {
        position.saturating_sub(shift)
    }
}

impl Player {
    /// Push the stored level and rate to the audio thread of the open track.
    pub(super) fn apply_audio_controls(&self) {
        let Some(audio) = &self.audio else {
            return;
        };
        audio.set_volume(if self.muted { 0 } else { self.volume_milli });
        audio.set_rate(self.rate_milli);
    }

    /// Show a control message over the picture for `OSD_AFTER`.
    pub(super) fn show_osd(&mut self, text: impl Into<String>) {
        self.osd = Some((text.into(), Instant::now()));
    }

    pub(super) fn set_volume(&mut self, milli: u32) {
        self.volume_milli = milli.min(VOLUME_MAX);
        // Turning the dial up is how a muted player comes back on in VLC.
        self.muted = false;
        self.apply_audio_controls();
        self.show_osd(volume_osd(self.volume_milli, self.muted));
    }

    pub(super) fn change_volume(&mut self, up: bool) {
        let next = volume_step(self.volume_milli, up);
        self.set_volume(next);
    }

    pub(super) fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.apply_audio_controls();
        self.show_osd(volume_osd(self.volume_milli, self.muted));
    }

    pub(super) fn set_rate(&mut self, milli: u32) {
        // Silent presentation owns a wall deadline. Retain its media phase
        // when changing speed, including lateness awaiting the next repaint.
        if !self.paused && !self.buffering && !self.seek_preview
            && self.presented.is_some() && (self.audio.is_none() || self.audio_ended)
            && self.rate_milli != 0 && milli != 0
        {
            let now = Instant::now();
            let future = self.deadline >= now;
            let span = if future { self.deadline - now } else { now - self.deadline };
            let adjusted = Duration::try_from_secs_f64(
                span.as_secs_f64() * f64::from(self.rate_milli) / f64::from(milli),
            ).unwrap_or(span);
            self.deadline = if future { now.checked_add(adjusted) }
                else { now.checked_sub(adjusted) }.unwrap_or(self.deadline);
        }
        self.rate_milli = milli;
        self.apply_audio_controls();
        self.show_osd(rate_osd(self.rate_milli));
    }

    pub(super) fn change_rate(&mut self, up: bool) {
        let next = rate_step(self.rate_milli, up);
        self.set_rate(next);
    }


    /// Open or close the panel saying what the item is. VLC keeps those facts in
    /// a window of its own; the player has no windows to spare, so they go over
    /// the picture and stay there until the key closes them.
    pub(super) fn toggle_info(&mut self) {
        if !self.item_open() {
            self.show_osd("Nothing open");
            return;
        }
        self.info = !self.info;
    }

    /// Open or close the dialog of picture settings, the one VLC keeps under
    /// Ctrl+E in its Tools menu. VLC lets its adjust filter be configured even
    /// with nothing playing, so the dialog does not wait for an item either.
    pub(super) fn toggle_effects(&mut self) {
        self.effects = !self.effects;
    }

    /// Press the loop key: the first go marks A, the next closes the region at
    /// B, and a third clears both. Rewinding is a seek, so a track that cannot
    /// seek has nothing to loop.
    pub(super) fn press_loop(&mut self) {
        if !self.seekable {
            self.show_osd("Loop needs seeking");
            return;
        }
        let Some(now) = self.presentation_clock() else {
            return;
        };
        let mark = loop_press(now, self.loop_a, self.loop_b);
        match mark {
            LoopMark::MarkedA(at) => {
                self.loop_a = Some(at);
                self.loop_b = None;
            }
            LoopMark::MarkedB(at) => self.loop_b = Some(at),
            LoopMark::Cleared => {
                self.loop_a = None;
                self.loop_b = None;
            }
        }
        self.show_osd(loop_osd(mark));
    }

    /// Convert an interval point without narrowing its total nanoseconds.
    pub(super) fn interval_time(ticks: u128, scale: u32) -> Option<Duration> {
        if scale == 0 { return None; }
        let scale = u128::from(scale);
        let Ok(seconds) = u64::try_from(ticks / scale) else { return Some(Duration::MAX); };
        Some(Duration::new(seconds, ((ticks % scale) * 1_000_000_000 / scale) as u32))
    }

    /// The active presentation clock, distinct from the progress label's
    /// future end of the current picture. Silent playback advances to its
    /// scheduled deadline; audio already owns a continuously published clock.
    pub(super) fn presentation_clock(&self) -> Option<Duration> {
        if let Some(target) = self.seek_target { return Some(target); }
        if let Some(at) = self.terminal_clock { return Some(at); }
        if self.playback.is_none() { return self.audio.as_ref().map(|audio| audio.transport_position()); }
        if let Some(audio) = &self.audio {
            let clock = delayed_clock(audio.position(), self.audio_delay_ms);
            let clock = self.audio_clock_floor.map_or(clock, |(floor, _)| clock.max(floor));
            // Device completion must not rewind an audio-paced picture to a
            // wall deadline that A/V presentation deliberately did not use.
            let completed_picture = self.picture_audio_clock && self.interval
                .and_then(|(_, end, scale)| Self::interval_time(end, scale))
                .is_some_and(|end| clock >= end);
            if !self.audio_ended || completed_picture { return Some(clock); }
        }
        let Some((start, end, scale)) = self.interval else { return Some(Duration::ZERO); };
        if self.paused { return self.paused_picture_time.or_else(|| Self::interval_time(start, scale)); }
        if self.ended { return Self::interval_time(end, scale); }
        if let Some(at) = self.buffered_picture_time { return Some(at); }
        let end = Self::interval_time(end, scale)?;
        let now = Instant::now();
        let late = now >= self.deadline;
        let span = if late { now - self.deadline } else { self.deadline - now };
        let media = span.as_nanos() * u128::from(self.rate_milli) / 1_000;
        let nanos = if late { end.as_nanos().saturating_add(media) }
            else { end.as_nanos().saturating_sub(media) };
        let at = Self::interval_time(nanos, 1_000_000_000)?;
        // A late UI callback still follows wall time when decoded pictures are
        // ready. Missing input cannot spend media beyond the visible interval.
        Some(if !self.picture_audio_clock && (self.awaiting_picture() || self.buffering) { at.min(end) } else { at })
    }

    pub(super) fn awaiting_picture(&self) -> bool {
        self.queued.is_none() && self.playback.as_ref().is_some_and(|p| p.filled() == 0)
    }

    pub(super) fn deadline_for_phase(end: Duration, phase: Duration, rate: u32, now: Instant) -> Instant {
        if end >= phase { now.checked_add(paced_period(end - phase, rate)).unwrap_or(now) }
        else { now.checked_sub(paced_period(phase - end, rate)).unwrap_or(now) }
    }

    pub(super) fn hold_picture_clock(&mut self) {
        let active_audio = self.audio.is_some() && !self.audio_ended;
        if self.playback.is_none() || self.presented.is_none() || self.ended
            || self.seek_preview || self.step > 0 || active_audio
        {
            self.buffered_picture_time = None;
            return;
        }
        if self.paused || self.buffered_picture_time.is_some() { return; }
        if self.buffering
            && let Some(end) = self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
        {
            // A completed/failed device can already be beyond the last video
            // interval. Retain its published phase instead of rewinding it.
            self.buffered_picture_time = Some(if self.picture_audio_clock {
                self.presentation_clock().unwrap_or(end).max(end)
            } else { end });
        }
    }

    /// Spend an explicit bound only once its live presentation clock reaches it.
    /// Return true when the current item changed or reached terminal state.
    pub(super) fn check_stop(&mut self) -> bool {
        let Some(stop) = self.stop_time else { return false; };
        if self.presentation_clock().is_some_and(|at| at >= stop) {
            self.terminal_clock = Some(stop);
            self.audio_clock_floor = None;
            self.buffered_picture_time = None;
            self.stop_time = None;
            self.buffering = false;
            self.seek_preview = false;
            self.seek_target = None;
            self.loading_since = None;
            if let Some(playback) = &self.playback { playback.pause(); }
            if let Some(audio) = &self.audio { audio.pause(); }
            self.continue_queue();
            return true;
        }
        false
    }

    pub(super) fn prioritize_swap(&self, time: Duration) {
        if let Some(spool) = &self.spool {
            if let Some(packet) = self
                .cache_packets
                .iter()
                .filter(|packet| packet.2 <= time)
                .max_by_key(|packet| packet.2)
            {
                spool.prioritize(packet.0);
            }
        }
    }

    pub(super) fn storage_ranges(&self, memory: bool) -> Vec<[f32; 2]> {
        let Some(total) = self.duration.filter(|total| !total.is_zero()) else { return Vec::new(); };
        let mut ranges = self.spool.as_ref().filter(|spool| spool.is_memory() == memory)
            .map_or_else(Vec::new, |spool|
                cached_time_ranges(&self.cache_packets, &spool.ranges(), total));
        if memory {
            if let Some(playback) = &self.playback { ranges.extend(playback.prepared_frame_ranges(total)); }
        }
        ranges
    }

    pub(super) fn swap_ranges(&self) -> Vec<[f32; 2]> {
        let mut ranges = self.storage_ranges(false);
        ranges.extend(self.storage_ranges(true));
        ranges
    }

    pub(super) fn toggle_pause(&mut self) {
        if self.playback.is_none() {
            self.toggle_sound();
            return;
        }
        let Some(playback) = &self.playback else {
            return;
        };
        if self.ended {
            self.restart();
            return;
        }
        let phase = self.presentation_clock();
        self.paused = !self.paused;
        if self.paused {
            self.paused_picture_time = phase;
            playback.pause();
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        } else {
            playback.play();
            if !self.buffering && let Some(audio) = &self.audio {
                audio.play();
            }
            let now = Instant::now();
            let held = self.paused_picture_time.take();
            self.deadline = if self.audio.is_none() || self.audio_ended {
                self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
                    .zip(held)
                    .and_then(|(end, at)| {
                        if end >= at { now.checked_add(paced_period(end - at, self.rate_milli)) }
                        else { now.checked_sub(paced_period(at - end, self.rate_milli)) }
                    }).unwrap_or(now)
            } else { now };
            if let Some((_, count)) = &mut self.presentation_stats {
                *count = 0;
            }
        }
    }

    /// Pause or resume an item that is only sound. The audio thread is the whole
    /// playback there, so it is the only thing to hold and release.
    pub(super) fn toggle_sound(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        if self.ended {
            self.restart();
            return;
        }
        self.paused = !self.paused;
        if self.paused {
            audio.pause();
        } else {
            audio.play();
        }
    }

    pub(super) fn restart(&mut self) {
        match (&mut self.playback, &mut self.audio) {
            (Some(playback), audio) => {
                playback.rewind();
                if let Some(audio) = audio {
                    // Hold the audio clock across restarted picture preroll.
                    // present() resumes it when the first frame is ready.
                    audio.pause();
                    audio.rewind();
                }
            }
            // With no picture there is no preroll to wait for, so the sound has
            // to be let go here rather than by the frame clock.
            (None, Some(audio)) => {
                audio.rewind();
                audio.play();
            }
            (None, None) => return,
        }
        self.audio_ended = false;
        self.queued = None;
        self.deferred_event = None;
        self.buffering = true;
        self.seek_preview = false;
        self.seek_target = None;
        self.paused_picture_time = None;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        self.ended = false;
        self.paused = false;
        self.error = None;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
    }

    /// Jump to `fraction` of the known duration; playback state is kept.
    pub(super) fn seek_fraction(&mut self, fraction: f32) {
        let Some(total) = self.duration else {
            return;
        };
        let target = total.mul_f32(fraction.clamp(0.0, 1.0));
        self.seek_time(target);
    }

    /// Move the timeline to an absolute position, clamped to the known end.
    /// Playback state is kept, and a track without a known duration can only
    /// move forward.
    pub(super) fn seek_time(&mut self, target: Duration) {
        // Explicit stop bounds park the producers while preserving the chosen
        // play/pause mode. Leaving that terminal state must restore that mode.
        let resume_after_terminal = self.ended && !self.paused;
        let target = match self.duration {
            Some(total) => target.min(total),
            None => target,
        };
        if let Some(playback) = &mut self.playback {
            playback.seek(target);
            if resume_after_terminal { playback.play(); }
        } else if self.audio.is_none() {
            return;
        }
        if let Some(audio) = &mut self.audio {
            if self.playback.is_some() {
                // The seek picture must finish preroll before sound can run.
                audio.pause();
            }
            audio.seek(target);
            // With video, present() resumes sound after the seek picture.
            if resume_after_terminal && self.playback.is_none() { audio.play(); }
        }
        self.audio_ended = false;
        self.paused_picture_time = None;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        self.prioritize_swap(target);
        // The placeholder position stands in until the decoder hands over the
        // frame at the other end of the seek; an audio thread answers at once.
        if self.playback.is_some() {
            self.seek_target = Some(target);
            self.seek_preview = true;
            self.queued = None;
            self.deferred_event = None;
            self.buffering = true;
        }
        self.ended = false;
        self.error = None;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
    }

    /// Move the timeline by a VLC jump size, from where the picture stands now.
    pub(super) fn jump(&mut self, delta: Duration, forward: bool) {
        if !self.seekable {
            return;
        }
        let Some(now) = self.presentation_clock() else {
            return;
        };
        let target = if forward {
            now.saturating_add(delta)
        } else {
            now.saturating_sub(delta)
        };
        self.seek_time(target);
        self.show_osd(clock(target));
    }


    /// Show exactly one more frame, like VLC's next-frame key. Playback is
    /// paused first; the decoder runs just long enough to hand over the frame.
    pub(super) fn step_frame(&mut self, forward: bool) {
        if self.playback.is_none() {
            if self.item_open() { self.show_osd("No picture"); }
            return;
        }
        if !self.paused {
            self.paused = true;
            if let Some(audio) = &self.audio { audio.pause(); }
        }
        if forward && self.ended {
            // A manual step at EOF holds the last picture and never advances
            // the playlist or waits for a frame that cannot arrive.
            self.step = 0;
            self.buffering = false;
            return;
        }
        self.ended = false;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        if !forward {
            // No previous picture exists yet. Show the first picture and park
            // the producer instead of leaving a paused black window.
            if self.interval.is_none() {
                if let Some(playback) = &self.playback { playback.play(); }
                self.step = 1;
                self.buffering = true;
                return;
            }
            if !self.seekable { return; }
            let target = self.interval.and_then(|(start, _, scale)| {
                if scale == 0 { return None; }
                // Last representable nanosecond strictly before this picture.
                // Using the current period fails for both CFR and VFR, and
                // subtracting a source tick before rounding loses fractional ticks.
                let nanos = start.checked_mul(1_000_000_000)?
                    .div_ceil(u128::from(scale)).saturating_sub(1);
                Some(Duration::new(u64::try_from(nanos / 1_000_000_000).ok()?, (nanos % 1_000_000_000) as u32))
            }).or_else(|| self.timeline().0.map(|end| end.saturating_sub(self.period)));
            if let Some(target) = target {
                self.seek_time(target);
                self.step = 1;
                self.show_osd(clock(target));
            }
            return;
        }
        if let Some(playback) = &self.playback { playback.play(); }
        self.step = self.step.saturating_add(1);
        self.buffering = true;
    }

    /// Check if a frame is ready for presentation based on A/V sync.
    /// Returns true if the frame can be shown now (audio clock has reached frame PTS).
    pub(super) fn frame_ready_for_sync(&self, frame: &Frame) -> bool {
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
        let audio_pos = delayed_clock(audio.position(), self.audio_delay_ms);
        let frame_seconds = pts as f64 / timescale as f64;
        let audio_seconds = audio_pos.as_secs_f64();
        audio_seconds >= frame_seconds - AUDIO_SYNC_SLACK
    }

    /// Drain audio-thread events. Audio that has run out or failed must stop
    /// gating presentation, otherwise the picture freezes at the last clock.
    pub(super) fn poll_audio(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        // Sound-only timelines follow the latest successfully completed source
        // estimate; a picture keeps its own container duration.
        if self.playback.is_none() && let Some(duration) = audio.duration() {
            self.duration = Some(duration);
        }
        // A new source owns time once it actually reaches the requested raw
        // anchor. An ended source can release the floor after a silent picture
        // takes over; until then, its shorter endpoint cannot rewind the view.
        if self.audio_clock_floor.is_some_and(|(_, at)| audio.position() >= at)
            || (self.audio_ended && !self.picture_audio_clock)
        {
            self.audio_clock_floor = None;
        }
        let generation = audio.generation();
        let mut ran_out = false;
        let mut failure = None;
        while let Some(event) = audio.poll() {
            match event {
                crate::audio_thread::AudioEvent::Started => {}
                crate::audio_thread::AudioEvent::Ended(at) => {
                    if at >= generation {
                        self.audio_ended = true;
                        ran_out = true;
                    }
                }
                crate::audio_thread::AudioEvent::Error(error) => {
                    self.audio_ended = true;
                    eprintln!("audio: {error}");
                    failure = Some(error);
                }
            }
        }
        // Once sound drains, video must continue from that last device clock.
        // A/V presentation did not consume the silent startup deadline, so
        // carrying it into silent presentation would hold the next picture.
        if (ran_out || failure.is_some()) && !self.paused && !self.seek_preview && (self.picture_audio_clock || self.audio_clock_floor.is_some()) {
            if let Some(end) = self.interval
                .and_then(|(_, end, scale)| Self::interval_time(end, scale))
            {
                let clock = delayed_clock(audio.position(), self.audio_delay_ms);
                let clock = self.audio_clock_floor.map_or(clock, |(floor, _)| clock.max(floor));
                let now = Instant::now();
                self.deadline = if end >= clock {
                    now.checked_add(paced_period(end - clock, self.rate_milli)).unwrap_or(now)
                } else {
                    now.checked_sub(paced_period(clock - end, self.rate_milli)).unwrap_or(now)
                };
            }
        }
        // Failed workers cannot answer a future Seek or advance their clock.
        // Remove that handle so seeking does not re-enable a permanently stopped
        // audio gate. Keep a healthy EOF handle: it can still seek and resume.
        if failure.is_some() {
            self.audio = None;
            self.audio_clock_floor = None;
        }
        // For an item that is only sound the track running out is the item
        // running out, so the queue carries on as it does after a last frame.
        if (ran_out || failure.is_some()) && self.playback.is_none() && !self.ended {
            // A healthy source can drain between UI callbacks, past B or a
            // hard stop. Let tick_sound resolve those bounds in transport
            // order; a paused item retains them until it resumes. Failed
            // devices cannot answer the seek and must still close normally.
            let pending_bound = failure.is_none() && self.presentation_clock().is_some_and(|at| {
                self.stop_time.is_some_and(|stop| at >= stop)
                    || loop_rewind(at, self.loop_a, self.loop_b).is_some()
            });
            if !pending_bound {
                self.stop_time = None;
                self.continue_queue();
            }
        }
    }

    /// Follow an item that is only sound. Its clock runs on the audio thread, so
    /// the screen has nothing to time: it keeps up with the position, honours the
    /// A–B region and the `--stop-time`, and asks for another look.
    pub(super) fn tick_sound(&mut self, ctx: &egui::Context) {
        if !self.item_open() || self.paused || self.ended {
            return;
        }
        if self.check_stop() {
            ctx.request_repaint();
            return;
        }
        if let Some(target) = loop_rewind(
            self.timeline().0.unwrap_or_default(),
            self.loop_a,
            self.loop_b,
        ) {
            self.seek_time(target);
        }
        ctx.request_repaint();
    }

    /// Whether the local copy of a slow source has taken the lead the first
    /// picture is meant to wait for. The wait ends by itself when the copier
    /// stops, and on a timer, because a picture shown late beats a window that
    /// never answers.
    pub(super) fn spool_primed(&self) -> bool {
        let Some(spool) = &self.spool else {
            return true;
        };
        let Some(wanted) = self.spool_lead() else {
            return true;
        };
        if spool.ready(wanted) {
            return true;
        }
        match self.spool_wait {
            Some(started) if started.elapsed() < crate::playback_spool::SPOOL_WAIT => false,
            _ => true,
        }
    }

    /// How many bytes of the item make its first span of local copy, or nothing
    /// for an item whose size or length the container never stated.
    pub(super) fn spool_lead(&self) -> Option<u64> {
        let (bytes, duration) = (self.bytes?, self.duration?);
        Some(crate::playback_spool::preroll_bytes(bytes, duration))
    }

    pub(super) fn buffer_on_stall(&mut self, now: Instant) {
        if !self.paused && !self.ended && !self.buffering
            && now.saturating_duration_since(self.deadline) >= Duration::from_millis(500)
        {
            self.buffering = true;
            self.spool_wait = Some(now);
            self.loading_since = Some(now);
            self.opening_stage = "Ожидание данных файла";
            if let Some(audio) = &self.audio { audio.pause(); }
        }
    }

    /// Take decoded frames from the thread and show the one whose time has come.
    pub(super) fn present(&mut self, ctx: &egui::Context) {
        if let Some(packets) = self.playback.as_ref().and_then(|playback| playback.cache_packets_if_changed(self.cache_packets.len())) {
            self.cache_packets = packets;
        }
        if let Some(time) = self.seek_target.or_else(|| self.timeline().0) { self.prioritize_swap(time); }
        self.poll_audio();
        self.hold_picture_clock();
        if self.playback.is_none() {
            self.tick_sound(ctx);
            return;
        }
        // A stop bound can leave prefetched pictures in the decoder queue.
        // Repaints keep the final picture until explicit navigation clears EOF.
        if self.ended {
            return;
        }
        let stepping = self.step > 0;
        if !self.paused && !self.seek_preview && !stepping && self.check_stop() {
            ctx.request_repaint();
            return;
        }
        // B can fall inside the displayed picture, or coincide with native
        // EOF. Spend it from the live clock before waiting for another frame.
        if !self.paused && !self.seek_preview && !stepping
            && let Some(target) = self.presentation_clock()
                .and_then(|at| loop_rewind(at, self.loop_a, self.loop_b))
        {
            self.seek_time(target);
            ctx.request_repaint();
            return;
        }
        let stepping_from_picture = stepping && self.presented.is_some();
        if self.paused && !self.seek_preview && !stepping {
            return;
        }
        let Some(playback) = &self.playback else {
            return;
        };
        let generation = playback.generation();
        let mut ran_out = None;
        while self.queued.is_none() {
            match self.deferred_event.take().or_else(|| playback.poll()) {
                Some(Event::Frame(frame)) if frame.generation < generation => continue,
                Some(Event::Frame(frame)) => self.queued = Some(frame),
                Some(Event::Ended(at)) => {
                    if at >= generation {
                        ran_out = Some(at);
                    }
                }
                Some(Event::Error(error)) => {
                    // A current decoder/converter refusal ends the whole
                    // transport,including any pending seek and running sound.
                    self.error = Some(error);
                    self.seek_target = None;
                    self.seek_preview = false;
                    self.buffering = false;
                    self.loading_since = None;
                    self.step = 0;
                    self.ended = true;
                    if let Some(playback) = &self.playback { playback.pause(); }
                    if let Some(audio) = &self.audio { audio.pause(); }
                    ctx.request_repaint();
                    return;
                }
                None => break,
            }
        }
        // A queue carries on by itself; the last picture of the last item is
        // where a single file stops.
        if let Some(at) = ran_out {
            if !stepping {
                if self.check_stop() {
                    ctx.request_repaint();
                    return;
                }
                let last_end = self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale));
                if last_end.zip(self.presentation_clock()).is_some_and(|(end, clock)| clock < end) {
                    // Decoding can finish while the last picture is still being
                    // displayed. Preserve the real EOF and its generation until
                    // the active media clock completes that picture's interval.
                    self.deferred_event = Some(Event::Ended(at));
                    ctx.request_repaint_after(Duration::from_millis(8));
                    return;
                }
            }
            if stepping {
                self.step = 0;
                self.buffering = false;
                self.seek_preview = false;
                self.seek_target = None;
                self.loading_since = None;
                self.paused = true;
                self.ended = true;
                if let Some(playback) = &self.playback { playback.pause(); }
                if let Some(audio) = &self.audio { audio.pause(); }
                return;
            }
            self.continue_queue();
        }
        let now = Instant::now();
        if self.buffering
            && let Some(frame) = &self.queued
        {
            // A decoded seek picture is ready to show even while future
            // read-ahead is pending. A manual step also needs only this
            // picture; keep the lead requirement for continuous startup.
            if !stepping && !self.seek_preview && !self.spool_primed() {
                ctx.request_repaint();
                return;
            }
            self.deadline = if self.paused || stepping {
                now
            } else if self.seek_preview {
                now + frame.period.min(Duration::from_millis(33))
            } else {
                now + crate::playback_thread::startup_buffer(frame.period)
            };
            self.buffering = false;
            self.loading_since = None;
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
        if self.queued.is_some()
            && let Some(phase) = self.buffered_picture_time
            && let Some(end) = self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
        {
            // Resume the held phase as soon as real data arrives; a new startup
            // lead would move the visible clock backwards before presentation.
            self.deadline = Self::deadline_for_phase(end, phase, self.rate_milli, now);
        }
        // Replace expired pictures with newer decoded ones before uploading.
        // Keep the last available picture when the decoder is starved, and
        // bound the drain so a long stall cannot monopolize the UI thread.
        if !stepping && !self.seek_preview && !self.paused {
            for _ in 0..64 {
                let Some(frame) = &self.queued else { break };
                let period = paced_period(frame.period, self.rate_milli);
                let expired = if let (Some(audio), Some((pts, scale))) = (&self.audio, frame.pts) {
                    if !self.audio_ended && scale != 0 {
                        let end = pts as f64 / f64::from(scale) + frame.period.as_secs_f64();
                        delayed_clock(audio.position(), self.audio_delay_ms).as_secs_f64() >= end
                    } else {
                        now >= self.deadline + period
                    }
                } else {
                    now >= self.deadline + period
                };
                if !expired {
                    break;
                }
                match self.playback.as_ref().unwrap().poll() {
                    Some(Event::Frame(next)) if next.generation < generation => continue,
                    Some(Event::Frame(next)) => {
                        self.queued = Some(next);
                        self.deadline += period;
                    }
                    Some(event) => {
                        // Keep EOF/errors until the final queued picture was shown.
                        self.deferred_event = Some(event);
                        break;
                    }
                    None => break,
                }
            }
        }
        if self.queued.is_some() {
            // A/V slack can make a future picture ready before the exact bound.
            // Retain the last in-range picture until the live clock catches up.
            let beyond_bound = !self.paused && !stepping && !self.seek_preview
                && self.stop_time.is_some_and(|stop| self.queued.as_ref()
                    .and_then(|frame| frame.interval)
                    .and_then(|(start, _, scale)| Self::interval_time(start, scale))
                    .is_some_and(|start| start >= stop));
            if beyond_bound {
                if self.check_stop() { ctx.request_repaint(); }
                else { ctx.request_repaint_after(Duration::from_millis(8)); }
                return;
            }
            // Presentation tolerance and A/V slack cannot expose a picture
            // at/after B while the clock is still completing the valid range.
            let outside_loop = !self.paused && !stepping && !self.seek_preview
                && self.loop_a.is_some()
                && self.loop_b.is_some_and(|b| self.queued.as_ref()
                    .and_then(|frame| frame.interval)
                    .and_then(|(start, _, scale)| Self::interval_time(start, scale))
                    .is_some_and(|start| start >= b));
            if outside_loop {
                if let Some(target) = self.presentation_clock()
                    .and_then(|at| loop_rewind(at, self.loop_a, self.loop_b))
                { self.seek_time(target); ctx.request_repaint(); }
                else { ctx.request_repaint_after(Duration::from_millis(1)); }
                return;
            }
            // Clock and vsync jitter must not postpone a ready frame by an
            // entire refresh. The media clock still advances by the exact PTS interval.
            let tolerance = (self.period / 8).min(Duration::from_millis(1));
            let frame = self.queued.as_ref().unwrap();
            // A live device clock already paces frames by their PTS. Applying
            // the wall-clock deadline too can keep moving that deadline ahead
            // forever while expired pictures are drained after startup/seek.
            let time_ready = stepping
                || (self.audio.is_some() && !self.audio_ended)
                || now + tolerance >= self.deadline;
            let av_ready = stepping || self.frame_ready_for_sync(frame);
            if time_ready && av_ready {
                let mut frame = self.queued.take().unwrap();
                self.presented_source = Some(frame.clone());
                if let Err(error) = prepare_view_frame(&mut frame, self.stereo3d, self.spherical) {
                    self.error = Some(error.to_string());
                    self.paused = true;
                    if let Some(playback) = &self.playback { playback.pause(); }
                    if let Some(audio) = &self.audio { audio.pause(); }
                    return;
                }
                // A covering picture keeps its encoded PTS, but continuous
                // playback must start at the requested position inside it.
                // Manual frame steps intentionally select the picture's start.
                let buffered_phase = self.buffered_picture_time;
                let sought_phase = if self.seek_preview && !stepping {
                    self.seek_target.and_then(|target| {
                        let (start, end, scale) = frame.interval?;
                        let start = Self::interval_time(start, scale)?;
                        let end = Self::interval_time(end, scale)?;
                        (start <= target && target <= end).then_some(target)
                    })
                } else { None };
                self.seek_preview = false;
                self.seek_target = None;
                self.upload_frame(&frame);
                if let Some((start, count)) = &mut self.presentation_stats {
                    if *count == 0 {
                        *start = Instant::now();
                        self.starved_polls = 0;
                    }
                    *count += 1;
                    if *count == 301 {
                        eprintln!(
                            "player presentation: 300 intervals in {:?}, {:.2} fps, visible={:?}, focused={:?}, starved_polls={}, video_seconds={:?}, audio_seconds={:?}",
                            start.elapsed(),
                            300.0 / start.elapsed().as_secs_f64(),
                            ctx.input(|i| i.viewport().visible()),
                            ctx.input(|i| i.viewport().focused),
                            self.starved_polls,
                            frame
                                .pts
                                .filter(|(_, scale)| *scale != 0)
                                .map(|(pts, scale)| pts as f64 / f64::from(scale)),
                            self.audio
                                .as_ref()
                                .map(|audio| audio.position().as_secs_f64())
                        );
                        self.starved_polls = 0;
                        *count = 1;
                        *start = Instant::now();
                    }
                }
                self.dimensions = frame.dimensions;
                self.period = frame.period;
                self.interval = frame.interval;
                if self.paused {
                    self.paused_picture_time = sought_phase.or_else(|| self.interval
                        .and_then(|(start, _, scale)| Self::interval_time(start, scale)));
                }
                // Advance the clock even after a late repaint; the next poll
                // discards expired pictures instead of slowing the timeline.
                let period = paced_period(frame.period, self.rate_milli);
                self.picture_audio_clock = self.audio.is_some() && !self.audio_ended;
                self.presented = Some(frame);
                if stepping_from_picture {
                    // Sound stays paused while walking pictures. Prepare it at
                    // the shown picture's start so an explicit resume agrees
                    // with the visual clock, without emitting preroll audio.
                    let target = self.interval.and_then(|(start, _, scale)| {
                        if scale == 0 { return None; }
                        let nanos = start.checked_mul(1_000_000_000)? / u128::from(scale);
                        Some(Duration::new(u64::try_from(nanos / 1_000_000_000).ok()?, (nanos % 1_000_000_000) as u32))
                    });
                    if let (Some(target), Some(audio)) = (target, &mut self.audio) {
                        if audio.position() != target { audio.seek(target); }
                    }
                }
                self.deadline = if !self.picture_audio_clock
                    && let Some(phase) = sought_phase.or(buffered_phase)
                    && let Some(end) = self.interval
                        .and_then(|(_, end, scale)| Self::interval_time(end, scale))
                {
                    Self::deadline_for_phase(end, phase, self.rate_milli, now)
                } else {
                    next_frame_deadline(self.deadline, period, now, stepping)
                };
                if let Some(phase) = buffered_phase
                    && self.interval.and_then(|(_, end, scale)| Self::interval_time(end, scale))
                        .is_none_or(|end| end > phase)
                {
                    self.buffered_picture_time = None;
                }
                if stepping {
                    self.step -= 1;
                    if self.step == 0 {
                        // A single-frame step parks the decoder again as soon as
                        // the picture is up, so pause does not keep burning CPU.
                        if let Some(playback) = &self.playback {
                            playback.pause();
                        }
                    }
                }
                // Re-check after a late presentation, keeping hard stop bounds
                // ahead of looping. Paused seeks and manual steps stay explicit.
                if !stepping && !self.check_stop() && !self.paused
                    && let Some(target) = loop_rewind(
                        self.presentation_clock().unwrap_or_default(),
                        self.loop_a,
                        self.loop_b,
                    )
                {
                    self.seek_time(target);
                }
                ctx.request_repaint_after(self.deadline.saturating_duration_since(Instant::now()));
            } else {
                ctx.request_repaint_after(self.deadline.saturating_duration_since(now));
            }
        } else if (!self.paused || self.seek_preview || stepping) && !self.ended {
            if now >= self.deadline && self.presentation_stats.is_some() {
                self.starved_polls = self.starved_polls.saturating_add(1);
            }
            // Stop the media clock during an I/O stall, while the UI keeps
            // showing progress over the last picture. Resume both clocks
            // together through the normal preroll path when frames return.
            self.buffer_on_stall(now);
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
    pub(super) fn timeline(&self) -> (Option<Duration>, Option<Duration>) {
        if !self.item_open() {
            return (None, None);
        }
        if let Some(target) = self.seek_target {
            return (Some(target), self.duration);
        }
        if self.playback.is_none() {
            // With no picture the audio thread's own clock is the timeline.
            return (
                self.audio.as_ref().map(|audio| audio.transport_position()),
                self.duration,
            );
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

    pub(super) fn subtitle(&self) -> String {
        if !self.item_open() {
            return String::new();
        }
        let head = if self.playback.is_none() {
            // Nothing to give the eye but the track the ear is getting.
            self.audio_tracks
                .get(self.audio_track)
                .map(crate::audio::AudioTrack::label)
                .unwrap_or_else(|| "sound".to_owned())
        } else {
            let [w, h] = self.dimensions;
            let decoder = if self.hardware {
                " · VideoToolbox"
            } else {
                ""
            };
            format!("{w}×{h} · {} fps{decoder}", fps_text(self.period))
        };
        let rate = if self.rate_milli == 1_000 {
            String::new()
        } else {
            format!(" · {}", rate_osd(self.rate_milli))
        };
        let level = if self.muted {
            " · muted".to_owned()
        } else if self.volume_milli != 1_000 {
            format!(" · {}%", self.volume_milli / 10)
        } else {
            String::new()
        };
        // Where this item sits in the queue, once there is more than one.
        let item = if self.queue.len() > 1 {
            format!(" · {}", playlist_osd(self.index, self.queue.len()))
        } else {
            String::new()
        };
        let sound = if self.audio_delay_ms == 0 {
            String::new()
        } else {
            format!(" · delay {} ms", self.audio_delay_ms)
        };
        format!("{head}{rate}{level}{item}{sound}")
    }

    /// What the item on screen is made of, one line for each kind of thing it
    /// carries: the picture and the codec it decodes from, the tracks the sound
    /// and the cues are taken from, how much file the timeline fills, and how
    /// many parts the container names. Every value is one the player already
    /// keeps, so opening the panel reads nothing from the file.
    pub(super) fn info_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (label, value) in [
            ("Name", &self.file_tags.title),
            ("Artist", &self.file_tags.artist),
            ("Album artist", &self.file_tags.album_artist),
            ("Album", &self.file_tags.album),
            ("Track", &self.file_tags.track),
            ("Disc", &self.file_tags.disc),
            ("Genre", &self.file_tags.genre),
            ("Publisher", &self.file_tags.publisher),
            ("Date", &self.file_tags.date),
            ("Comment", &self.file_tags.comment),
            ("Copyright", &self.file_tags.copyright),
            ("Description", &self.file_tags.description),
            ("Rating", &self.file_tags.rating),
        ] {
            if !value.is_empty() {
                lines.push(format!("{label}: {value}"));
            }
        }
        if self.playback.is_some() {
            let [w, h] = self.dimensions;
            let pixels = if self.pixel_aspect == (1, 1) {
                String::new()
            } else {
                let (x, y) = self.pixel_aspect;
                format!(" · pixels {x}:{y}")
            };
            let decoder = if self.hardware {
                " · VideoToolbox"
            } else {
                ""
            };
            lines.push(format!(
                "Video: {} · {w}×{h} · {} fps{pixels}{decoder}",
                self.video_codec,
                fps_text(self.period)
            ));
            if let Some(line) = colour_line(self.signal) {
                lines.push(line);
            }
            if let Some(line) = light_line(&self.hdr) {
                lines.push(line);
            }
            if let Some(line) = &self.grade_line {
                lines.push(line.clone());
            }
        }
        if let Some(playback) = &self.playback {
            lines.push(buffer_text(
                playback.filled(),
                playback.depth(),
                self.period,
            ));
        }
        if let (Some(spool), Some(bytes), Some(total)) = (&self.spool, self.bytes, self.duration) {
            lines.push(spool_text(spool.ahead(), bytes, total));
        }
        if let Some(track) = self.audio_tracks.get(self.audio_track) {
            lines.push(format!(
                "Sound: {} · {} · {}",
                playlist_osd(self.audio_track, self.audio_tracks.len()),
                self.sound_codec,
                track.label()
            ));
        }
        if let Some(source) = self.subtitle_sources.get(self.subtitle_source) {
            lines.push(format!(
                "Subtitles: {} · {}",
                playlist_osd(self.subtitle_source, self.subtitle_sources.len()),
                source.label
            ));
        }
        if let Some(bytes) = self.bytes {
            let mut line = format!("File: {}", byte_size(bytes));
            if let Some(total) = self.duration {
                line += &format!(" · {}", clock(total));
                if let Some(rate) = bitrate_text(bytes, total) {
                    line += &format!(" · {rate}");
                }
            }
            lines.push(line);
        }
        if !self.chapters.is_empty() {
            lines.push(format!("Chapters: {}", self.chapters.len()));
        }
        lines
    }

    pub(super) fn controls_visible(&self) -> bool {
        !self.item_open()
            || self.info
            || self.effects
            || self.paused
            || self.ended
            || self.error.is_some()
            || self.activity.elapsed() < HIDE_AFTER
    }
}
