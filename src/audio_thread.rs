//! Audio decode thread that runs independently from video decoding.
//! Reads encoded packets, decodes them to PCM, and pushes to the audio backend.
use crate::audio::{AudioBackend, AudioDecode, AudioStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Depth of the worker's event channel.
const QUEUE: usize = 24;

/// Audio the worker keeps queued ahead of the device. The ring behind
/// `AudioBackend::push` is finite and drops what does not fit, so decoding at
/// CPU speed would throw most of a track away.
const LEAD: Duration = Duration::from_millis(200);

/// How long to hold the decoder before queueing another packet, so the decoded
/// frontier stays `LEAD` ahead of the device instead of running away from it.
fn pacing(frontier: Duration, device: Duration) -> Duration {
    frontier.saturating_sub(device).saturating_sub(LEAD)
}

enum Command {
    Play,
    Pause,
    Rewind,
    Seek(i64),
    /// Output gain in thousandths.
    Level(u32),
    /// Stream frames consumed per device frame, in thousandths.
    Tempo(u32),
    Stop,
}

pub enum AudioEvent {
    Started,
    Ended(u64),
    Error(String),
}

/// Handle to the audio decoding thread; dropping it stops the thread.
pub struct AudioPlayback {
    commands: SyncSender<Command>,
    events: Receiver<AudioEvent>,
    generation: u64,
    thread: Option<thread::JoinHandle<()>>,
    position: Arc<Mutex<Duration>>,
    timescale: u32,
    duration: Option<Duration>,
}

impl AudioPlayback {
    /// Open audio in a background thread: build the backend and the decoder,
    /// then wait for `play`. Starting paused is what lets the caller anchor the
    /// audio clock at the same instant it starts presenting pictures; a device
    /// that began clocking here would run ahead by exactly the caller's preroll.
    /// The factory closure creates the backend on the audio thread, avoiding
    /// Send requirements on backends whose underlying streams are thread-local
    /// (e.g. cpal on macOS).
    pub fn start<F>(stream: Box<dyn AudioStream>, backend_factory: F) -> Self
    where
        F: FnOnce() -> Box<dyn AudioBackend> + Send + 'static,
    {
        let timescale = stream.timescale();
        let duration = stream.duration();
        let (commands, command_rx) = sync_channel(16);
        let (event_tx, events) = sync_channel(QUEUE);
        let position = Arc::new(Mutex::new(Duration::ZERO));
        let position_clone = position.clone();

        let thread = thread::Builder::new()
            .name("fvid-audio-decode".into())
            .spawn(move || {
                #[cfg(feature = "player")]
                fvid_platform::prioritize_playback_thread();

                let sample_rate = stream.sample_rate();
                let channels = stream.channels();

                let mut backend = backend_factory();

                if let Err(e) = backend.start(crate::audio::AudioSpec {
                    sample_rate,
                    channels,
                    format: crate::audio::SampleFormat::F32,
                }) {
                    let _ = event_tx.send(AudioEvent::Error(format!("Audio backend start: {e}")));
                    return;
                }

                let decoder = match crate::codec::make_audio_decoder(
                    stream.codec(),
                    stream.extra_data(),
                    sample_rate,
                    channels,
                    stream.bits_per_sample(),
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ =
                            event_tx.send(AudioEvent::Error(format!("Audio decoder init: {e}")));
                        return;
                    }
                };

                let _ = event_tx.send(AudioEvent::Started);

                Worker {
                    stream,
                    decoder,
                    backend,
                    commands: command_rx,
                    events: event_tx,
                    playing: false,
                    ended: false,
                    generation: 0,
                    position: position_clone,
                }
                .run();
            })
            .expect("spawn audio decode thread");

        Self {
            commands,
            events,
            generation: 0,
            thread: Some(thread),
            position,
            timescale,
            duration,
        }
    }

    /// Length of the track, as the container stated it when the stream opened.
    /// An item with no picture takes its timeline total from here.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Generation of the most recent rewind or seek.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Run the audio clock, and the device behind it, from its current position.
    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    /// Set the output gain in thousandths; 1000 is the recorded level.
    pub fn set_volume(&self, milli: u32) {
        let _ = self.commands.send(Command::Level(milli));
    }

    /// Set how much stream audio the device pulls per unit of its own time.
    pub fn set_rate(&self, milli: u32) {
        let _ = self.commands.send(Command::Tempo(milli));
    }

    pub fn rewind(&mut self) {
        self.generation += 1;
        let _ = self.commands.send(Command::Rewind);
    }

    /// Seek the audio clock to a stream position.
    pub fn seek(&mut self, target: Duration) {
        self.generation += 1;
        if self.timescale > 0 {
            let pts = (target.as_secs_f64() * f64::from(self.timescale)) as i64;
            let _ = self.commands.send(Command::Seek(pts));
        }
    }

    /// Poll for the next audio event without blocking.
    pub fn poll(&self) -> Option<AudioEvent> {
        self.events.try_recv().ok()
    }

    /// Current audio playback position (for A/V sync).
    /// Returns the position tracked by the audio decode thread.
    pub fn position(&self) -> Duration {
        *self.position.lock().unwrap()
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    stream: Box<dyn AudioStream>,
    decoder: Box<dyn AudioDecode>,
    backend: Box<dyn AudioBackend>,
    commands: Receiver<Command>,
    events: SyncSender<AudioEvent>,
    playing: bool,
    ended: bool,
    generation: u64,
    position: Arc<Mutex<Duration>>,
}

impl Worker {
    /// Decode and queue one packet, reporting a terminal event if the stream
    /// ended or failed and how long to wait before feeding the next one.
    fn decode_next(&mut self) -> (Option<AudioEvent>, Duration) {
        let packet = match self.stream.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => {
                self.ended = true;
                return (Some(AudioEvent::Ended(self.generation)), Duration::ZERO);
            }
            Err(e) => {
                self.ended = true;
                return (Some(AudioEvent::Error(e.to_string())), Duration::ZERO);
            }
        };

        let mut decoded = match self.decoder.decode_encoded(
            &packet.data,
            packet.pts.max(0) as u64,
            packet.duration.max(0) as u64,
        ) {
            Ok(Some(d)) => d,
            Ok(None) => return (None, Duration::ZERO),
            Err(e) => {
                // A packet that cannot be decoded usually means the whole track
                // is undecodable, so stop rather than report one error per frame.
                self.ended = true;
                return (Some(AudioEvent::Error(e.to_string())), Duration::ZERO);
            }
        };

        match self.stream.packet_sample_limit(packet.duration.max(0) as u64) {
            Ok(Some(frames)) => {
                let bytes = frames.checked_mul(usize::from(self.stream.channels())).and_then(|n| n.checked_mul(4));
                match bytes {
                    Some(bytes) if bytes <= decoded.data.len() => decoded.data.truncate(bytes),
                    _ => {
                        self.ended = true;
                        return (Some(AudioEvent::Error("audio presentation window exceeds decoded PCM".into())), Duration::ZERO);
                    }
                }
            }
            Ok(None) => {},
            Err(error) => {
                self.ended = true;
                return (Some(AudioEvent::Error(error.to_string())), Duration::ZERO);
            }
        }

        let decoded = match self.stream.present_decoded(decoded, packet.pts) {
            Ok(Some(packet)) => packet,
            Ok(None) => return (None,Duration::ZERO),
            Err(error) => {
                self.ended = true;
                return (Some(AudioEvent::Error(error.to_string())),Duration::ZERO);
            }
        };

        let frontier = if self.stream.codec() == "mp4a" {
            let stride = usize::from(self.stream.channels()) * 4;
            let rate = self.stream.sample_rate();
            if stride == 0 || rate == 0 || !decoded.data.len().is_multiple_of(stride) {
                self.ended = true;
                return (Some(AudioEvent::Error("invalid decoded AAC PCM geometry".into())),Duration::ZERO);
            }
            let frames = decoded.data.len()/stride;
            let nanos = frames as u128 * 1_000_000_000/u128::from(rate);
            decoded.presentation_time().saturating_add(Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)))
        } else {
            self.stream.time_of(packet.pts.max(0).saturating_add(packet.duration.max(0)))
        };

        if let Err(e) = self.backend.push(decoded) {
            self.ended = true;
            return (
                Some(AudioEvent::Error(format!("Audio push: {e}"))),
                Duration::ZERO,
            );
        }

        // The backend counts frames actually handed to the audio hardware, so
        // its clock reflects what has been heard rather than what has decoded.
        let device_pos = self.backend.position();
        if let Ok(mut pos) = self.position.lock() {
            *pos = device_pos;
        }

        (None, pacing(frontier, device_pos))
    }

    /// Run the device from wherever its clock currently sits. Play is
    /// idempotent because the caller commands it at every resume as well as at
    /// the first presentation.
    fn resume_clock(&mut self) {
        if self.playing {
            return;
        }
        self.playing = true;
        // A device that refuses to run means no audio at all, and without a
        // clock the picture would freeze at the last frame the gate allowed.
        if let Err(error) = self.backend.resume() {
            self.ended = true;
            let _ = self
                .events
                .send(AudioEvent::Error(format!("Audio resume: {error}")));
        }
    }

    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Play => self.resume_clock(),
            Command::Pause => {
                if self.playing {
                    self.playing = false;
                    let _ = self.backend.pause();
                }
            }
            Command::Rewind => {
                self.generation += 1;
                self.ended = false;
                self.stream.rewind();
                self.decoder.reset();
                let _ = self.backend.flush(Duration::ZERO);
                if let Ok(mut pos) = self.position.lock() {
                    *pos = Duration::ZERO;
                }
                self.resume_clock();
            }
            Command::Seek(pts) => {
                self.generation += 1;
                self.ended = false;
                let result_pts = self.stream.seek_to(pts);
                self.decoder.reset();
                // Anchor the clock at the sample actually landed on, which can
                // precede the requested point; the backend advances from there.
                let anchor = self.stream.time_of(result_pts);
                let _ = self.backend.flush(anchor);
                if let Ok(mut pos) = self.position.lock() {
                    *pos = anchor;
                }
            }
            Command::Level(milli) => {
                if let Err(error) = self.backend.set_volume(milli) {
                    let _ = self
                        .events
                        .send(AudioEvent::Error(format!("Audio volume: {error}")));
                }
            }
            Command::Tempo(milli) => {
                if let Err(error) = self.backend.set_rate(milli) {
                    let _ = self
                        .events
                        .send(AudioEvent::Error(format!("Audio rate: {error}")));
                }
            }
            Command::Stop => return false,
        }
        true
    }

    fn run(mut self) {
        loop {
            // Drain commands before doing work.
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

            if self.playing && !self.ended {
                let (event, pace) = self.decode_next();
                if let Some(event) = event {
                    if self.events.send(event).is_err() {
                        return;
                    }
                }
                // Waiting on the command channel instead of sleeping lets a seek
                // or a stop land in the middle of the pacing window.
                if !pace.is_zero() {
                    match self.commands.recv_timeout(pace) {
                        Ok(command) => {
                            if !self.handle(command) {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                continue;
            }

            // Nothing to do until commanded.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioStream, EncodedPacket, NullBackend};

    /// esds carrying an AAC-LC, 44.1 kHz mono AudioSpecificConfig: the same
    /// shape the MP4 reader hands to the decoder factory.
    const ESDS: &[u8] = &[
        0, 0, 0, 0, 3, 22, 0, 1, 0, 4, 17, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 2, 0x12,
        0x08,
    ];

    /// A 44.1 kHz mono track with no packets at all: enough for the thread to
    /// build a backend and a decoder, so commands reach a live worker.
    struct FakeStream {
        seeked: Arc<Mutex<Option<i64>>>,
    }

    impl AudioStream for FakeStream {
        fn codec(&self) -> &str {
            "mp4a"
        }
        fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
            Vec::new()
        }
        fn timescale(&self) -> u32 {
            44100
        }
        fn sample_rate(&self) -> u32 {
            44100
        }
        fn channels(&self) -> u16 {
            1
        }
        fn extra_data(&self) -> &[u8] {
            ESDS
        }
        fn next_packet(&mut self) -> crate::Result<Option<EncodedPacket>> {
            Ok(None)
        }
        fn rewind(&mut self) {}
        fn seek_to(&mut self, pts: i64) -> i64 {
            *self.seeked.lock().unwrap() = Some(pts);
            pts
        }
    }

    fn start() -> (AudioPlayback, Arc<Mutex<Option<i64>>>) {
        let seeked = Arc::new(Mutex::new(None));
        let playback = AudioPlayback::start(
            Box::new(FakeStream {
                seeked: seeked.clone(),
            }),
            || Box::new(NullBackend::default()),
        );
        (playback, seeked)
    }

    /// Wait for the audio worker to reach a state, failing loudly if it reports
    /// an error instead of letting the test time out.
    fn wait(playback: &AudioPlayback, pred: impl Fn() -> bool) -> bool {
        for _ in 0..2000 {
            while let Some(event) = playback.poll() {
                if let AudioEvent::Error(error) = event {
                    panic!("audio worker: {error}");
                }
            }
            if pred() {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// Consume events for a while, reporting whether the worker said it was done.
    fn ran_to_end(playback: &AudioPlayback, polls: u32) -> bool {
        for _ in 0..polls {
            while let Some(event) = playback.poll() {
                if let AudioEvent::Error(error) = &event {
                    panic!("audio worker: {error}");
                }
                if matches!(event, AudioEvent::Ended(_)) {
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    }

    /// The clock is commanded, not implied: a worker that decoded from the
    /// moment its thread existed would run ahead of pictures the player has not
    /// started presenting, and the sync gate can only hold video back.
    #[test]
    fn the_track_waits_for_a_play_command() {
        let (playback, _) = start();
        // The fake stream has no packets, so a running worker reports its end
        // within the first milliseconds.
        assert!(
            !ran_to_end(&playback, 100),
            "decoded before it was commanded to play"
        );

        playback.play();
        assert!(ran_to_end(&playback, 2000), "play did not start the worker");
    }

    #[test]
    fn seek_counts_ticks_in_the_stream_timescale() {
        let (mut playback, seeked) = start();
        playback.seek(Duration::from_secs(2));
        assert!(wait(&playback, || playback.position() > Duration::ZERO));

        // Converting with a movie timescale of 1000 would ask for tick 2000,
        // which is 44 times earlier than the requested position.
        assert_eq!(seeked.lock().unwrap().take(), Some(88200));
        assert!((playback.position().as_secs_f64() - 2.0).abs() < 0.01);
    }

    #[test]
    fn pacing_holds_the_decoder_at_the_lead() {
        let second = Duration::from_secs(1);
        assert_eq!(pacing(second * 3, Duration::ZERO), second * 3 - LEAD);
        assert_eq!(pacing(LEAD, Duration::ZERO), Duration::ZERO);
        // A device that has overtaken the decoder must not be held back.
        assert_eq!(pacing(Duration::ZERO, second * 5), Duration::ZERO);
    }

    /// A device that records what it was commanded, standing in for the
    /// hardware volume and rate controls.
    #[derive(Clone, Default)]
    struct Controls(Arc<Mutex<(u32, u32)>>);

    impl crate::audio::AudioBackend for Controls {
        fn start(&mut self, _: crate::audio::AudioSpec) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn push(&mut self, _: crate::audio::AudioPacket) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn position(&self) -> Duration {
            Duration::ZERO
        }
        fn flush(&mut self, _: Duration) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn pause(&mut self) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn resume(&mut self) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn stop(&mut self) -> Result<(), crate::audio::AudioError> {
            Ok(())
        }
        fn set_volume(&mut self, milli: u32) -> Result<(), crate::audio::AudioError> {
            self.0.lock().unwrap().0 = milli;
            Ok(())
        }
        fn set_rate(&mut self, milli: u32) -> Result<(), crate::audio::AudioError> {
            self.0.lock().unwrap().1 = milli;
            Ok(())
        }
    }

    /// The keys act on the device behind the thread, so a level set while the
    /// worker was still building its backend must still arrive.
    #[test]
    fn level_and_rate_reach_the_device() {
        let controls = Controls::default();
        let seen = controls.clone();
        let playback = AudioPlayback::start(
            Box::new(FakeStream {
                seeked: Arc::new(Mutex::new(None)),
            }),
            move || Box::new(controls),
        );
        playback.set_volume(350);
        playback.set_rate(2_000);
        assert!(wait(&playback, || {
            let (volume, rate) = *seen.0.lock().unwrap();
            volume == 350 && rate == 2_000
        }));
    }

    #[test]
    fn rewind_anchors_the_clock_at_zero() {
        let (mut playback, _) = start();
        playback.seek(Duration::from_secs(2));
        assert!(wait(&playback, || playback.position() > Duration::ZERO));

        playback.rewind();
        assert!(wait(&playback, || playback.position() == Duration::ZERO));
    }
}

#[cfg(test)]
mod presentation_window_tests {
    use super::*;
    pub(super) struct Capture(pub(super) Arc<Mutex<Vec<crate::audio::AudioPacket>>>);
    impl AudioBackend for Capture {
        fn start(&mut self, _:crate::audio::AudioSpec)->Result<(),crate::audio::AudioError>{Ok(())}
        fn push(&mut self, packet:crate::audio::AudioPacket)->Result<(),crate::audio::AudioError>{self.0.lock().unwrap().push(packet);Ok(())}
        fn position(&self)->Duration{Duration::ZERO}
        fn flush(&mut self,_:Duration)->Result<(),crate::audio::AudioError>{Ok(())}
        fn pause(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn resume(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn stop(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
    }
    #[test]
    fn worker_queues_container_window_after_consuming_complete_aac_packet() {
        let stream = crate::playback_mp4_audio::Mp4AudioReader::open_at(std::io::Cursor::new(include_bytes!("../tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a").as_slice()),Default::default(),1).unwrap();
        let decoder = crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let counts = Arc::new(Mutex::new(Vec::new()));
        let (_,commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { stream:Box::new(stream),decoder,backend:Box::new(Capture(counts.clone())),commands,events,playing:true,ended:false,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = counts.lock().unwrap();
        let sizes:Vec<usize> = packets.iter().map(|p| p.data.len()/8).collect();
        assert_eq!(packets[0].pts,0);
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        let mut expected = Vec::new();
        let demuxer = crate::container::mp4::Mp4Reader::open(std::io::Cursor::new(include_bytes!("../tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a").as_slice()),Default::default()).unwrap();
        let mut control = crate::native_media::DecodeProgress::new(None,None).unwrap();
        crate::native_media::decode_mp4_audio_reader_controlled(demuxer,&mut expected,None,Some(1),&mut control).unwrap();
        assert!(actual==expected,"playback and export presentation samples differ");
        assert_eq!(sizes.len(),48);
        assert_eq!(sizes.iter().sum::<usize>(),48008);
        assert_eq!(sizes[0],16);
        assert_eq!(sizes[1],1016);
        assert_eq!(sizes[47],896);
        assert!(sizes[2..47].iter().all(|n| *n==1024));
        drop(packets);
        // A seek must rebuild decoder state while suppressing preroll output.
        counts.lock().unwrap().clear();
        worker.handle(Command::Seek(2400));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = counts.lock().unwrap();
        assert_eq!(packets[0].pts,2056);
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected[2056*8..],"seek PCM differs from continuous decode");
        drop(packets);
        counts.lock().unwrap().clear();
        worker.handle(Command::Seek(0));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = counts.lock().unwrap();
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected,"backward seek PCM differs from initial decode");

    }
}

#[cfg(test)]
mod no_edit_seek_tests {
    use super::*;
    use super::presentation_window_tests::Capture;
    #[test]
    fn no_edit_aac_seek_matches_continuous_pcm() {
        let file = include_bytes!("../tests/fixtures/playback-errors/aac-no-edit.m4a").as_slice();
        let stream = crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(file),Default::default()).unwrap();
        assert!(stream.track().edits.is_empty());
        let decoder = crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_,commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let expected:Vec<u8> = captured.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        captured.lock().unwrap().clear();
        worker.handle(Command::Seek(2400));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = captured.lock().unwrap();
        assert_eq!(packets[0].pts,2040);
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected[2040*8..],"un-edited seek PCM differs from continuous decode");
    }
}

#[cfg(test)]
mod preroll_control_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize,Ordering};
    struct Decoder {
        inner:Box<dyn AudioDecode>,
        commands:SyncSender<Command>,
        calls:Arc<AtomicUsize>,
    }
    impl AudioDecode for Decoder {
        fn decode_encoded(&mut self,data:&[u8],pts:u64,duration:u64)->crate::Result<Option<crate::audio::AudioPacket>> {
            let result = self.inner.decode_encoded(data,pts,duration);
            if self.calls.fetch_add(1,Ordering::SeqCst)==0 { self.commands.send(Command::Pause).unwrap(); }
            result
        }
        fn reset(&mut self){self.inner.reset();}
    }
    struct Backend(std::sync::mpsc::Sender<()>);
    impl AudioBackend for Backend {
        fn start(&mut self,_:crate::audio::AudioSpec)->Result<(),crate::audio::AudioError>{Ok(())}
        fn push(&mut self,_:crate::audio::AudioPacket)->Result<(),crate::audio::AudioError>{panic!("preroll reached device")}
        fn position(&self)->Duration{Duration::ZERO}
        fn flush(&mut self,_:Duration)->Result<(),crate::audio::AudioError>{Ok(())}
        fn pause(&mut self)->Result<(),crate::audio::AudioError>{self.0.send(()).unwrap();Ok(())}
        fn resume(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
        fn stop(&mut self)->Result<(),crate::audio::AudioError>{Ok(())}
    }
    #[test]
    fn pause_and_stop_interrupt_preroll_between_access_units() {
        let file = include_bytes!("../tests/fixtures/playback-errors/aac-no-edit.m4a").as_slice();
        let mut stream = crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(file),Default::default()).unwrap();
        stream.seek_to(34000);
        let inner = crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (command_tx,commands) = sync_channel(16);
        let (events,_event_rx) = sync_channel(16);
        let (paused_tx,paused_rx) = std::sync::mpsc::channel();
        let worker_commands = command_tx.clone();
        let worker_calls = calls.clone();
        let thread = std::thread::spawn(move || {
            let worker = Worker { stream:Box::new(stream),decoder:Box::new(Decoder { inner,commands:worker_commands,calls:worker_calls }),backend:Box::new(Backend(paused_tx)),commands,events,playing:true,ended:false,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
            worker.run();
        });
        let paused = paused_rx.recv_timeout(Duration::from_secs(5));
        let count = calls.load(Ordering::SeqCst);
        command_tx.send(Command::Stop).unwrap();
        thread.join().unwrap();
        assert!(paused.is_ok(),"worker did not handle pause during preroll");
        assert_eq!(count,1,"worker decoded beyond the pause command");
    }
}

#[cfg(test)]
mod adts_seek_tests {
    use super::*;
    use super::presentation_window_tests::Capture;
    #[test]
    fn adts_seek_and_rewind_preserve_continuous_decoder_samples() {
        let file = include_bytes!("../tests/fixtures/audio/aac-mono-44k.aac").as_slice();
        let stream = crate::playback_aac::AacAudioReader::open(std::io::Cursor::new(file),Default::default()).unwrap();
        let frames = stream.aac().frames.len();
        let decoder = crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_,commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        for _ in 0..frames { assert!(worker.decode_next().0.is_none()); }
        let expected:Vec<u8> = captured.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        for (request,landed) in [(3000,2048),(0,0)] {
            captured.lock().unwrap().clear();
            worker.handle(Command::Seek(request));
            for _ in 0..frames { assert!(worker.decode_next().0.is_none()); }
            let packets = captured.lock().unwrap();
            assert_eq!(packets[0].pts,landed);
            let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
            assert!(actual==expected[landed as usize*4..],"ADTS seek changed decoded PCM");
        }
    }
}

#[cfg(test)]
mod matroska_aac_seek_tests {
    use super::*;
    use super::presentation_window_tests::Capture;
    #[test]
    fn matroska_aac_seek_preserves_pcm_and_nanosecond_timestamps() {
        let file = include_bytes!("../tests/fixtures/audio/aac-stereo.mka").as_slice();
        let stream = crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(file),Default::default()).unwrap();
        let decoder = crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_,commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        let mut final_pace = Duration::ZERO;
        for _ in 0..48 {
            let (event,pace) = worker.decode_next();
            assert!(event.is_none());
            final_pace = pace;
        }
        let packets = captured.lock().unwrap();
        let last = packets.last().unwrap();
        let expected_end = last.presentation_time()+Duration::from_nanos(21_333_333);
        assert_eq!(final_pace,expected_end.saturating_sub(LEAD));
        drop(packets);
        let expected:Vec<(u64,Vec<u8>)> = captured.lock().unwrap().iter().map(|p| { assert_eq!(p.timebase_den,1_000_000_000); (p.pts,p.data.clone()) }).collect();
        captured.lock().unwrap().clear();
        worker.handle(Command::Seek(50_000_000));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = captured.lock().unwrap();
        assert!((25_000_000..=50_000_000).contains(&packets[0].pts));
        let index = expected.iter().position(|(pts,_)| *pts==packets[0].pts).unwrap();
        assert_eq!(packets.len(),expected.len()-index);
        for (actual,(pts,data)) in packets.iter().zip(&expected[index..]) {
            assert_eq!(actual.pts,*pts);
            assert_eq!(actual.timebase_den,1_000_000_000);
            assert!(actual.data==*data,"Matroska seek PCM differs from continuous decode");
        }
    }
}
