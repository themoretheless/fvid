//! Serves a file from a local copy of itself so a mount slower than the item's
//! own rate cannot put its latency on the frame clock.
//!
//! A cloud drive answers in bursts of a few megabytes per second while a 4K
//! item asks for tens of them, and every read the decoder makes sits in front
//! of a picture it is waiting for. Here a background thread pulls the file into
//! a local spool as fast as the source gives it up, and the reader consumes the
//! spool instead: a read the spool already covers is a local-disk read, and the
//! reader only meets the source's latency when the spool has run out ahead of
//! it. The copier stays a span of the item ahead of the reader rather than
//! copying everything, so the disk it uses is the buffer, not the library.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Bytes the copier and a beyond-spool read move at a time, as an offset into
/// the item: small enough that one slow read of a cloud drive is a fraction of
/// a second rather than the several seconds a megabyte-hungry request can cost.
const CHUNK: u64 = 1 << 20;

/// The same block as a buffer length. Memory is indexed in machine words while
/// the item is counted in 64-bit offsets, and only the handful of places where
/// the two meet need this: every one of them is a `Vec` or a slice bound.
const CHUNK_BYTES: usize = CHUNK as usize;

/// How long a reader will wait for the copier to reach the next block off the
/// source rather than fetching it itself. Past this the mount is being answered
/// twice for the same bytes, which is the thing to avoid; short of it, waiting
/// is just the buffering the reader came for.
const CHUNK_WAIT: Duration = Duration::from_secs(2);

/// How much of the source a reader buffers in front of itself, as a buffer
/// capacity: small reads are what turn a mount's latency into a stutter - eight
/// kilobytes at a time asks the question once per frame and a megabyte asks it a
/// hundred times less.
pub const READ_AHEAD: usize = CHUNK_BYTES;

/// How far ahead of the reader the copier runs, in bytes: the most a window
/// costs on disk while an item plays, and the span a source slower than its
/// item can still be caught up in. An item shorter than this gets a window of
/// its own length rather than one it cannot fill.
pub const SPOOL_LEAD: u64 = 256 << 20;

/// The shortest preroll a caller waits for, whatever the item: five seconds is
/// more than a whole group of pictures needs and little enough that a source
/// which cannot stream the item at all is known about at once.
pub const SPOOL_PREROLL: Duration = Duration::from_secs(5);

/// The longest preroll, however large the item: half a minute is as long as a
/// viewer will stand behind a window that shows nothing.
pub const SPOOL_PREROLL_CAP: Duration = Duration::from_secs(30);

/// The share of an item the first picture waits for, where that is shorter than
/// the cap. A tenth of a three-minute clip is eighteen seconds of it; a tenth
/// of an hour is six minutes of black window, which no amount of buffering is
/// worth, so the cap answers for long items and the share for short ones.
pub const SPOOL_SHARE: f64 = 0.10;

/// How long the first picture will wait for that preroll before it is shown
/// with whatever lead a very slow source has managed.
pub const SPOOL_WAIT: Duration = Duration::from_secs(30);

/// A source at or above this many megabytes per second is served directly. It
/// can answer a frame period's bytes within the frame period, which is the only
/// promise a buffer can keep; below it a spindle or a cloud drive answers in
/// bursts a decoder cannot wait inside, so the spool runs.
pub const SPOOL_UNDER: f64 = 60.0;

/// The shortest item worth asking how fast it is. A rate is measured over one
/// read, and a source that cannot fill it is being timed on the syscall rather
/// than on the mount: a few hundred bytes off a small local file came out under
/// the floor often enough to spool files that one request already covers.
pub const SPOOL_ABOVE: u64 = 1 << 20;

/// How many bytes of an item a span of buffer costs at the item's own rate. An
/// item of no stated length, or no length at all, buffers whole.
pub fn lead_bytes(item_bytes: u64, duration: Duration, lead: Duration) -> u64 {
    if duration.is_zero() {
        return item_bytes;
    }
    let lead = lead.min(duration);
    (item_bytes as u128 * lead.as_nanos() / duration.as_nanos().max(1)) as u64
}

/// How much of an item the first picture waits for: the lesser of a tenth of it
/// and half a minute of its picture, never less than five seconds of picture,
/// and never more than one window or the item itself. Both ends matter because
/// the same span of time costs wildly different amounts of bytes - five seconds
/// of a phone clip is a megabyte and of a 4K record six times the whole clip -
/// and a fixed wait is wrong at one end or the other.
pub fn preroll_bytes(item_bytes: u64, duration: Duration) -> u64 {
    let share = (item_bytes as f64 * SPOOL_SHARE) as u64;
    let longest = lead_bytes(item_bytes, duration, SPOOL_PREROLL_CAP);
    let shortest = lead_bytes(item_bytes, duration, SPOOL_PREROLL);
    share
        .min(longest)
        .max(shortest)
        .min(item_bytes)
        .min(SPOOL_LEAD)
}

/// Bytes of the local copy that lie in front of a reader standing at `read_at`,
/// for a window holding `[base, end)`: none at all while the reader is behind
/// the window, because the run of bytes it can take without asking the source
/// starts where the copy starts. The window follows the reader and the reader
/// can leave it far behind - a file whose index sits at the tail is opened by
/// seeking there, and the window goes with it - and counting the far edge from
/// the reader then reports the whole gap as a lead that has been fetched.
pub fn lead_ahead(read_at: u64, base: u64, end: u64) -> u64 {
    if read_at >= base {
        end.saturating_sub(read_at)
    } else {
        0
    }
}

/// Where a speed probe reads: just past the header, because that is where the
/// item's own bytes begin and playback goes first. A drive that has cached the
/// header says nothing about the body behind it, and the middle of a large item
/// is territory the reader has not asked for yet.
fn probe_at(size: u64) -> u64 {
    (1u64 << 23).min(size / 2)
}

/// How fast a source really is: the megabytes per second one cold read of the
/// item's body delivers, or nothing when the file cannot be read at all.
pub fn source_rate(path: &Path, probe: u64) -> Option<f64> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size == 0 {
        return None;
    }
    file.seek(SeekFrom::Start(probe_at(size))).ok()?;
    let mut buffer = vec![0u8; probe as usize];
    let started = Instant::now();
    let read = file.read(&mut buffer).ok()? as u64;
    let elapsed = started.elapsed();
    if read == 0 || elapsed.is_zero() {
        return None;
    }
    Some(read as f64 / elapsed.as_secs_f64() / 1e6)
}

/// The span of the item this spool holds locally, as `[base, end)`: a window
/// that follows the reader rather than a copy of the file from byte zero.
struct Window {
    /// The item offset of the first byte the spool file can answer for.
    base: u64,
    /// Upper bound of cached bytes; there may be gaps between blocks.
    end: u64,
    /// Where the reader stands, so the copier runs ahead of playback rather
    /// than ahead of nothing.
    read_at: u64,
    play_at: Option<u64>,
    blocks: BTreeMap<u64, (u64, u64)>, // source offset -> (disk slot, byte count)
}

struct Shared {
    /// Bytes held locally, mirrored from the window for whoever only watches:
    /// the user's line and a caller waiting for a lead.
    held: Arc<AtomicU64>,
    /// Bytes held locally ahead of the reader, so a lead can be asked for and
    /// shown in picture rather than in disk: the window's whole span says how
    /// much room the copier has filled, this says how much of it is still
    /// unbitten.
    slack: Arc<AtomicU64>,
    /// Bounding offsets for compatibility; exact coverage comes from `ranges`.
    from: Arc<AtomicU64>,
    to: Arc<AtomicU64>,
    /// The copier has stopped: end of file, an unmet read, or shutdown.
    finished: Arc<AtomicBool>,
    /// The offset the copier has a block in flight for, or `u64::MAX` while it
    /// works on nothing. A reader at that offset waits for the block instead of
    /// fetching it a second time from the same slow source.
    claim: AtomicU64,
    /// Blocks the temporary file is made of, enough for the window and a block
    /// to spare: the copy is written round these, so a lead costs the ring's
    /// size and never the item's. A file cannot be cut from the front, so the
    /// oldest block is given up to the newest one that overwrites it.
    slots: u64,
    stop: AtomicBool,
    terminated: AtomicBool,
    window: Mutex<Window>,
    wake: Condvar,
}

impl Shared {
    /// Publishes where the window stands for the watchers, from the window the
    /// caller already holds locked: a mirror taken outside the lock can be out
    /// of date by the time it lands, and a preroll would wait on a lead that
    /// has already been bitten.
    fn mirror(&self, window: &Window) {
        let held = window.blocks.values().map(|(_, bytes)| bytes).sum();
        self.held.store(held, Ordering::Release);
        let position = window.play_at.unwrap_or(window.read_at);
        let mut end = position;
        loop {
            let at = end / CHUNK * CHUNK;
            let Some((_, bytes)) = window.blocks.get(&at) else {
                break;
            };
            let next = at + bytes;
            if next <= end {
                break;
            }
            end = next;
        }
        self.slack.store(end - position, Ordering::Release);
        self.from.store(window.base, Ordering::Release);
        self.to.store(window.end, Ordering::Release);
    }

    /// Records the reader's position and wakes the copier: the reader has just
    /// told it where to run ahead from.
    fn arrive(&self, at: u64) {
        let mut window = self.window.lock().unwrap_or_else(|p| p.into_inner());
        window.read_at = at;
        self.mirror(&window);
        self.wake.notify_all();
    }

    fn pause(&self, span: Duration) {
        let window = self.window.lock().unwrap_or_else(|p| p.into_inner());
        let _ = self
            .wake
            .wait_timeout(window, span)
            .unwrap_or_else(|p| p.into_inner());
    }
}

/// A reader of the spool's own progress, kept by whoever has to wait for it:
/// the `Spool` itself is moved into the decoder's reader and out of reach.
#[derive(Clone)]
pub struct SpoolHandle {
    shared: Arc<Shared>,
    held: Arc<AtomicU64>,
    slack: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
}

impl SpoolHandle {
    /// Bytes of the item the spool holds locally, in a window around wherever
    /// the reader has got to: the disk this spool costs.
    pub fn copied(&self) -> u64 {
        self.held.load(Ordering::Acquire)
    }

    /// Bytes of the item held locally ahead of the reader, which is the span of
    /// picture a source's next stall cannot reach: what a preroll waits for and
    /// what a user is shown. It is the window minus what has been bitten, so a
    /// copier that has fallen behind the playhead shows up here first.
    pub fn ahead(&self) -> u64 {
        self.slack.load(Ordering::Acquire)
    }

    /// Bounding offsets only. Use `ranges()` to avoid inventing cached gaps.
    pub fn covered(&self) -> (u64, u64) {
        let window = self.shared.window.lock().unwrap_or_else(|p| p.into_inner());
        (window.base, window.end)
    }

    /// Anchor prefetch to media playback, independently of metadata seeks.
    pub fn prioritize(&self, offset: u64) {
        let mut window = self.shared.window.lock().unwrap_or_else(|p| p.into_inner());
        window.play_at = Some(offset);
        self.shared.finished.store(
            self.shared.terminated.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.shared.mirror(&window);
        self.shared.wake.notify_all();
    }

    /// Exact cached byte ranges, including disjoint future sections.
    pub fn ranges(&self) -> Vec<(u64, u64)> {
        let window = self.shared.window.lock().unwrap_or_else(|p| p.into_inner());
        let mut ranges: Vec<(u64, u64)> = Vec::new();
        for (&at, &(_, bytes)) in &window.blocks {
            if let Some(last) = ranges.last_mut().filter(|last| last.1 == at) {
                last.1 += bytes;
            } else {
                ranges.push((at, at + bytes));
            }
        }
        ranges
    }

    /// Whether the copier has stopped, so a wait for a lead this source cannot
    /// deliver ends rather than hangs.
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Has the lead arrived ahead of the reader, or is it never going to.
    pub fn ready(&self, wanted: u64) -> bool {
        self.ahead() >= wanted || self.finished()
    }
}

/// A readable, seekable copy of a file, kept in the system's temporary
/// directory for as long as it is open.
pub struct Spool {
    shared: Arc<Shared>,
    spool: File,
    source: File,
    size: u64,
    /// The window this spool runs in: what was asked for, less what the item
    /// cannot fill.
    lead: u64,
    path: PathBuf,
    position: u64,
    staging: Vec<u8>,
    /// What the copier cannot yet give is staged from the source a block at a
    /// time, and `staged_at` is the item offset the first staged byte belongs
    /// to: the copier can overtake a reader mid-stage, and bytes kept for one
    /// offset must not be handed out at another.
    staged: usize,
    staged_at: u64,
    copier: Option<JoinHandle<()>>,
}

impl Spool {
    /// Copies `path` into a temporary file in the background, staying `lead`
    /// bytes ahead of wherever the reader has got to.
    pub fn open(path: &Path, lead: u64) -> io::Result<Self> {
        let source = File::open(path)?;
        let size = source.metadata()?.len();
        // A window longer than the item is disk the copier never fills: it has
        // the whole item locally as soon as it reaches the end, which is all a
        // short item's reader was asking for.
        let lead = lead.min(size);
        let spool_path = temporary(path, size);
        let spool = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&spool_path)?;
        // The ring is a whole number of blocks, long enough for a lead, the
        // block being written and one to spare, and it is sized up front so no
        // write can ever extend it past that.
        let slots = lead.div_ceil(CHUNK) + 3;
        spool.set_len(slots * CHUNK)?;
        let shared = Arc::new(Shared {
            held: Arc::new(AtomicU64::new(0)),
            slack: Arc::new(AtomicU64::new(0)),
            from: Arc::new(AtomicU64::new(0)),
            to: Arc::new(AtomicU64::new(0)),
            finished: Arc::new(AtomicBool::new(false)),
            claim: AtomicU64::new(u64::MAX),
            slots,
            stop: AtomicBool::new(false),
            terminated: AtomicBool::new(false),
            window: Mutex::new(Window {
                base: 0,
                end: 0,
                read_at: 0,
                play_at: None,
                blocks: BTreeMap::new(),
            }),
            wake: Condvar::new(),
        });
        // Both sides of the spool open the two files by name rather than
        // cloning these handles: a duplicated descriptor shares one seek
        // offset, so a reader moving to the end of an item would drag the
        // copier's next write along with it.
        let copier = {
            let shared = shared.clone();
            let from = File::open(path)?;
            let mut to = OpenOptions::new().write(true).open(&spool_path)?;
            thread::Builder::new()
                .name("fvid-spool".into())
                .spawn(move || copy(from, &mut to, size, lead, &shared))
                .map_err(|error| io::Error::other(error.to_string()))?
        };
        Ok(Self {
            shared,
            spool,
            source,
            size,
            lead,
            path: spool_path,
            position: 0,
            staging: vec![0u8; CHUNK_BYTES],
            staged: 0,
            staged_at: 0,
            copier: Some(copier),
        })
    }

    /// What a caller watches while this reader is busy with the item.
    pub fn handle(&self) -> SpoolHandle {
        SpoolHandle {
            shared: self.shared.clone(),
            held: self.shared.held.clone(),
            slack: self.shared.slack.clone(),
            finished: self.shared.finished.clone(),
        }
    }

    /// Bytes of the item the spool already holds.
    pub fn copied(&self) -> u64 {
        self.shared.held.load(Ordering::Acquire)
    }

    /// How far ahead of the reader this window runs, so what the spool costs on
    /// disk can be read back rather than assumed from what was asked for.
    pub fn lead(&self) -> u64 {
        self.lead
    }

    /// Whether the copier has stopped, so a caller can stop waiting for a lead
    /// this source will never deliver.
    pub fn exhausted(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }
}

/// Extends the window one block at a time, pausing while it is `lead` ahead of
/// the reader and following the reader when it moves further away than that.
fn copy(mut source: File, spool: &mut File, size: u64, lead: u64, shared: &Arc<Shared>) {
    let mut buffer = vec![0u8; CHUNK_BYTES];
    while !shared.stop.load(Ordering::Relaxed) {
        let at = {
            let window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
            let position = window.play_at.unwrap_or(window.read_at);
            let mut at = position / CHUNK * CHUNK;
            let limit = position.saturating_add(lead).min(size);
            while at < limit && window.blocks.contains_key(&at) {
                at += CHUNK;
            }
            (at < limit).then_some(at)
        };
        let Some(at) = at else {
            // EOF is an idle state, not termination: an index read at the tail
            // must not prevent the copier serving playback back at the start.
            let window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
            shared.finished.store(
                window.play_at.unwrap_or(window.read_at) >= size || window.end >= size,
                Ordering::Release,
            );
            drop(window);
            shared.pause(Duration::from_millis(20));
            continue;
        };
        shared.finished.store(false, Ordering::Release);
        shared.claim.store(at, Ordering::Release);
        let bytes = CHUNK.min(size - at);
        if source
            .seek(SeekFrom::Start(at))
            .and_then(|_| source.read_exact(&mut buffer[..bytes as usize]))
            .is_err()
        {
            break;
        }
        let mut window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
        // Allocate a free slot; when full, evict played blocks before any data
        // ahead. Among future blocks keep the closest ones to playback.
        let slot = if window.blocks.len() < (lead.div_ceil(CHUNK) + 1) as usize {
            (0..shared.slots)
                .find(|slot| !window.blocks.values().any(|(used, _)| used == slot))
                .unwrap()
        } else {
            let victim = window
                .blocks
                .keys()
                .copied()
                .find(|offset| {
                    offset.saturating_add(CHUNK) <= window.play_at.unwrap_or(window.read_at)
                })
                .unwrap_or_else(|| *window.blocks.keys().next_back().unwrap());
            window.blocks.remove(&victim).unwrap().0
        };
        if spool
            .seek(SeekFrom::Start(slot * CHUNK))
            .and_then(|_| spool.write_all(&buffer[..bytes as usize]))
            .is_err()
        {
            break;
        }
        window.blocks.insert(at, (slot, bytes));
        window.base = *window.blocks.keys().next().unwrap();
        window.end = window
            .blocks
            .iter()
            .next_back()
            .map(|(at, (_, bytes))| at + bytes)
            .unwrap();
        shared.mirror(&window);
        shared.claim.store(u64::MAX, Ordering::Release);
        shared.wake.notify_all();
    }
    shared.claim.store(u64::MAX, Ordering::Release);
    shared.terminated.store(true, Ordering::Release);
    shared.finished.store(true, Ordering::Release);
    shared.wake.notify_all();
}

impl Read for Spool {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // The copier has the block this byte is in on its way: let it land
        // instead of asking the source for the same bytes twice over, which is
        // the mount's latency back and twice the traffic to pay for it. Anywhere
        // the copier is not heading, the reader is served directly.
        let claim = self.shared.claim.load(Ordering::Acquire);
        if self.position >= claim
            && self.position < claim.saturating_add(CHUNK)
            && !self.exhausted()
        {
            let until = Instant::now() + CHUNK_WAIT;
            while !self
                .shared
                .window
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .blocks
                .contains_key(&(self.position / CHUNK * CHUNK))
                && Instant::now() < until
                && !self.exhausted()
            {
                self.shared.pause(Duration::from_millis(10));
            }
        }
        // A local read is taken with the window held: the copier cuts the file
        // back as the window moves forward, and what it has not handed over yet
        // must not go missing under a read standing in the middle of it.
        let mut window = self.shared.window.lock().unwrap_or_else(|p| p.into_inner());
        let block = self.position / CHUNK * CHUNK;
        if let Some(&(slot, bytes)) = window.blocks.get(&block) {
            let inside = self.position - block;
            if inside < bytes {
                let take = (bytes - inside).min(out.len() as u64) as usize;
                self.spool.seek(SeekFrom::Start(slot * CHUNK + inside))?;
                let done = self.spool.read(&mut out[..take])?;
                if done > 0 {
                    self.position += done as u64;
                    window.read_at = self.position;
                    self.shared.mirror(&window);
                    self.shared.wake.notify_all();
                    return Ok(done);
                }
            }
        }
        drop(window);
        if self.staged == 0 || self.staged_at != self.position {
            self.source.seek(SeekFrom::Start(self.position))?;
            self.staged = self.source.read(&mut self.staging)?;
            self.staged_at = self.position;
            if self.staged == 0 {
                return Ok(0);
            }
        }
        let take = self.staged.min(out.len());
        out[..take].copy_from_slice(&self.staging[..take]);
        self.staging.copy_within(take..self.staged, 0);
        self.staged -= take;
        let taken = take as u64;
        self.staged_at += taken;
        self.advance(taken);
        Ok(take)
    }
}

impl Seek for Spool {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let base = match from {
            SeekFrom::Start(at) => at,
            SeekFrom::End(back) => self.size.saturating_sub(back.unsigned_abs()),
            SeekFrom::Current(back) => {
                if back < 0 {
                    self.position.saturating_sub(back.unsigned_abs())
                } else {
                    self.position.saturating_add(back as u64)
                }
            }
        };
        self.position = base;
        self.staged = 0;
        self.shared.finished.store(
            self.shared.terminated.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.shared.arrive(base);
        Ok(base)
    }
}

impl Spool {
    /// Records progress for the copier and wakes it: the reader has just taken
    /// the bytes it was waiting to be told about.
    fn advance(&mut self, read: u64) {
        self.position += read;
        self.shared.arrive(self.position);
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        {
            let _window = self.shared.window.lock().unwrap_or_else(|p| p.into_inner());
            self.shared.wake.notify_all();
        }
        if let Some(copier) = self.copier.take() {
            let _ = copier.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The name this item's spool goes under, distinct per process so two players
/// on one file do not write the same temporary.
fn temporary(path: &Path, size: u64) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| {
            n.to_string_lossy()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        })
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "fvid-spool-{}-{}-{name}-{size}.bin",
        std::process::id(),
        {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            NEXT.fetch_add(1, Ordering::Relaxed)
        }
    ))
}

/// A file, optionally standing in front of a spool of itself. A reader takes
/// this instead of a `File` so the choice of how its bytes arrive happens once,
/// on measurement, and every reader downstream keeps one type.
pub enum Source {
    Plain(File),
    Spooled(Spool),
}

impl Source {
    /// Opens `path`, copying it locally when a probe read says the source
    /// cannot keep a frame period's bytes inside a frame period. The handle
    /// comes back as well, so the caller can wait for the lead it wants.
    pub fn open(path: &Path, under: f64, lead: u64) -> io::Result<(Source, Option<SpoolHandle>)> {
        // An item shorter than a block is answered by one request whatever the
        // source, and the probe over its few hundred bytes only says what the
        // syscalls cost that run - which is how a small local file came to be
        // judged slower than a cloud one.
        let body = fs::metadata(path).is_ok_and(|meta| meta.len() >= SPOOL_ABOVE);
        let slow = body && source_rate(path, CHUNK).is_some_and(|rate| rate < under);
        if !slow {
            return Ok((Source::Plain(File::open(path)?), None));
        }
        match Spool::open(path, lead) {
            Ok(spool) => {
                let handle = spool.handle();
                Ok((Source::Spooled(spool), Some(handle)))
            }
            // A disk too full to spool is a disk to read from anyway.
            Err(_) => Ok((Source::Plain(File::open(path)?), None)),
        }
    }
}

impl Read for Source {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(file) => file.read(out),
            Self::Spooled(spool) => spool.read(out),
        }
    }
}

impl Seek for Source {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Plain(file) => file.seek(from),
            Self::Spooled(spool) => spool.seek(from),
        }
    }
}
