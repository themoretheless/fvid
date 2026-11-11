use super::{
    NextFile, Player,
    session::prepare_source,
};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What happens when the picture reaches the end of the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Repeat {
    /// Stop on the last item.
    #[default]
    Off,
    /// Wrap around to the first item.
    All,
    /// Start the current item again.
    One,
}

/// The item to start once the current one runs out, or `None` to stop.
pub(super) fn advance(index: usize, len: usize, repeat: Repeat) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if index + 1 < len {
        return Some(index + 1);
    }
    (repeat == Repeat::All).then_some(0)
}

/// The item before the current one, wrapping only when the list repeats.
pub(super) fn retreat(index: usize, len: usize, repeat: Repeat) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if index > 0 {
        return Some(index - 1);
    }
    (repeat == Repeat::All).then_some(len - 1)
}

/// The next audio track to open, wrapping within the tracks a decoder exists
/// for. The caller only asks when there is more than one.
pub(super) fn track_step(current: usize, len: usize, forward: bool) -> usize {
    (current + if forward { 1 } else { len - 1 }) % len
}

/// One draw of a splitmix64 generator: the value and the state advanced past
/// it. Cheap, and random enough for an order nobody can bet on.
pub(super) fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Deal one shuffled cycle of the list: every index but the one on screen, in
/// an order drawn by a Fisher-Yates walk over the draws of `seed`, which is
/// left advanced for the next dealing. The last item of the list is the next
/// draw, so a walk over the cycle plays each of them once before the list
/// ends or deals again.
pub(super) fn deal_cycle(len: usize, current: usize, seed: &mut u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).filter(|&i| i != current).collect();
    for i in 1..order.len() {
        let draw = (splitmix(seed) >> 32) as usize;
        order.swap(i, draw % (i + 1));
    }
    order
}

pub(super) fn cycle_repeat(repeat: Repeat) -> Repeat {
    match repeat {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    }
}

pub(super) fn repeat_osd(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Off => "Repeat off",
        Repeat::All => "Repeat all",
        Repeat::One => "Repeat one",
    }
}

/// Position in the list, one-based, as VLC writes it.
pub(super) fn playlist_osd(index: usize, len: usize) -> String {
    format!("{}/{len}", index + 1)
}

/// Expand playlist files into the videos they list; anything else is already a
/// path, including one that does not exist, which the caller reports on open. A
/// list with nothing usable in it is handed back untouched so opening it says
/// why rather than leaving the window blank.
pub(super) fn playable_path(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("y4m" | "mp4" | "mov" | "m4v" | "m4a" | "webm" | "mkv" | "mka" | "avi" |
            "mp3" | "mp2" | "mp1" | "flac" | "aac" | "ac3" | "wav" | "w64" | "aif" | "aiff" |
            "au" | "snd" | "ogg" | "wv" | "wvc" | "mid" | "xm"))
}

pub(super) fn expand_inputs(inputs: &[PathBuf]) -> Vec<PathBuf> {
    const MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        // Known media paths need no synchronous filesystem query on the UI
        // thread. Their worker reports missing files and slow mounts.
        if playable_path(input) { out.push(input.clone()); continue; }
        if input.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(input).into_iter().flatten()
                .filter_map(Result::ok).map(|entry| entry.path())
                .filter(|path| path.is_file() && playable_path(path)).collect();
            files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            out.extend(files);
            continue;
        }
        let listing = matches!(
            input
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .as_deref(),
            Some("m3u") | Some("m3u8") | Some("pls")
        );
        match listing {
            true => {
                if let Ok(file) = File::open(input) {
                    let mut text = String::new();
                    if file
                        .take((MAX_PLAYLIST_BYTES + 1) as u64)
                        .read_to_string(&mut text)
                        .is_ok()
                        && text.len() <= MAX_PLAYLIST_BYTES
                    {
                        let base = input.parent().unwrap_or(Path::new(""));
                        out.extend(parse_playlist(&text, base));
                    }
                }
            }
            false => out.push(input.clone()),
        }
    }
    if out.is_empty() && !inputs.is_empty() {
        return inputs.to_vec();
    }
    out
}

/// Read the video paths out of an M3U (`#EXTM3U`, `#EXTINF:` notes, then one
/// path per line) or a PLS (`FileN=path`). Entries are resolved against the
/// list's own directory, and ones that are not on disk are dropped rather than
/// failing the whole list. Network entries are not something the native player
/// can open, so they are skipped too.
pub(super) fn parse_playlist(text: &str, base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let entry = match line.split_once('=') {
            Some((key, value)) => {
                if key
                    .trim()
                    .trim_end_matches(char::is_numeric)
                    .eq_ignore_ascii_case("file")
                {
                    value
                } else {
                    continue;
                }
            }
            None => {
                if line.is_empty()
                    || line.starts_with(['#', ';'])
                    || (line.starts_with('[') && line.ends_with(']'))
                {
                    continue;
                }
                line
            }
        };
        let entry = entry.trim();
        let entry = entry
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| entry.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(entry)
            .trim();
        if entry.is_empty() || entry.contains("://") {
            continue;
        }
        let path = if Path::new(entry).is_absolute() {
            PathBuf::from(entry)
        } else {
            base.join(entry)
        };
        if path.is_file() {
            out.push(path);
        }
    }
    out
}


impl Player {
    /// Replace the queue with `paths`, open the first of them, and remember
    /// where in the list the picture came from.
    pub(super) fn clear_prefetch(&mut self) {
        if let Some(next) = self.next_file.take() {
            next.cancelled.store(true, std::sync::atomic::Ordering::Release);
            // Dropping a completed spool can join its copier.
            std::thread::spawn(move || drop(next));
        }
    }

    pub(super) fn prefetch_next(&mut self) {
        if self.memory_check.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) {
            self.memory_check = Some(Instant::now());
            self.cache_pressure = crate::playback_spool::available_memory().is_some_and(|bytes| bytes < 768 << 20);
            if self.cache_pressure {
                self.clear_prefetch();
                crate::playback_spool::release_startup_buffer();
            }
        }
        if self.cache_pressure { return; }
        if self.opening.is_some() || !self.item_open() || self.ended {
            return;
        }
        let target = if self.repeat == Repeat::One {
            Some(self.index)
        } else if self.shuffle {
            self.upcoming.last().copied().or_else(|| {
                if self.repeat != Repeat::All || self.queue.is_empty() { return None; }
                let mut seed = self.shuffle_seed;
                deal_cycle(self.queue.len(), self.index, &mut seed).pop().or(Some(self.index))
            })
        } else {
            advance(self.index, self.queue.len(), self.repeat)
        };
        let path = target.and_then(|index| self.queue.get(index)).cloned();
        if self.next_file.as_ref().map(|next| &next.path) == path.as_ref() {
            return;
        }
        self.clear_prefetch();
        let Some(path) = path else { return; };
        if path.to_str().is_some_and(fvid_media::play_math::is_playback_url) { return; }
        let (send, source) = std::sync::mpsc::channel();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.next_file = Some(NextFile { path: path.clone(), source, cancelled: cancelled.clone() });
        std::thread::spawn(move || {
            let _ = send.send(prepare_source(&path, &cancelled));
        });
    }

    pub(super) fn open_queue(&mut self, paths: Vec<PathBuf>) {
        self.clear_prefetch();
        self.queue = expand_inputs(&paths);
        self.index = 0;
        if self.shuffle {
            // The old cycle belonged to the old list; a shuffled player gets
            // a freshly dealt one.
            self.deal_cycle();
        }
        let Some(path) = self.queue.first().cloned() else {
            return;
        };
        self.try_open(path);
    }

    /// Show item `index` of the queue, leaving the list itself alone. A file
    /// that will not open is reported and the picture stays where it was.
    pub(super) fn play_index(&mut self, index: usize) {
        let Some(path) = self.queue.get(index).cloned() else {
            return;
        };
        if self.try_open(path) {
            self.index = index;
        }
    }

    /// Turn Random on or off. Turning it on keeps the item on screen and lines
    /// the rest of the list up behind it in a drawn order, the way VLC's
    /// shuffle places the current item in the new permutation and plays on
    /// from there; turning it off forgets the cycle and lets the plain order
    /// resume from wherever the walk stopped.
    pub(super) fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.start_shuffle();
            self.show_osd("Shuffle on");
        } else {
            self.upcoming.clear();
            self.walked.clear();
            self.show_osd("Shuffle off");
        }
    }

    /// Begin a fresh shuffled cycle from the item on screen, mixing the clock
    /// into the seed so that the same list walks a different way each time it
    /// is started out with `--random` or turned on by the key.
    pub(super) fn start_shuffle(&mut self) {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        self.shuffle_seed ^= u64::from(since.subsec_nanos()) ^ since.as_secs();
        self.deal_cycle();
    }

    /// Line the items the list holds besides the one on screen up in a drawn
    /// order and start the walk fresh from the item on screen.
    pub(super) fn deal_cycle(&mut self) {
        self.upcoming = deal_cycle(self.queue.len(), self.index, &mut self.shuffle_seed);
        self.walked = vec![self.index];
    }

    /// The next item of a shuffled walk: the last draw of what the cycle has
    /// left. A cycle that has run out deals the list anew when it repeats, as
    /// VLC does, and ends it otherwise; a list of one has nothing to draw, so
    /// repeating it is all there is.
    pub(super) fn shuffle_forward(&mut self) -> Option<usize> {
        if let Some(next) = self.upcoming.pop() {
            self.walked.push(next);
            return Some(next);
        }
        if self.repeat != Repeat::All || self.queue.is_empty() {
            return None;
        }
        self.deal_cycle();
        match self.upcoming.pop() {
            Some(next) => {
                self.walked.push(next);
                Some(next)
            }
            None => Some(self.index),
        }
    }

    /// The previous item of a shuffled walk: back along the path it walked,
    /// leaving the item stepped past open for the next draw again. A cycle at
    /// its first item has nothing behind it.
    pub(super) fn shuffle_back(&mut self) -> Option<usize> {
        if self.walked.len() < 2 {
            return None;
        }
        self.upcoming.push(self.index);
        self.walked.pop();
        self.walked.last().copied()
    }

    /// Step through the list with the next and previous keys; with Random on,
    /// the drawn cycle takes their walk instead of the numeric order.
    pub(super) fn step_queue(&mut self, back: bool) {
        let len = self.queue.len();
        let target = if self.shuffle {
            if back {
                self.shuffle_back()
            } else {
                self.shuffle_forward()
            }
        } else if back {
            retreat(self.index, len, self.repeat)
        } else {
            advance(self.index, len, self.repeat)
        };
        match target {
            Some(target) => self.play_index(target),
            None => self.show_osd(if back {
                "First item"
            } else {
                "End of playlist"
            }),
        }
    }

    /// Carry playback into the next item when the picture runs out, the way a
    /// playlist continues by itself; a shuffled list continues along its cycle.
    pub(super) fn should_quit(&self) -> bool {
        self.quit_at_end && self.ended && self.error.is_none()
            && self.opening.is_none() && !self.queue.is_empty()
    }

    pub(super) fn continue_queue(&mut self) {
        let target = match self.repeat {
            Repeat::One => Some(self.index),
            _ if self.shuffle => self.shuffle_forward(),
            _ => advance(self.index, self.queue.len(), self.repeat),
        };
        match target {
            Some(target) => self.play_index(target),
            None => self.ended = true,
        }
    }

}
