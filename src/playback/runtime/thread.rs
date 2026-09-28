//! Runs a `NativeReader` on its own threads so a slow decoder never blocks the
//! window. The decode thread produces raw pictures; a converter thread turns
//! them into RGB while the next picture is being decoded, so throughput is
//! the slower of the two stages rather than their sum. Frames flow through a
//! queue sized to a span of presentation time and a budget of bytes; control
//! messages (pause, rewind, seek) go the other way. Every frame carries the
//! generation of the last rewind or seek so stale queued frames can be dropped.
use crate::color::Grade;
use crate::playback_native::{
    NativeReader, Planar8, RawFrame, avc_to_planar8, planar8_to_rgb, rotate_planar8, yuv_to_rgb,
};
use std::{
    io::{BufRead, Seek},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread,
    time::Duration,
};

/// Presentation slack the queue holds, matching the `LEAD` the audio thread
/// keeps ahead of its device so both clocks wait the same span of time.
const LEAD: Duration = Duration::from_millis(200);

/// Most and least a queue may hold for one item, whatever its rate and size.
/// The floor keeps a picture in hand while the window shows one; the ceiling
/// stops a very high frame rate from asking for an unreasonable count.
const QUEUE_MIN: usize = 2;
const QUEUE_MAX: usize = 24;

/// Most picture data a queue may hold for one item.
const QUEUE_BYTES: usize = 64 << 20;

/// Queue depth holding `LEAD` of presentation within `QUEUE_BYTES` of pictures.
///
/// A fixed count is wrong at both ends. Twenty-four pictures are 4.8 seconds of
/// slack at 5 fps and 200 ms at 120, so the same constant gives one source a
/// buffer twenty-four times the other's; and the same count holds 142 MiB of
/// 1080p RGB or 570 MiB at 4K, which the reader's own budget never covered.
/// How much the window needs is a span of time, and what the item costs is a
/// span of bytes, so depth is taken from those and not from a count.
pub fn queue_depth(period: Duration, frame_bytes: usize) -> usize {
    if period.is_zero() {
        return QUEUE_MAX;
    }
    let frames = LEAD
        .as_nanos()
        .div_ceil(period.as_nanos().max(1))
        .clamp(QUEUE_MIN as u128, QUEUE_MAX as u128) as usize;
    if frame_bytes == 0 {
        return frames;
    }
    frames.min(QUEUE_BYTES / frame_bytes).max(QUEUE_MIN)
}

/// Fill the queue's span of presentation before starting/restarting the clock,
/// so the first picture appears with the buffer as full as it ever runs.
/// The cap keeps a low-frame-rate source from waiting out a stale twelve.
pub fn startup_buffer(period: Duration) -> Duration {
    period.saturating_mul(12).min(LEAD)
}

/// Picture data as the window draws it: packed RGB through an egui texture,
/// or 8-bit planes converted to RGB by the GPU shader.
pub enum Pixels {
    Rgb(Vec<u8>),
    /// 8-bit planes, and the grade they still owe the window. A plane picture
    /// only keeps its planes when that grade is one table the fragment shader
    /// can bind — see [`Grade::shader_look`] — so the second half is never a
    /// grade the shader would have to refuse.
    Planar(Arc<Planar8>, Option<Arc<Grade>>),
}

pub struct Frame {
    pub pixels: Pixels,
    pub dimensions: [usize; 2],
    pub period: Duration,
    /// `NativeReader::frame_interval` of this frame.
    pub interval: Option<(u128, u128, u32)>,
    /// Presentation timestamp in track timescale units (for A/V sync).
    pub pts: Option<(i64, u32)>,
    pub generation: u64,
    /// Increases with every frame handed to the window; the GPU uploads a
    /// frame once and skips repaints that show the same one.
    pub serial: u64,
}

pub enum Event {
    Frame(Frame),
    /// The stream ran out at this generation.
    Ended(u64),
    Error(String),
}

enum Command {
    Play,
    Pause,
    Rewind,
    Seek(Duration),
    Stop,
    StageReady,
}

/// What the decode thread hands to the converter, in order.
enum Stage {
    Raw {
        raw: RawFrame,
        dimensions: [usize; 2],
        period: Duration,
        interval: Option<(u128, u128, u32)>,
        pts: Option<(i64, u32)>,
        generation: u64,
        /// Degrees clockwise the picture is stored away from upright. Only the
        /// plane path is turned with it: packed RGB reaches this thread already
        /// turned, shaped that way by the reader.
        rotation: u16,
    },
    Event(Event),
}

/// The picture a stage carries, in the form the window draws it.
///
/// This is where a [`Grade`] is applied, because it is the one place every
/// CPU-bound frame passes through before the window sees it, and it is applied
/// after the container's turn so the codes are looked up in the orientation
/// they are shown in. A plane picture whose grade is a single table does not
/// have to become packed RGB for it: the fragment shader reads that table, so
/// the planes travel with it and are named by the `Planar` arm's second half.
/// Anything the shader cannot carry — a second grid after the conversion, or
/// nothing at all — is applied here, and a grade that turns out to be its own
/// input leaves the picture untouched, which is what a caller asking for the
/// panel's own curve gets.
fn into_pixels(
    raw: RawFrame,
    rotation: u16,
    budget: usize,
    grade: Option<&Arc<Grade>>,
) -> crate::Result<Pixels> {
    let pixels = match raw {
        RawFrame::Rgb(rgb) => Pixels::Rgb(rgb),
        RawFrame::Avc { picture, colour } => {
            Pixels::Planar(Arc::new(avc_to_planar8(&picture, colour)), None)
        }
        // Hardware output is already 8-bit planes: no copy at all.
        RawFrame::Planar8(planes) => Pixels::Planar(planes, None),
        RawFrame::Yuv {
            data,
            luma_len,
            chroma_len,
            width,
            height,
            sx,
            sy,
        } => {
            let mut rgb = Vec::new();
            yuv_to_rgb(&data, luma_len, chroma_len, width, height, sx, sy, &mut rgb);
            Pixels::Rgb(rgb)
        }
    };
    // The turn the container asked for, still owed to the planes: packed RGB
    // reaches this thread already turned, shaped that way by the reader.
    let pixels = match (pixels, rotation) {
        (Pixels::Planar(planes, None), rotation) if rotation != 0 => {
            Pixels::Planar(Arc::new(rotate_planar8(&planes, rotation)), None)
        }
        (pixels, _) => pixels,
    };
    let Some(grade) = grade.filter(|grade| !grade.is_identity()) else {
        return Ok(pixels);
    };
    match pixels {
        Pixels::Rgb(mut rgb) => {
            grade.apply(&mut rgb);
            Ok(Pixels::Rgb(rgb))
        }
        // One table the shader can bind is one table the planes can travel to it
        // with, and the window's own draw reads it there. Measured in
        // `player_gpu`, the two routes then land on the same bytes.
        Pixels::Planar(planes, _) if grade.is_shader_look() => {
            Ok(Pixels::Planar(planes, Some(Arc::clone(grade))))
        }
        Pixels::Planar(planes, _) => {
            let mut rgb = Vec::new();
            planar8_to_rgb(&planes, &mut rgb, budget)?;
            grade.apply(&mut rgb);
            Ok(Pixels::Rgb(rgb))
        }
    }
}

/// Handle to the decoding threads; dropping it stops them.
pub struct Playback {
    commands: SyncSender<Command>,
    events: Receiver<Event>,
    /// How many pictures the event queue can hold; `filled` is measured against it.
    depth: usize,
    /// Pictures sitting in the queue: the converter raises it as one goes in,
    /// the window lowers it as one comes out. `Receiver::len` is not available,
    /// and a display of the buffer has to say something between the two ends.
    filled: Arc<AtomicUsize>,
    generation: u64,
    decoder: Option<thread::JoinHandle<()>>,
    converter: Option<thread::JoinHandle<()>>,
}

impl Playback {
    /// Takes a reader whose first frame is already decoded and starts decoding
    /// in the background, playing from that frame. A `grade` is applied to every
    /// picture this thread hands over, on its converter thread — or handed over
    /// with the picture for the fragment shader to apply, when it is one the
    /// shader can carry.
    pub fn start<R: BufRead + Seek + Send + 'static>(
        reader: NativeReader<R>,
        grade: Option<Grade>,
    ) -> Self {
        let budget = reader.rgb_budget();
        let [width, height] = reader.dimensions();
        let depth = queue_depth(
            reader.frame_period(),
            width.saturating_mul(height).saturating_mul(3),
        );
        let (commands, command_rx) = sync_channel(16);
        let (stage_tx, stage_rx) = sync_channel::<Stage>(1);
        let (event_tx, events) = sync_channel(depth);
        let filled = Arc::new(AtomicUsize::new(0));
        let queued = filled.clone();
        let decoder = thread::Builder::new()
            .name("fvid-decode".into())
            .spawn(move || {
                #[cfg(feature = "player")]
                fvid_platform::prioritize_playback_thread();
                Worker::new(reader, command_rx, stage_tx).run()
            })
            .expect("spawn decoder thread");
        let stage_ready = commands.clone();
        let converter = thread::Builder::new()
            .name("fvid-convert".into())
            .spawn(move || {
                #[cfg(feature = "player")]
                fvid_platform::prioritize_playback_thread();
                let mut serial = 0;
                // Shared rather than moved per frame: a plane picture hands the
                // same baked table to the window with itself.
                let grade = grade.map(Arc::new);
                for stage in stage_rx {
                    // Freeing a slot must wake the producer immediately. A full
                    // control queue already contains messages that will wake it.
                    let _ = stage_ready.try_send(Command::StageReady);
                    let event = match stage {
                        Stage::Raw {
                            raw,
                            dimensions,
                            period,
                            interval,
                            pts,
                            generation,
                            rotation,
                        } => match into_pixels(raw, rotation, budget, grade.as_ref()) {
                            Ok(pixels) => {
                                serial += 1;
                                Event::Frame(Frame {
                                    pixels,
                                    dimensions,
                                    period,
                                    interval,
                                    pts,
                                    generation,
                                    serial,
                                })
                            }
                            Err(error) => Event::Error(error.to_string()),
                        },
                        Stage::Event(event) => event,
                    };
                    // Counted before the send: the window can take the picture
                    // out of the channel before this thread is scheduled again,
                    // and a count raised after that would leave it high forever.
                    queued.fetch_add(1, Ordering::Relaxed);
                    if event_tx.send(event).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn converter thread");
        Self {
            commands,
            events,
            depth,
            filled,
            generation: 0,
            decoder: Some(decoder),
            converter: Some(converter),
        }
    }
    /// How many pictures the queue can hold for this item.
    pub fn depth(&self) -> usize {
        self.depth
    }
    /// How many decoded pictures are waiting for the window right now. Empty
    /// means the producer cannot keep up, whether that is the decoder or the
    /// bytes behind it; full means presentation is the only thing in the way.
    ///
    /// A converter held at a full channel has already counted the picture it is
    /// trying to place, so the reading is kept inside the depth it is drawn against.
    pub fn filled(&self) -> usize {
        self.filled.load(Ordering::Relaxed).min(self.depth)
    }
    /// Generation of the most recent rewind or seek; frames from earlier
    /// generations are stale.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }
    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }
    pub fn rewind(&mut self) {
        self.generation += 1;
        let _ = self.commands.send(Command::Rewind);
    }
    pub fn seek(&mut self, target: Duration) {
        self.generation += 1;
        let _ = self.commands.send(Command::Seek(target));
    }
    /// Take the next event, counting it out of the queue the fill display reads.
    pub fn poll(&self) -> Option<Event> {
        let event = self.events.try_recv().ok();
        if event.is_some() {
            let _ = self
                .filled
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_sub(1))
                });
        }
        event
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.decoder.take() {
            let _ = thread.join();
        }
        // The converter may be blocked handing over a frame; keep draining
        // until it has seen the closed stage channel and exited.
        if let Some(thread) = self.converter.take() {
            while !thread.is_finished() {
                while self.events.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(1));
            }
            let _ = thread.join();
        }
    }
}

struct Worker<R> {
    reader: NativeReader<R>,
    commands: Receiver<Command>,
    stages: SyncSender<Stage>,
    playing: bool,
    ended: bool,
    generation: u64,
    pending: Option<Stage>,
}

impl<R: BufRead + Seek> Worker<R> {
    fn new(
        reader: NativeReader<R>,
        commands: Receiver<Command>,
        stages: SyncSender<Stage>,
    ) -> Self {
        let mut worker = Self {
            reader,
            commands,
            stages,
            playing: true,
            ended: false,
            generation: 0,
            pending: None,
        };
        // The reader already holds its first frame converted.
        worker.pending = Some(worker.stage(RawFrame::Rgb(worker.reader.rgb().to_vec())));
        worker
    }
    fn stage(&self, raw: RawFrame) -> Stage {
        Stage::Raw {
            raw,
            dimensions: self.reader.dimensions(),
            period: self.reader.frame_period(),
            interval: self.reader.frame_interval(),
            pts: self.reader.current_pts(),
            generation: self.generation,
            rotation: self.reader.rotation(),
        }
    }
    fn decode_next(&mut self) -> Stage {
        match self.reader.read_frame_raw() {
            Ok(Some(raw)) => self.stage(raw),
            Ok(None) => {
                self.ended = true;
                Stage::Event(Event::Ended(self.generation))
            }
            Err(error) => {
                self.ended = true;
                Stage::Event(Event::Error(error.to_string()))
            }
        }
    }
    /// Returns false when the thread should exit.
    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Play => self.playing = true,
            Command::Pause => self.playing = false,
            Command::Rewind => {
                self.generation += 1;
                self.playing = true;
                self.ended = false;
                self.pending = Some(match self.reader.rewind() {
                    Ok(()) => self.decode_next(),
                    Err(error) => Stage::Event(Event::Error(error.to_string())),
                });
            }
            Command::Seek(target) => {
                self.generation += 1;
                self.ended = false;
                // Preserve the normal GPU plane path for the target picture.
                self.pending = Some(match self.reader.seek_raw(target) {
                    Ok(Some(raw)) => self.stage(raw),
                    Ok(None) => Stage::Event(Event::Ended(self.generation)),
                    Err(error) => Stage::Event(Event::Error(error.to_string())),
                });
            }
            Command::StageReady => {}
            Command::Stop => return false,
        }
        true
    }
    fn run(mut self) {
        loop {
            // Drain every waiting command before doing any more work.
            loop {
                match self.commands.try_recv() {
                    Ok(command) => {
                        if !self.handle(command) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if let Some(stage) = self.pending.take() {
                match self.stages.try_send(stage) {
                    Ok(()) => {}
                    Err(TrySendError::Full(stage)) => {
                        // Converter is busy: keep the stage and wait for a command or a slot.
                        self.pending = Some(stage);
                        match self.commands.recv_timeout(Duration::from_millis(2)) {
                            Ok(command) => {
                                if !self.handle(command) {
                                    return;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
                continue;
            }
            if self.playing && !self.ended {
                self.pending = Some(self.decode_next());
                continue;
            }
            // Nothing to do until the window says so.
            match self.commands.recv() {
                Ok(command) => {
                    if !self.handle(command) {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    }
}
