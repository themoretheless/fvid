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

/// Bytes the copier and a beyond-spool read move at a time. Small enough that
/// one slow read of a cloud drive is a fraction of a second rather than the
/// several seconds a megabyte-hungry request can cost.
const CHUNK: usize = 1 << 20;

/// How long a reader will wait for the copier to reach the next block off the
/// source rather than fetching it itself. Past this the mount is being answered
/// twice for the same bytes, which is the thing to avoid; short of it, waiting
/// is just the buffering the reader came for.
const CHUNK_WAIT: Duration = Duration::from_secs(2);

/// How much of the source a reader buffers in front of itself. Small reads are
/// what turn a mount's latency into a stutter: eight kilobytes at a time asks
/// the question once per frame and a megabyte asks it a hundred times less.
pub const READ_AHEAD: usize = 1 << 20;

/// How far ahead of the reader the copier runs, in bytes: the disk a spool
/// costs while an item plays, and the span a source slower than its item can
/// still be caught up in.
pub const SPOOL_LEAD: u64 = 256 << 20;

/// The preroll a caller waits for before the first picture: five seconds is
/// more than a whole group of pictures needs and little enough that a source
/// which cannot stream the item at all is known about at once.
pub const SPOOL_PREROLL: Duration = Duration::from_secs(5);

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

/// Where a speed probe reads: just past the header, because that is where the
/// item's own bytes begin and playback goes first. A drive that has cached the
/// header says nothing about the body behind it, and the middle of a large item
/// is territory the reader has not asked for yet.
fn probe_at(size: u64) -> u64 {
    (1u64 << 23).min(size / 2)
}

/// How fast a source really is: the megabytes per second one cold read of the
/// item's body delivers, or nothing when the file cannot be read at all.
pub fn source_rate(path: &Path, probe: usize) -> Option<f64> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size == 0 {
        return None;
    }
    file.seek(SeekFrom::Start(probe_at(size))).ok()?;
    let mut buffer = vec![0u8; probe];
    let started = Instant::now();
    let read = file.read(&mut buffer).ok()?;
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
    /// One past the last byte written, so every offset below it is on disk.
    end: u64,
    /// Where the reader stands, so the copier runs ahead of playback rather
    /// than ahead of nothing.
    read_at: u64,
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
    slots: usize,
    stop: AtomicBool,
    window: Mutex<Window>,
    wake: Condvar,
}

impl Shared {
    /// Where a block of the item lives in the ring.
    fn slot(&self, at: u64) -> u64 {
        (at as usize / CHUNK % self.slots) as u64 * CHUNK as u64
    }

    /// Where the window stands, for a reader that has to know what is on disk.
    fn span(&self) -> (u64, u64) {
        let window = self.window.lock().unwrap_or_else(|p| p.into_inner());
        (window.base, window.end)
    }

    /// Publishes where the window stands for the watchers, from the window the
    /// caller already holds locked: a mirror taken outside the lock can be out
    /// of date by the time it lands, and a preroll would wait on a lead that
    /// has already been bitten.
    fn mirror(&self, window: &Window) {
        self.held
            .store(window.end.saturating_sub(window.base), Ordering::Release);
        self.slack
            .store(window.end.saturating_sub(window.read_at), Ordering::Release);
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
        let slots = (lead as usize).div_ceil(CHUNK) + 3;
        spool.set_len(slots as u64 * CHUNK as u64)?;
        let shared = Arc::new(Shared {
            held: Arc::new(AtomicU64::new(0)),
            slack: Arc::new(AtomicU64::new(0)),
            finished: Arc::new(AtomicBool::new(false)),
            claim: AtomicU64::new(u64::MAX),
            slots,
            stop: AtomicBool::new(false),
            window: Mutex::new(Window {
                base: 0,
                end: 0,
                read_at: 0,
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
            path: spool_path,
            position: 0,
            staging: vec![0u8; CHUNK],
            staged: 0,
            staged_at: 0,
            copier: Some(copier),
        })
    }

    /// What a caller watches while this reader is busy with the item.
    pub fn handle(&self) -> SpoolHandle {
        SpoolHandle {
            held: self.shared.held.clone(),
            slack: self.shared.slack.clone(),
            finished: self.shared.finished.clone(),
        }
    }

    /// Bytes of the item the spool already holds.
    pub fn copied(&self) -> u64 {
        self.shared.held.load(Ordering::Acquire)
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
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        // Claim the next offset under the same lock that says what is on disk,
        // so a committed offset always has its bytes behind it.
        let at = {
            let mut window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
            let read_at = window.read_at;
            if read_at >= window.end.saturating_add(lead)
                || read_at.saturating_add(lead) < window.base
            {
                // The reader has moved further from the window than a lead's
                // worth of gap, in either direction. The bytes it left behind
                // are not going to be watched again soon, and pulling them would
                // download the whole item to reach a byte the reader is already
                // asking for - and pay twice, since the reader is taking those
                // bytes from the source itself meanwhile.
                let base = (read_at / CHUNK as u64) * CHUNK as u64;
                window.base = base;
                window.end = base;
                shared.mirror(&window);
            }
            window.end
        };
        if at >= size {
            break;
        }
        // A window that has run its lead is full: wait for the reader to make
        // room rather than filling the disk behind it.
        let room = {
            let window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
            window.end.saturating_sub(window.read_at) < lead
        };
        if !room {
            shared.pause(Duration::from_millis(100));
            continue;
        }
        // Say which block is in flight before fetching it: a reader standing in
        // the same block waits for it rather than pulling it a second time.
        shared.claim.store(at, Ordering::Release);
        if source.seek(SeekFrom::Start(at)).is_err() {
            break;
        }
        // A block is copied whole before it is laid down: the ring is made of
        // blocks, and a short one would leave everything after it out of
        // position. Only the end of the item is allowed to be a short block.
        let wanted = CHUNK.min(size.saturating_sub(at) as usize);
        if source.read_exact(&mut buffer[..wanted]).is_err() {
            break;
        }
        if spool
            .seek(SeekFrom::Start(shared.slot(at)))
            .and_then(|_| spool.write_all(&buffer[..wanted]))
            .is_err()
        {
            break;
        }
        let read = wanted;
        let mut window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
        window.end = at.saturating_add(read as u64);
        // The window is the disk this spool costs, so its back edge follows its
        // front: what the reader has passed is not watched again, and a lead that
        // only ever extends forward is a copy of the whole item. Nothing is
        // released by it - the file is a ring of the window's size, and a slot is
        // given up by the block that overwrites it.
        window.base = window.end.saturating_sub(lead + CHUNK as u64);
        shared.mirror(&window);
        shared.wake.notify_all();
        // The block has landed, so nothing is in flight for it: a stale claim
        // would park a reader that has already outrun it.
        shared.claim.store(u64::MAX, Ordering::Release);
    }
    shared.finished.store(true, Ordering::Release);
    let _window = shared.window.lock().unwrap_or_else(|p| p.into_inner());
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
            && self.position < claim.saturating_add(CHUNK as u64)
            && !self.exhausted()
        {
            let until = Instant::now() + CHUNK_WAIT;
            while self.shared.span().1 <= self.position
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
        if self.position >= window.base && self.position < window.end {
            let want = (window.end - self.position).min(out.len() as u64) as usize;
            // The window is read a block at a time because the ring it lives in
            // wraps: the newest bytes can sit below the oldest ones in the file.
            let mut done = 0usize;
            while done < want {
                let at = self.position + done as u64;
                let inside = (at % CHUNK as u64) as usize;
                let take = (CHUNK - inside).min(want - done);
                self.spool
                    .seek(SeekFrom::Start(self.shared.slot(at) + inside as u64))?;
                let read = self.spool.read(&mut out[done..done + take])?;
                if read == 0 {
                    break;
                }
                done += read;
            }
            if done > 0 {
                self.position += done as u64;
                window.read_at = self.position;
                self.shared.mirror(&window);
                self.shared.wake.notify_all();
                return Ok(done);
            }
            // A local read that answers nothing while the copier still has the
            // byte to give is not the end of the item, so the source is asked.
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
        self.staged_at += take as u64;
        self.advance(take as u64);
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
        "fvid-spool-{}-{name}-{size}.bin",
        std::process::id()
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
