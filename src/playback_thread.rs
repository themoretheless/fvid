//! Runs a `NativeReader` on its own thread so a slow decoder never blocks the
//! window. Decoded RGB frames flow through a small bounded queue; control
//! messages (pause, rewind, seek) go the other way. Every frame carries the
//! generation of the last rewind or seek so stale queued frames can be dropped.
use crate::playback_native::NativeReader;
use std::{
    io::{BufRead, Seek},
    sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel},
    thread,
    time::Duration,
};

/// How many decoded frames may wait for the window. Two keeps the decoder
/// one frame ahead without holding many large RGB buffers.
const QUEUE: usize = 2;

pub struct Frame {
    pub rgb: Vec<u8>,
    pub dimensions: [usize; 2],
    pub period: Duration,
    /// `NativeReader::frame_interval` of this frame.
    pub interval: Option<(u128, u128, u32)>,
    pub generation: u64,
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

/// Handle to the decoding thread; dropping it stops the thread.
pub struct Playback {
    commands: SyncSender<Command>,
    events: Receiver<Event>,
    generation: u64,
    thread: Option<thread::JoinHandle<()>>,
}

impl Playback {
    /// Takes a reader whose first frame is already decoded and starts decoding
    /// in the background, playing from that frame.
    pub fn start<R: BufRead + Seek + Send + 'static>(reader: NativeReader<R>) -> Self {
        let (commands, command_rx) = sync_channel(16);
        let (event_tx, events) = sync_channel(QUEUE);
        let thread = thread::Builder::new()
            .name("fvid-decode".into())
            .spawn(move || Worker::new(reader, command_rx, event_tx).run())
            .expect("spawn decoder thread");
        Self {
            commands,
            events,
            generation: 0,
            thread: Some(thread),
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
        // Unblock a worker waiting to hand over a frame, then let it exit.
        while self.events.try_recv().is_ok() {}
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker<R> {
    reader: NativeReader<R>,
    commands: Receiver<Command>,
    events: SyncSender<Event>,
    playing: bool,
    ended: bool,
    generation: u64,
    pending: Option<Event>,
}

impl<R: BufRead + Seek> Worker<R> {
    fn new(reader: NativeReader<R>, commands: Receiver<Command>, events: SyncSender<Event>) -> Self {
        let mut worker = Self {
            reader,
            commands,
            events,
            playing: true,
            ended: false,
            generation: 0,
            pending: None,
        };
        worker.pending = Some(worker.current_frame());
        worker
    }
    fn current_frame(&self) -> Event {
        Event::Frame(Frame {
            rgb: self.reader.rgb().to_vec(),
            dimensions: self.reader.dimensions(),
            period: self.reader.frame_period(),
            interval: self.reader.frame_interval(),
            generation: self.generation,
        })
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
                self.pending = match self.reader.rewind().and_then(|()| self.reader.read_frame())
                {
                    Ok(true) => Some(self.current_frame()),
                    Ok(false) => Some(Event::Ended(self.generation)),
                    Err(error) => Some(Event::Error(error.to_string())),
                };
            }
            Command::Seek(target) => {
                self.generation += 1;
                self.ended = false;
                self.pending = match self.reader.seek(target) {
                    Ok(()) => Some(self.current_frame()),
                    Err(error) => Some(Event::Error(error.to_string())),
                };
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
            if let Some(event) = self.pending.take() {
                match self.events.try_send(event) {
                    Ok(()) => {}
                    Err(TrySendError::Full(event)) => {
                        // Queue is full: keep the event and wait for a command or a slot.
                        self.pending = Some(event);
                        match self.commands.recv_timeout(Duration::from_millis(4)) {
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
                self.pending = match self.reader.read_frame() {
                    Ok(true) => Some(self.current_frame()),
                    Ok(false) => {
                        self.ended = true;
                        Some(Event::Ended(self.generation))
                    }
                    Err(error) => {
                        self.ended = true;
                        Some(Event::Error(error.to_string()))
                    }
                };
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
