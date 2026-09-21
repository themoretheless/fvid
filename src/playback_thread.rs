//! Runs a `NativeReader` on its own threads so a slow decoder never blocks the
//! window. The decode thread produces raw pictures; a converter thread turns
//! them into RGB while the next picture is being decoded, so throughput is
//! the slower of the two stages rather than their sum. Frames flow through a
//! small bounded queue; control messages (pause, rewind, seek) go the other
//! way. Every frame carries the generation of the last rewind or seek so
//! stale queued frames can be dropped.
use crate::playback_native::{NativeReader, Planar8, RawFrame, avc_to_planar8};
use std::{
    io::{BufRead, Seek},
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread,
    time::Duration,
};

/// How many converted frames may wait for the window. Two keeps the decoder
/// one frame ahead without holding many large buffers.
const QUEUE: usize = 2;

/// Picture data as the window draws it: packed RGB through an egui texture,
/// or 8-bit planes converted to RGB by the GPU shader.
pub enum Pixels {
    Rgb(Vec<u8>),
    Planar(Arc<Planar8>),
}

pub struct Frame {
    pub pixels: Pixels,
    pub dimensions: [usize; 2],
    pub period: Duration,
    /// `NativeReader::frame_interval` of this frame.
    pub interval: Option<(u128, u128, u32)>,
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
}

/// What the decode thread hands to the converter, in order.
enum Stage {
    Raw {
        raw: RawFrame,
        dimensions: [usize; 2],
        period: Duration,
        interval: Option<(u128, u128, u32)>,
        generation: u64,
    },
    Event(Event),
}

/// Handle to the decoding threads; dropping it stops them.
pub struct Playback {
    commands: SyncSender<Command>,
    events: Receiver<Event>,
    generation: u64,
    decoder: Option<thread::JoinHandle<()>>,
    converter: Option<thread::JoinHandle<()>>,
}

impl Playback {
    /// Takes a reader whose first frame is already decoded and starts decoding
    /// in the background, playing from that frame.
    pub fn start<R: BufRead + Seek + Send + 'static>(reader: NativeReader<R>) -> Self {
        let (commands, command_rx) = sync_channel(16);
        let (stage_tx, stage_rx) = sync_channel::<Stage>(1);
        let (event_tx, events) = sync_channel(QUEUE);
        let decoder = thread::Builder::new()
            .name("fvid-decode".into())
            .spawn(move || Worker::new(reader, command_rx, stage_tx).run())
            .expect("spawn decoder thread");
        let converter = thread::Builder::new()
            .name("fvid-convert".into())
            .spawn(move || {
                let mut serial = 0;
                for stage in stage_rx {
                    let event = match stage {
                        Stage::Raw {
                            raw,
                            dimensions,
                            period,
                            interval,
                            generation,
                        } => {
                            let pixels = match raw {
                                RawFrame::Rgb(rgb) => Pixels::Rgb(rgb),
                                RawFrame::Avc { picture, colour } => {
                                    Pixels::Planar(Arc::new(avc_to_planar8(&picture, colour)))
                                }
                                // Hardware output is already 8-bit planes: no copy at all.
                                RawFrame::Planar8(planes) => Pixels::Planar(planes),
                            };
                            serial += 1;
                            Event::Frame(Frame {
                                pixels,
                                dimensions,
                                period,
                                interval,
                                generation,
                                serial,
                            })
                        }
                        Stage::Event(event) => event,
                    };
                    if event_tx.send(event).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn converter thread");
        Self {
            commands,
            events,
            generation: 0,
            decoder: Some(decoder),
            converter: Some(converter),
        }
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
    /// The next queued event, if any, without waiting.
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
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
    fn new(reader: NativeReader<R>, commands: Receiver<Command>, stages: SyncSender<Stage>) -> Self {
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
            generation: self.generation,
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
                // `seek` leaves the target frame converted in the reader.
                self.pending = Some(match self.reader.seek(target) {
                    Ok(()) => self.stage(RawFrame::Rgb(self.reader.rgb().to_vec())),
                    Err(error) => Stage::Event(Event::Error(error.to_string())),
                });
            }
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
