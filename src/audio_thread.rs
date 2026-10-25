//! Audio decode thread that runs independently from video decoding.
//! Reads encoded packets, decodes them to PCM, and pushes to the audio backend.
use crate::audio::{AudioBackend, AudioDecode, AudioStream};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Dolby choices owned by a playback session, independent of decoder lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DolbyPlaybackPolicy {
    pub compression: crate::codec::ac3_decoder::CompressionMode,
    pub normalize_dialogue: bool,
    pub ac3_stereo_output: bool,
    pub ac3_stereo_downmix: crate::codec::ac3_decoder::StereoDownmixPolicy,
    /// Request codec-aware L/R output from E-AC-3 readers. Native output is
    /// retained by default; stereo uses the signalled independent downmix.
    pub eac3_stereo_output: bool,
    pub eac3_stereo_downmix: crate::codec::eac3_decoder::StereoDownmixPolicy,
}
impl DolbyPlaybackPolicy {
    fn apply_to(self, decoder: &mut dyn AudioDecode) {
        decoder.set_dolby_compression(self.compression);
        decoder.set_dolby_dialogue_normalization(self.normalize_dialogue);
        decoder.set_ac3_stereo_downmix(self.ac3_stereo_downmix);
        decoder.set_eac3_stereo_downmix(self.eac3_stereo_downmix);
    }
}
impl From<crate::codec::ac3_decoder::CompressionMode> for DolbyPlaybackPolicy {
    fn from(compression: crate::codec::ac3_decoder::CompressionMode) -> Self {
        Self { compression, ..Self::default() }
    }
}

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
    #[cfg(test)]
    Seek(i64),
    SeekTime(Duration),
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
    duration: Arc<Mutex<Option<Duration>>>,
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
        Self::start_with_compression(stream,crate::codec::ac3_decoder::CompressionMode::Line,backend_factory)
    }

    /// Preserve a session's Dolby policy across reader-created decoder resets.
    /// Other codecs keep their own behavior; their policy hook is a no-op.
    pub fn start_with_compression<F>(stream: Box<dyn AudioStream>, compression: crate::codec::ac3_decoder::CompressionMode, backend_factory: F) -> Self
    where F: FnOnce() -> Box<dyn AudioBackend> + Send + 'static {
        Self::start_with_dolby_policy(stream,compression.into(),backend_factory)
    }

    /// Apply all session Dolby choices whenever a reader supplies a decoder.
    pub fn start_with_dolby_policy<F>(mut stream:Box<dyn AudioStream>,policy:DolbyPlaybackPolicy,backend_factory:F)->Self
    where F:FnOnce()->Box<dyn AudioBackend>+Send+'static {
        let timescale = stream.timescale();
        let duration = Arc::new(Mutex::new(stream.duration()));
        let duration_clone = duration.clone();
        let (commands, command_rx) = sync_channel(16);
        let (event_tx, events) = sync_channel(QUEUE);
        let position = Arc::new(Mutex::new(Duration::ZERO));
        let position_clone = position.clone();

        let thread = thread::Builder::new()
            .name("fvid-audio-decode".into())
            .spawn(move || {
                #[cfg(feature = "player")]
                fvid_platform::prioritize_playback_thread();

                if let Err(error) = stream.set_ac3_stereo_output(policy.ac3_stereo_output).and_then(|_|stream.set_eac3_stereo_output(policy.eac3_stereo_output)) {
                    let _ = event_tx.send(AudioEvent::Error(format!("Audio output negotiation: {error}")));
                    return;
                }
                let sample_rate = stream.sample_rate();
                let channels = stream.channels();

                let mut backend = backend_factory();

                let stereo_router=match stream.stereo_router(){Ok(mask)=>mask,Err(error)=>{let _=event_tx.send(AudioEvent::Error(format!("Audio speaker layout: {error}")));return;}};
                let output_configuration = Some((crate::audio::AudioSpec {sample_rate,channels,format:crate::audio::SampleFormat::F32},stereo_router.clone()));
                if let Err(e) = backend.start_with_router(crate::audio::AudioSpec {
                    sample_rate,
                    channels,
                    format: crate::audio::SampleFormat::F32,
                },stereo_router) {
                    let _ = event_tx.send(AudioEvent::Error(format!("Audio backend start: {e}")));
                    return;
                }

                let mut decoder = match stream.make_decoder() {
                    Ok(d) => d,
                    Err(e) => {
                        let _ =
                            event_tx.send(AudioEvent::Error(format!("Audio decoder init: {e}")));
                        return;
                    }
                };

                policy.apply_to(&mut *decoder);
                let _ = event_tx.send(AudioEvent::Started);

                Worker { output_configuration, reconfiguration: None, output_needs_restart: false, duration: duration_clone, checkpoints:Vec::new(), dolby_policy:policy,
                    stream,
                    decoder,
                    backend,
                    commands: command_rx,
                    events: event_tx,
                    playing: false,
                    ended: false, draining: false, queued_frontier: Duration::ZERO,
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

    /// Latest track length, refreshed after the source completes successfully.
    /// An item with no picture takes its timeline total from here.
    pub fn duration(&self) -> Option<Duration> {
        *self.duration.lock().unwrap()
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

    /// Return to the beginning while preserving the current play/pause state.
    pub fn rewind(&mut self) {
        self.generation += 1;
        let _ = self.commands.send(Command::Rewind);
    }

    /// Seek the audio clock to a stream position.
    pub fn seek(&mut self, target: Duration) {
        self.generation += 1;
        if self.timescale > 0 {
            let _ = self.commands.send(Command::SeekTime(target));
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
        while self.commands.try_send(Command::Stop).is_err() {
            while self.events.try_recv().is_ok() {}
            if self.thread.as_ref().is_none_or(|t| t.is_finished()) {
                break;
            }
            thread::sleep(Duration::from_micros(200));
        }
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() {
                while self.events.try_recv().is_ok() {}
                thread::sleep(Duration::from_micros(200));
            }
            let _ = thread.join();
        }
    }
}

enum Reconfiguration { DecoderTail, DeviceDrain }

struct Worker {
    output_configuration: Option<(crate::audio::AudioSpec, Option<fvid_media::owned_stereo_router::StereoRouter>)>,
    reconfiguration: Option<Reconfiguration>,
    output_needs_restart: bool,
    duration: Arc<Mutex<Option<Duration>>>,
    checkpoints:Vec<(i64,crate::audio::AudioCheckpoint)>,
    dolby_policy: DolbyPlaybackPolicy,
    stream: Box<dyn AudioStream>,
    decoder: Box<dyn AudioDecode>,
    backend: Box<dyn AudioBackend>,
    commands: Receiver<Command>,
    events: SyncSender<AudioEvent>,
    playing: bool,
    ended: bool,
    draining: bool,
    queued_frontier: Duration,
    generation: u64,
    position: Arc<Mutex<Duration>>,
}

impl Worker {
    fn sync_output(&mut self, at: Duration) -> crate::Result<()> {
        let spec = crate::audio::AudioSpec {
            sample_rate: self.stream.sample_rate(),
            channels: self.stream.channels(),
            format: crate::audio::SampleFormat::F32,
        };
        let router = self.stream.stereo_router()?;
        if self.output_needs_restart
            || self
                .output_configuration
                .as_ref()
                .is_some_and(|current| current.0 != spec || current.1.as_ref() != router.as_ref())
        {
            self.backend
                .reconfigure(spec, router.clone(), at, self.playing)
                .map_err(|e| crate::invalid(&e.to_string()))?;
        }
        self.output_needs_restart = false;
        self.output_configuration = Some((spec, router));
        Ok(())
    }
    fn reconfigure_next(&mut self) -> (Option<AudioEvent>, Duration) {
        let result: crate::Result<Option<crate::audio::DecodedAudio>> = (|| {
            match self.reconfiguration {
                Some(Reconfiguration::DecoderTail) => {
                    if let Some(frame) = self.decoder.finish_packet()? {
                        return Ok(Some(frame));
                    }
                    self.backend
                        .begin_drain()
                        .map_err(|e| crate::invalid(&e.to_string()))?;
                    self.reconfiguration = Some(Reconfiguration::DeviceDrain);
                }
                Some(Reconfiguration::DeviceDrain) => {
                    if !self
                        .backend
                        .drain_status()
                        .map_err(|e| crate::invalid(&e.to_string()))?
                    {
                        return Ok(None);
                    }
                    self.stream.advance_configuration()?;
                    self.decoder = self.stream.make_decoder()?;
                    self.dolby_policy.apply_to(&mut *self.decoder);
                    self.checkpoints.clear();
                    self.sync_output(self.queued_frontier)?;
                    self.backend
                        .end_drain()
                        .map_err(|e| crate::invalid(&e.to_string()))?;
                    self.reconfiguration = None;
                }
                None => {}
            }
            Ok(None)
        })();
        match result {
            Ok(Some(frame)) => self.present_frame(frame),
            Ok(None) => (
                None,
                if self.reconfiguration.is_some() {
                    Duration::from_millis(5)
                } else {
                    Duration::ZERO
                },
            ),
            Err(error) => {
                self.reconfiguration = None;
                self.output_needs_restart = true;
                self.fail_output("configuration change", error);
                (None, Duration::ZERO)
            }
        }
    }
    fn reset_decoder(&mut self)->crate::Result<()> {
        self.stream.reset_decoder(&mut self.decoder)?;
        self.dolby_policy.apply_to(&mut *self.decoder);
        Ok(())
    }
    fn restore_preroll(&mut self)->crate::Result<()> {
        let Some(target)=self.stream.preroll_target() else {return Ok(());};
        let Some((pts,state))=self.checkpoints.iter().filter(|(p,_)|*p<=target).max_by_key(|(p,_)|*p) else {return Ok(());};
        if self.stream.resume_preroll(*pts) {
            self.decoder.restore(state)?;
            self.dolby_policy.apply_to(&mut *self.decoder);
        }
        Ok(())
    }

    fn finish_source(&mut self) -> (Option<AudioEvent>, Duration) {
        if let Err(error) = self.stream.validate_eof() {
            self.ended = true;
            return (Some(AudioEvent::Error(error.to_string())), Duration::ZERO);
        }
        if let Some(duration) = self.stream.duration() {
            if let Ok(mut published) = self.duration.lock() { *published = Some(duration); }
        }
        if let Err(error) = self.backend.begin_drain() {
            self.ended = true;
            return (Some(AudioEvent::Error(format!("Audio output drain: {error}"))), Duration::ZERO);
        }
        self.draining = true;
        self.drain_output()
    }

    fn drain_output(&mut self) -> (Option<AudioEvent>, Duration) {
        let drained = match self.backend.drain_status() {
            Ok(drained) => drained,
            Err(error) => {
                self.ended = true;
                self.draining = false;
                return (Some(AudioEvent::Error(format!("Audio output drain: {error}"))), Duration::ZERO);
            }
        };
        if let Ok(mut position) = self.position.lock() { *position = self.backend.position(); }
        if drained {
            self.ended = true;
            self.draining = false;
            (Some(AudioEvent::Ended(self.generation)), Duration::ZERO)
        } else {
            // Command-channel waits keep Pause/Seek/Stop responsive during the
            // final device buffer, without re-decoding or re-validating input.
            (None, Duration::from_millis(5))
        }
    }

    /// Decode and queue one packet, reporting a terminal event if the stream
    /// ended or failed and how long to wait before feeding the next one.
    fn decode_next(&mut self) -> (Option<AudioEvent>, Duration) {
        if self.reconfiguration.is_some() { return self.reconfigure_next(); }
        if self.draining { return self.drain_output(); }
        let step = match self.stream.next_step() {
            Ok(Some(p)) => p,
            Ok(None) => {
                if !self.stream.drain_decoded_at_eof() {
                    self.decoder.reset();
                    return self.finish_source();
                }
                return match self.decoder.finish_packet() {
                    Ok(Some(frame)) => self.present_frame(frame),
                    Ok(None) => self.finish_source(),
                    Err(error) => {
                        self.ended = true;
                        (Some(AudioEvent::Error(error.to_string())), Duration::ZERO)
                    }
                };
            }
            Err(e) => {
                self.ended = true;
                return (Some(AudioEvent::Error(e.to_string())), Duration::ZERO);
            }
        };

        let (decoded, source_end) = match step {
            crate::audio::AudioStep::ReconfigureDecoder => {
                self.reconfiguration = Some(Reconfiguration::DecoderTail);
                return self.reconfigure_next();
            }
            crate::audio::AudioStep::ResetDecoder => {
                if let Err(e)=self.reset_decoder().and_then(|_|self.restore_preroll()) {self.ended=true;return (Some(AudioEvent::Error(e.to_string())),Duration::ZERO);}
                return (None,Duration::ZERO);
            }
            crate::audio::AudioStep::Pcm(packet) => (packet,None),
            crate::audio::AudioStep::Encoded(packet) => {
                let stamp=self.stream.decoder_stamp(&packet);
                if self.stream.preroll_target().is_some() && self.stream.timescale()!=0 {
                    let bucket=self.stream.checkpoint_bucket(stamp);
                    if !self.checkpoints.iter().any(|(p,_)|self.stream.checkpoint_bucket(*p)==bucket) {
                        if let Some(state)=self.decoder.checkpoint() {
                            if self.checkpoints.len()==32 {self.checkpoints.remove(0);}
                            self.checkpoints.push((stamp,state));
                        }
                    }
                }

                return match self.decoder.decode_packet(&packet.data,stamp,packet.duration.max(0) as u64) {
                    Ok(Some(frame)) => self.present_frame(frame),
                    Ok(None) => (None,Duration::ZERO),
                    Err(error) => {self.ended=true;(Some(AudioEvent::Error(error.to_string())),Duration::ZERO)},
                };
            }
        };
        self.queue_pcm(decoded,source_end)
    }

    fn present_frame(&mut self,frame:crate::audio::DecodedAudio)->(Option<AudioEvent>,Duration) {
        let mut decoded=frame.packet;

        match self.stream.decoded_packet_sample_limit(frame.source_pts,frame.source_duration) {
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

        let decoded = match self.stream.present_decoded(decoded, frame.source_pts) {
            Ok(Some(packet)) => packet,
            Ok(None) => return (None,Duration::ZERO),
            Err(error) => {
                self.ended = true;
                return (Some(AudioEvent::Error(error.to_string())),Duration::ZERO);
            }
        };

        let end=i64::try_from(frame.source_duration).unwrap_or(i64::MAX);
        self.queue_pcm(decoded,Some(frame.source_pts.max(0).saturating_add(end)))
    }

    fn queue_pcm(&mut self,decoded:crate::audio::AudioPacket,_source_end:Option<i64>)->(Option<AudioEvent>,Duration) {
        let frontier = {
            let stride = usize::from(self.stream.channels()) * 4;
            let rate = self.stream.sample_rate();
            if stride == 0 || rate == 0 || !decoded.data.len().is_multiple_of(stride) {
                self.ended = true;
                return (Some(AudioEvent::Error("invalid decoded PCM geometry".into())),Duration::ZERO);
            }
            let frames = decoded.data.len()/stride;
            let nanos = frames as u128 * 1_000_000_000/u128::from(rate);
            decoded.presentation_time().saturating_add(Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX)))
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

        self.queued_frontier = frontier;
        (None, pacing(frontier, device_pos))
    }

    /// Terminate a failed device transition before publishing a new clock.
    fn fail_output(&mut self, operation: &str, error: impl std::fmt::Display) {
        self.playing = false;
        self.ended = true;
        self.draining = false;
        let message = match self.backend.stop() {
            Ok(()) => format!("Audio {operation}: {error}"),
            Err(stop) => format!("Audio {operation}: {error}; stopping device: {stop}"),
        };
        let _ = self.events.send(AudioEvent::Error(message));
    }

    /// Run the device from wherever its clock currently sits. Play is
    /// idempotent because the caller commands it at every resume as well as at
    /// the first presentation.
    fn resume_clock(&mut self) -> bool {
        if self.playing {
            return true;
        }
        if let Err(error) = self.backend.resume() {
            self.fail_output("resume", error);
            return false;
        }
        self.playing = true;
        true
    }

    fn finish_seek(&mut self,result_pts:i64)->bool {
                if let Err(error) = self.reset_decoder().and_then(|_|self.restore_preroll()) {
                    self.fail_output("seek decoder reset", error);
                    return false;
                }
                // Anchor the clock at the sample actually landed on, which can
                // precede the requested point; the backend advances from there.
                let anchor = self.stream.time_of(result_pts);
                if let Err(error) = self.backend.flush(anchor) {
                    self.fail_output("seek flush", error);
                    return false;
                }
                if let Err(error) = self.sync_output(anchor) {
                    self.fail_output("seek output configuration", error);return false;
                }
                self.queued_frontier = anchor;

                if let Ok(mut pos) = self.position.lock() {
                    *pos = anchor;
                }
        true
    }

    fn drain_commands(&mut self, first: Option<Command>) -> bool {
        enum DeferredNav {
            Rewind,
            #[cfg(test)]
            Seek(i64),
            SeekTime(Duration),
        }
        let mut nav: Option<DeferredNav> = None;
        let mut latest_level: Option<u32> = None;
        let mut latest_tempo: Option<u32> = None;
        let mut next = first;
        loop {
            let cmd = match next.take() {
                Some(c) => c,
                None => match self.commands.try_recv() {
                    Ok(c) => c,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return false,
                },
            };
            match cmd {
                Command::Play => { if !self.resume_clock() { return false; } },
                Command::Pause => {
                    if self.playing {
                        self.playing = false;
                        if let Err(error) = self.backend.pause() {
                            self.fail_output("pause", error);
                            return false;
                        }
                    }
                }
                Command::Rewind => {
                    self.generation += 1;
                    self.ended = false;
                    self.draining = false;
                    self.reconfiguration = None;
                    nav = Some(DeferredNav::Rewind);
                }
                Command::SeekTime(target) => {
                    self.generation += 1;self.ended=false;self.draining=false;self.reconfiguration=None;
                    nav=Some(DeferredNav::SeekTime(target));
                }
                #[cfg(test)]
                Command::Seek(pts) => {
                    self.generation += 1;
                    self.ended = false;
                    self.draining = false;
                    self.reconfiguration = None;
                    nav = Some(DeferredNav::Seek(pts));
                }
                Command::Level(milli) => latest_level = Some(milli),
                Command::Tempo(milli) => latest_tempo = Some(milli),
                Command::Stop => return false,
            }
        }
        if let Some(milli) = latest_level {
            if let Err(error) = self.backend.set_volume(milli) {
                self.fail_output("volume", error);
                return false;
            }
        }
        if let Some(milli) = latest_tempo {
            if let Err(error) = self.backend.set_rate(milli) {
                self.fail_output("rate", error);
                return false;
            }
        }
        match nav {
            Some(DeferredNav::Rewind) => {
                self.stream.rewind();
                if let Err(error) = self.reset_decoder() {
                    self.fail_output("rewind decoder reset", error);
                    return false;
                }
                if let Err(error) = self.backend.flush(Duration::ZERO) {
                    self.fail_output("rewind flush", error);
                    return false;
                }
                if let Err(error) = self.sync_output(Duration::ZERO) {
                    self.fail_output("rewind output configuration", error);return false;
                }
                self.queued_frontier = Duration::ZERO;

                if let Ok(mut pos) = self.position.lock() {
                    *pos = Duration::ZERO;
                }
            }
            Some(DeferredNav::SeekTime(target)) => {
                let at=self.stream.seek_time(target);if !self.finish_seek(at){return false;}
            }
            #[cfg(test)]
            Some(DeferredNav::Seek(pts)) => {
                let at=self.stream.seek_to(pts);if !self.finish_seek(at){return false;}
            }
            None => {}
        }
        true
    }

    fn run(mut self) {
        loop {
            // Drain commands before doing work.
            if !self.drain_commands(None) {
                return;
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
                    let started = std::time::Instant::now();
                    let mut remaining = pace;
                    while !remaining.is_zero() {
                        match self.commands.recv_timeout(remaining.min(Duration::from_millis(5))) {
                            Ok(command) => {
                                if !self.drain_commands(Some(command)) {
                                    return;
                                }
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                if let Ok(mut position) = self.position.lock() {
                                    *position = self.backend.position();
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                        remaining = if self.draining {
                            pace.saturating_sub(started.elapsed())
                        } else {
                            pacing(self.queued_frontier, self.backend.position())
                        };
                    }
                }
                continue;
            }

            // Nothing to do until commanded.
            match self.commands.recv() {
                Ok(command) => {
                    if !self.drain_commands(Some(command)) {
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
    /// Independent PCM oracle for a seek inside a saved nanosecond packet.
    /// Trim sample boundaries directly; do not call production seek helpers.
    pub(super) fn matroska_suffix(packets:&[(u64,Vec<u8>)],target:u64,rate:u32,channels:u16)->Vec<(u64,Vec<u8>)>{
        // These authored/legacy controls begin on the zero sample grid. Their
        // integer ns PTS are rational-floor projections, not fractional samples.
        assert_eq!(packets.first().map(|p|p.0),Some(0));
        let stride=usize::from(channels)*4;
        let target_sample=(u128::from(target)*u128::from(rate)).div_ceil(1_000_000_000);
        packets.iter().filter_map(|(pts,data)|{
            let scaled=u128::from(*pts)*u128::from(rate);
            let sample=(scaled+500_000_000)/1_000_000_000;
            assert!(scaled.abs_diff(sample*1_000_000_000)<=2*u128::from(rate));
            let frames=data.len()/stride;
            let skip=target_sample.saturating_sub(sample).min(frames as u128)as usize;
            (skip<frames).then(||(((sample+skip as u128)*1_000_000_000/u128::from(rate))as u64,data[skip*stride..].to_vec()))
        }).collect()
    }

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
        let (_command_tx, commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(), stream:Box::new(stream),decoder,backend:Box::new(Capture(counts.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
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
        worker.drain_commands(Some(Command::Seek(2400)));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = counts.lock().unwrap();
        assert_eq!(packets[0].pts,2400);
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected[2400*8..],"seek PCM differs from continuous decode");
        drop(packets);
        counts.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Seek(0)));
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
        let (_command_tx, commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(), stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let expected:Vec<u8> = captured.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        captured.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Seek(2400)));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = captured.lock().unwrap();
        assert_eq!(packets[0].pts,2400);
        let actual:Vec<u8> = packets.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected[2400*8..],"un-edited seek PCM differs from continuous decode");
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
            let worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(), stream:Box::new(stream),decoder:Box::new(Decoder { inner,commands:worker_commands,calls:worker_calls }),backend:Box::new(Backend(paused_tx)),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
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
        let (_command_tx, commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(), stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        for _ in 0..frames { assert!(worker.decode_next().0.is_none()); }
        let expected:Vec<u8> = captured.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        for (request,landed) in [(3000,3000),(0,0)] {
            captured.lock().unwrap().clear();
            worker.drain_commands(Some(Command::Seek(request)));
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
        let (_command_tx, commands) = sync_channel(1);
        let (events,_) = sync_channel(1);
        let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(), stream:Box::new(stream),decoder,backend:Box::new(Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO)) };
        let mut final_pace = Duration::ZERO;
        for _ in 0..48 {
            let (event,pace) = worker.decode_next();
            assert!(event.is_none());
            final_pace = pace;
        }
        let packets = captured.lock().unwrap();
        let last = packets.last().unwrap();
        // The fixture has Matroska tail padding: pacing follows the audible
        // PCM window, not the untrimmed 1024-sample coded access unit.
        let frames=last.data.len()/4/usize::from(worker.stream.channels());
        let expected_end = last.presentation_time()+Duration::from_nanos(frames as u64*1_000_000_000/u64::from(worker.stream.sample_rate()));
        assert_eq!(final_pace,expected_end.saturating_sub(LEAD));
        drop(packets);
        let expected:Vec<(u64,Vec<u8>)> = captured.lock().unwrap().iter().map(|p| { assert_eq!(p.timebase_den,1_000_000_000); (p.pts,p.data.clone()) }).collect();
        captured.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Seek(50_000_000)));
        for _ in 0..48 { assert!(worker.decode_next().0.is_none()); }
        let packets = captured.lock().unwrap();
        let wanted=super::presentation_window_tests::matroska_suffix(&expected,50_000_000,worker.stream.sample_rate(),worker.stream.channels());
        assert_eq!(packets[0].pts,50_000_000);
        assert_eq!(packets.len(),wanted.len());
        for (actual,(pts,data))in packets.iter().zip(&wanted){
            assert_eq!(actual.pts,*pts);assert_eq!(actual.timebase_den,1_000_000_000);
            assert!(actual.data==*data,"Matroska exact seek PCM differs from continuous decode");
        }
    }
}

#[cfg(test)]
mod scheduling_step_tests {
    use super::*;
    use crate::audio::EncodedPacket;
    struct Steps(std::collections::VecDeque<crate::audio::AudioStep>);
    impl AudioStream for Steps {
        fn codec(&self)->&str {"mp4a"}
        fn timescale(&self)->u32 {48000}
        fn sample_rate(&self)->u32 {48000}
        fn channels(&self)->u16 {2}
        fn extra_data(&self)->&[u8] {&[]}
        fn audio_tracks(&self)->Vec<crate::audio::AudioTrack> {Vec::new()}
        fn next_packet(&mut self)->crate::Result<Option<EncodedPacket>> {panic!("scheduled PCM must not enter encoded packet path")}
        fn next_step(&mut self)->crate::Result<Option<crate::audio::AudioStep>> {Ok(self.0.pop_front())}
        fn rewind(&mut self) {}
        fn seek_to(&mut self,pts:i64)->i64 {pts}
    }
    struct Decoder(Arc<Mutex<usize>>);
    impl crate::audio::AudioDecode for Decoder {
        fn decode_encoded(&mut self,_:&[u8],_:u64,_:u64)->crate::Result<Option<crate::audio::AudioPacket>> {panic!("silence must bypass codec")}
        fn reset(&mut self) {*self.0.lock().unwrap()+=1;}
    }
    #[test]
    fn silence_and_reset_preserve_queued_pcm_and_timestamps() {
        use crate::audio::{AudioPacket,AudioStep};
        let pcm=|pts| AudioStep::Pcm(AudioPacket {data:vec![0;960*8],pts,timebase_num:1,timebase_den:48000});
        let steps=Steps([pcm(0),AudioStep::ResetDecoder,pcm(960)].into());
        let counts=Arc::new(Mutex::new(Vec::new()));let resets=Arc::new(Mutex::new(0));
        let (_command_tx, commands)=sync_channel(1);let(events,_)=sync_channel(1);
        let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(steps),decoder:Box::new(Decoder(resets.clone())),backend:Box::new(super::presentation_window_tests::Capture(counts.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
        assert!(worker.decode_next().0.is_none());
        let (event,pace)=worker.decode_next();assert!(event.is_none());assert_eq!(pace,Duration::ZERO);
        assert_eq!(*resets.lock().unwrap(),1);
        assert!(worker.decode_next().0.is_none());
        let packets=counts.lock().unwrap();assert_eq!(packets.len(),2);assert_eq!(packets[0].pts,0);assert_eq!(packets[1].pts,960);
    }
}

#[cfg(test)]
mod mp4_edit_scheduler_tests {
    use super::*;
    #[test]
    fn gap_and_repeat_playback_matches_owned_pcm_export() {
        exercise(include_bytes!("../tests/fixtures/playback-errors/aac-gap-repeat.m4a"));
    }
    #[test]
    fn nonzero_media_range_repeats_restore_aac_overlap_before_seek() {
        let edited=include_bytes!("../tests/fixtures/playback-errors/aac-gap-repeat-offset.m4a");
        let mut continuous=Vec::new();
        crate::native_media::decode_mp4_aac_pcm(include_bytes!("../tests/fixtures/playback-errors/aac-no-edit.m4a"),&mut continuous).unwrap();
        let mut expected=vec![0;960*8];
        expected.extend_from_slice(&continuous[4800*8..9600*8]);
        expected.extend_from_slice(&continuous[4800*8..9600*8]);
        let mut rendered=Vec::new();crate::native_media::decode_mp4_aac_pcm(edited,&mut rendered).unwrap();
        assert!(rendered==expected,"nonzero edits must select the correct source samples after overlap preroll");
        exercise(edited);
    }
    fn exercise(data:&'static [u8]) {
        let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(data),Default::default()).unwrap();
        let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let packets=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let(events,_)=sync_channel(1);
        let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false, duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(packets.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
        for _ in 0..200 {
            let(event,_)=worker.decode_next();
            if let Some(event)=event {assert!(matches!(event,AudioEvent::Ended(_)),"unexpected playback error");break;}
        }
        assert!(worker.ended);
        let rendered=packets.lock().unwrap();let mut cursor=0;
        for packet in rendered.iter() {assert_eq!(packet.pts,cursor);cursor+=(packet.data.len()/8) as u64;}
        assert_eq!(cursor,10560);
        let actual:Vec<_>=rendered.iter().flat_map(|p|p.data.iter().copied()).collect();
        let mut expected=Vec::new();crate::native_media::decode_mp4_aac_pcm(data,&mut expected).unwrap();
        assert!(actual==expected,"AAC edited playback differs from PCM export");
        drop(rendered);
        for request in [480usize,5760,7000,2000,0,10560] {
            packets.lock().unwrap().clear();
            worker.drain_commands(Some(Command::Seek(request as i64)));
            assert_eq!(*worker.position.lock().unwrap(),Duration::from_nanos(request as u64*1_000_000_000/48000));
            for _ in 0..200 {
                let(event,_)=worker.decode_next();
                if let Some(event)=event {assert!(matches!(event,AudioEvent::Ended(_)),"edited AAC seek failed");break;}
            }
            assert!(worker.ended);
            let rendered=packets.lock().unwrap();
            if request<10560 {assert_eq!(rendered[0].pts,request as u64);}
            let tail:Vec<_>=rendered.iter().flat_map(|p|p.data.iter().copied()).collect();
            assert!(tail==expected[request*8..],"PCM after seek to sample {request} differs from continuous playback");
        }
        packets.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Rewind));
        for _ in 0..200 {if worker.decode_next().0.is_some() {break;}}
        let replay:Vec<_>=packets.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(replay==expected,"AAC edit rewind differs from continuous playback");
    }
}

#[cfg(test)]
mod checkpoint_seek_tests {
    use super::*;
    #[test]
    fn warmed_adts_and_matroska_seek_restore_exact_pcm_tails() {
        let adts=crate::playback_aac::AacAudioReader::open(std::io::Cursor::new(include_bytes!("../tests/fixtures/playback-errors/aac-seek-checkpoints.aac")),Default::default()).unwrap();
        let mka=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(include_bytes!("../tests/fixtures/playback-errors/aac-seek-checkpoints.mka")),Default::default()).unwrap();
        for stream in [Box::new(adts) as Box<dyn AudioStream>,Box::new(mka)] {
            let scale=stream.timescale();
            let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
            let packets=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let(events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream,decoder,backend:Box::new(super::presentation_window_tests::Capture(packets.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            fn finish(w:&mut Worker)->usize {
                for n in 0..300 {match w.decode_next().0 {Some(AudioEvent::Ended(_))=>return n,Some(AudioEvent::Error(e))=>panic!("{e}"),_=>{}}}
                panic!("audio did not end");
            }
            let full=finish(&mut worker);assert!(worker.checkpoints.len()>=2);
            let expected:Vec<_>=packets.lock().unwrap().iter().map(|p|(p.pts,p.data.clone())).collect();
            packets.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(i64::from(scale)*17/10)));
            let steps=finish(&mut worker);assert!(steps<full*3/4,"preroll was not skipped: {steps}/{full}");
            let actual=packets.lock().unwrap();
            if scale<1_000_000_000 {
                let request=u64::from(scale)*17/10;assert_eq!(actual[0].pts,request);
                let full:Vec<_>=expected.iter().flat_map(|(_,data)|data.iter().copied()).collect();let suffix:Vec<_>=actual.iter().flat_map(|p|p.data.iter().copied()).collect();
                assert!(suffix==full[request as usize*usize::from(worker.stream.channels())*4..],"checkpoint seek changed exact ADTS suffix");
            } else {
                let wanted=super::presentation_window_tests::matroska_suffix(&expected,1_700_000_000,worker.stream.sample_rate(),worker.stream.channels());
                assert_eq!(actual.len(),wanted.len());
                for(p,(pts,data))in actual.iter().zip(&wanted){assert_eq!(p.pts,*pts);assert!(p.data==*data,"checkpoint seek changed the exact Matroska suffix");}
            }
            drop(actual);packets.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(0)));finish(&mut worker);
            let actual=packets.lock().unwrap();assert_eq!(actual.len(),expected.len());
            for(p,(pts,data))in actual.iter().zip(&expected) {assert_eq!(p.pts,*pts);assert!(p.data==*data);}
        }
    }
    #[test]
    fn warmed_aac_seek_skips_source_prefix_without_changing_pcm() {
        let data=include_bytes!("../tests/fixtures/audio/two-audio.mp4").as_slice();
        let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(data),Default::default()).unwrap();
        let scale=stream.timescale();let channels=usize::from(stream.channels());
        let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
        let packets=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let(events,_)=sync_channel(1);
        let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(packets.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
        fn finish(worker:&mut Worker)->usize {
            for count in 0..300 {
                match worker.decode_next().0 {
                    Some(AudioEvent::Ended(_))=>return count,
                    Some(AudioEvent::Error(e))=>panic!("{e}"),
                    _=>{}
                }
            }
            panic!("audio did not finish");
        }
        let full_steps=finish(&mut worker);
        assert!(worker.checkpoints.len()>=2,"fixture must cross checkpoint buckets");
        let expected:Vec<_>=packets.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        packets.lock().unwrap().clear();
        worker.drain_commands(Some(Command::Seek(i64::from(scale)*17/10)));
        let seek_steps=finish(&mut worker);
        assert!(seek_steps<full_steps*3/4,"checkpoint did not skip preroll: {seek_steps}/{full_steps}");
        eprintln!("AAC warm seek packet steps: {seek_steps}; full decode steps: {full_steps}");
        let captured=packets.lock().unwrap();let start=captured[0].pts as usize;
        let actual:Vec<_>=captured.iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected[start*channels*4..],"checkpoint seek changed PCM");drop(captured);
        packets.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(0)));finish(&mut worker);
        let actual:Vec<_>=packets.lock().unwrap().iter().flat_map(|p|p.data.iter().copied()).collect();
        assert!(actual==expected,"backward checkpoint seek changed PCM");
    }
}

#[cfg(test)]
mod ps_eof_tests {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn ps_worker_uses_original_unequal_windows_and_queues_eof_before_ended() {
        for bytes in [
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-timing-1024-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-timing-960-synthetic.mp4").as_slice(),
        ] {
            let stream=crate::playback_mp4_audio::Mp4AudioReader::open_at(std::io::Cursor::new(bytes),Default::default(),0).unwrap();
            let mut baseline=crate::codec::aac_ps_playback::PsAacDecoder::new(stream.extra_data(),stream.sample_rate(),stream.channels()).unwrap();
            let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
            let mut source=crate::playback_mp4_audio::Mp4AudioReader::open_at(std::io::Cursor::new(bytes),Default::default(),0).unwrap();
            let mut expected=Vec::new();
            while let Some(p)=source.next_packet().unwrap() {
                if let Some(mut f)=baseline.decode(&p.data,p.pts,p.duration as u64).unwrap() {
                    f.packet.data.truncate(f.source_duration as usize*8);expected.push(f.packet);
                }
            }
            let mut f=baseline.finish().unwrap().unwrap();f.packet.data.truncate(f.source_duration as usize*8);expected.push(f.packet);
            let captured=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let (events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:7,position:Arc::new(Mutex::new(Duration::ZERO))};
            assert!(worker.decode_next().0.is_none());assert!(captured.lock().unwrap().is_empty());
            for count in 1..=3 {assert!(worker.decode_next().0.is_none());assert!(!worker.ended);assert_eq!(captured.lock().unwrap().len(),count);}
            assert!(matches!(worker.decode_next().0,Some(AudioEvent::Ended(7))));assert!(worker.ended);
            let actual=captured.lock().unwrap();assert_eq!(actual.len(),3);
            for (a,e) in actual.iter().zip(expected) {assert_eq!(a.pts,e.pts);assert_eq!(a.data,e.data);assert_eq!(a.timebase_den,e.timebase_den);}
        }
    }
    #[test]
    fn ps_and_lc_trait_checkpoints_reject_wrong_codec_and_restore_pending_source_metadata() {
        let bytes=include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-timing-1024-synthetic.mp4");
        let mut stream=crate::playback_mp4_audio::Mp4AudioReader::open_at(std::io::Cursor::new(bytes.as_slice()),Default::default(),0).unwrap();
        let mut ps:Box<dyn AudioDecode>=Box::new(crate::codec::aac_ps_playback::PsAacDecoder::new(stream.extra_data(),48000,2).unwrap());
        let first=stream.next_packet().unwrap().unwrap();assert!(ps.decode_packet(&first.data,-1024,333).unwrap().is_none());let checkpoint=ps.checkpoint().unwrap();
        let esds=crate::container::adts::esds_for(&[0x12,0x10]).unwrap();let mut lc=crate::codec::aac_decoder::AacDecoder::new(&esds,44100,2).unwrap();let lc_checkpoint=lc.checkpoint().unwrap();
        assert!(ps.restore(&lc_checkpoint).unwrap_err().to_string().contains("codec mismatch"));assert!(lc.restore(&checkpoint).unwrap_err().to_string().contains("codec mismatch"));
        let second=stream.next_packet().unwrap().unwrap();let frame=ps.decode_packet(&second.data,8888,777).unwrap().unwrap();assert_eq!(frame.source_pts,-1024);assert_eq!(frame.source_duration,333);
        ps.restore(&checkpoint).unwrap();let replay=ps.decode_packet(&second.data,8888,777).unwrap().unwrap();assert_eq!(frame.packet.data,replay.packet.data);assert_eq!(replay.source_pts,-1024);
        let last=ps.finish_packet().unwrap().unwrap();assert_eq!(last.source_pts,8888);assert_eq!(last.source_duration,777);assert!(ps.finish_packet().unwrap().is_none());
    }
}

#[cfg(test)]
mod ps_edit_worker_tests {
    use super::*;
    use crate::audio::AudioStream;
    fn run(worker:&mut Worker) {
        for _ in 0..64 {
            match worker.decode_next().0 {
                Some(AudioEvent::Ended(_))=>return,
                Some(AudioEvent::Error(e))=>panic!("PS edited worker: {e}"),
                _=>{},
            }
        }
        panic!("PS worker did not terminate");
    }
    #[test]
    fn factory_ps_worker_accepts_silence_repeated_ranges_eof_seek_and_rewind() {
        for (edited,original) in [
            (include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-worker-repeat-1024-synthetic.mp4").as_slice(),include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-matrix-retain-synthetic.mp4").as_slice()),
            (include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-worker-repeat-960-synthetic.mp4").as_slice(),include_bytes!("../tests/fixtures/playback-errors/he-aac-sbr-ps-960-retain-synthetic.mp4").as_slice()),
        ] {
            let mut raw=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(original),Default::default()).unwrap();
            let mut baseline=crate::codec::make_audio_decoder(raw.codec(),raw.extra_data(),raw.sample_rate(),raw.channels(),raw.bits_per_sample()).unwrap();
            let mut pcm=Vec::new();while let Some(p)=raw.next_packet().unwrap() {if let Some(f)=baseline.decode_packet(&p.data,p.pts,p.duration as u64).unwrap(){pcm.extend(f.packet.data);}}
            pcm.extend(baseline.finish_packet().unwrap().unwrap().packet.data);
            let range=&pcm[720*8..3920*8];let mut expected=vec![0;1600*8];expected.extend_from_slice(range);expected.extend_from_slice(range);expected.extend(vec![0;1600*8]);assert_eq!(expected.len(),9600*8);
            let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(edited),Default::default()).unwrap();
            let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
            let captured=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let (events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            run(&mut worker);
            let check=|from:usize| {let a=captured.lock().unwrap();let mut cursor=from as u64;let mut bytes=Vec::new();for p in a.iter(){assert_eq!(p.pts,cursor);assert_eq!(p.timebase_den,48000);cursor+=p.data.len() as u64/8;bytes.extend_from_slice(&p.data);}assert_eq!(cursor,9600);assert_eq!(bytes,&expected[from*8..]);};
            check(0);
            for request in [4000,6000,9000,0] {captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(request)));run(&mut worker);check(request as usize);}
            captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Rewind));run(&mut worker);check(0);
        }
    }
    #[test]
    fn implicit_ps_mono_metadata_edits_seek_and_rewind_use_negotiated_stereo_stride() {
        for edited in [
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-export-lc-1024-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-export-lc-960-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-export-sbr-1024-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-export-sbr-960-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-unedited-lc-1024-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-unedited-lc-960-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-unedited-sbr-1024-synthetic.mp4").as_slice(),
            include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-inband-unedited-sbr-960-synthetic.mp4").as_slice(),
        ] {
            let mut expected = vec![];
            crate::native_media::decode_mp4_aac_pcm(edited, &mut expected).unwrap();
            let total=expected.len()/8;assert!(matches!(total,5760|6144|9600));
            let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(edited),Default::default()).unwrap();
            assert_eq!(stream.channels(),2);
            let decoder=stream.make_decoder().unwrap();
            let captured=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let (events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            let check=|from:usize| {let a=captured.lock().unwrap();let mut cursor=from as u64;let mut bytes=Vec::new();for p in a.iter(){assert_eq!(p.pts,cursor);assert_eq!(p.timebase_den,48000);cursor+=p.data.len() as u64/8;bytes.extend_from_slice(&p.data);}assert_eq!(cursor,total as u64);assert!(bytes==expected[from*8..],"implicit PS presentation differs from independently qualified export");};
            run(&mut worker);check(0);
            for request in [4000,(total*2/3) as i64,(total-480) as i64,0] {captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(request)));run(&mut worker);check(request as usize);}
            captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Rewind));run(&mut worker);check(0);
        }
    }
    #[test]
    fn core_clock_ps_mp4_uses_asc_output_rate_for_pcm_edits_seek_and_rewind() {
        let manifest: serde_json::Value=serde_json::from_str(include_str!("../tests/fixtures/playback-errors/aac-ps-core-clock-oracles.json")).unwrap();
        for case in manifest["cases"].as_array().unwrap() {
            let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
            let bytes=std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
            let mut expected=vec![];
            crate::native_media::decode_mp4_aac_pcm(&std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap(),&mut expected).unwrap();
            let mut owned=vec![];
            let stats=fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(std::io::Cursor::new(&bytes),&mut owned,None,&fvid_control::CopyOptions::default()).unwrap();
            assert_eq!((stats.sample_rate,stats.channels,stats.sample_frames),(48000,2,9600));assert!(owned==expected);
            let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap();
            assert_eq!((stream.track().sample_rate,stream.timescale(),stream.sample_rate(),stream.channels()),(48000,24000,48000,2));
            let old_error=match crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),24000,1,stream.bits_per_sample()) {
                Err(error)=>error.to_string(),Ok(_)=>panic!("ASC-only factory using raw geometry must reproduce the clock failure"),
            };
            assert!(old_error.contains("configuration disagrees with container sample rate or channels"));
            let decoder=stream.make_decoder().unwrap();
            let captured=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let (events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            let check=|from:usize| {let a=captured.lock().unwrap();let mut cursor=from as u64;let mut bytes=Vec::new();for p in a.iter(){assert_eq!(p.pts,cursor);assert_eq!(p.timebase_den,48000);cursor+=p.data.len() as u64/8;bytes.extend_from_slice(&p.data);}assert_eq!(cursor,9600);assert!(bytes==expected[from*8..],"core/output clock changes altered PCM");};
            run(&mut worker);check(0);
            for sample in [4000,6000,9000,0] {captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Seek(sample/2)));run(&mut worker);check(sample as usize);}
            captured.lock().unwrap().clear();worker.drain_commands(Some(Command::Rewind));run(&mut worker);check(0);
        }
    }
    #[test]
    fn source_gap_is_rejected_after_pending_ps_pcm_is_drained() {
        for bytes in [include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-worker-source-gap-1024-synthetic.mp4").as_slice(),include_bytes!("../tests/fixtures/playback-errors/he-aac-ps-worker-source-gap-960-synthetic.mp4").as_slice()] {
            let stream=crate::playback_mp4_audio::Mp4AudioReader::open(std::io::Cursor::new(bytes),Default::default()).unwrap();let decoder=crate::codec::make_audio_decoder(stream.codec(),stream.extra_data(),stream.sample_rate(),stream.channels(),stream.bits_per_sample()).unwrap();
            let captured=Arc::new(Mutex::new(Vec::new()));let (_command_tx, commands)=sync_channel(1);let (events,_)=sync_channel(1);
            let mut worker=Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,duration: Arc::new(Mutex::new(None)), checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(super::presentation_window_tests::Capture(captured.clone())),commands,events,playing:true,ended:false, draining: false, queued_frontier: Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            let mut saw_error=false;for _ in 0..16 {match worker.decode_next().0 {Some(AudioEvent::Error(e))=>{assert!(e.contains("AAC edit extends beyond source packets"),"{e}");saw_error=true;break;},Some(AudioEvent::Ended(_))=>panic!("silently accepted a source gap"),_=>{}}}
            assert!(saw_error);assert_eq!(captured.lock().unwrap().len(),3);
        }
    }
}

#[cfg(test)]
mod ps_matroska_preroll_tests {
    use super::*;
    use crate::audio::AudioStream;
    #[test]
    fn original_ps_matroska_preroll_is_trimmed_and_eof_seek_rewind_keep_stereo_history() {
        let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/playback-errors/aac-ps-matroska-oracles.json"
        ))
        .unwrap();
        let output_clock: serde_json::Value=serde_json::from_str(include_str!("../tests/fixtures/playback-errors/aac-ps-output-clock-matroska-oracles.json")).unwrap();
        manifest["cases"].as_array_mut().unwrap().extend(output_clock["cases"].as_array().unwrap().iter().cloned());
        let asc_clock: serde_json::Value=serde_json::from_str(include_str!("../tests/fixtures/playback-errors/aac-ps-asc-clock-matroska-oracles.json")).unwrap();
        manifest["cases"].as_array_mut().unwrap().extend(asc_clock["cases"].as_array().unwrap().iter().cloned());
        let oracle = include_bytes!("../tests/fixtures/playback-errors/aac-ps-absence-pcm.bin");
        for case in manifest["cases"].as_array().unwrap() {
            let file = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors")
                    .join(case["file"].as_str().unwrap()),
            )
            .unwrap();
            let mut video = crate::playback_webm::WebmVideoReader::open_software(
                std::io::Cursor::new(file.clone()), 1024 * 1024,
            ).unwrap();
            assert!(video.read_frame().unwrap());
            let stream = crate::playback_webm_audio::WebmAudioReader::open(
                std::io::Cursor::new(file),
                Default::default(),
            )
            .unwrap();
            assert_eq!((stream.sample_rate(),stream.channels()),(48000,2));
            let decoder = stream.make_decoder().unwrap();
            let captured = Arc::new(Mutex::new(Vec::new()));
            let (_command_tx, commands) = sync_channel(1);
            let (events, _) = sync_channel(1);
            let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,
                duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),
                stream: Box::new(stream),
                decoder,
                backend: Box::new(super::presentation_window_tests::Capture(captured.clone())),
                commands,
                events,
                playing: true,
                ended: false, draining: false, queued_frontier: Duration::ZERO,
                generation: 0,
                position: Arc::new(Mutex::new(Duration::ZERO)),
            };
            for _ in 0..4 {
                assert!(worker.decode_next().0.is_none());
            }
            assert!(matches!(worker.decode_next().0, Some(AudioEvent::Ended(_))));
            let baseline = std::mem::take(&mut *captured.lock().unwrap());
            assert_eq!(baseline.len(), 3);
            let frame_samples = case["slots"].as_u64().unwrap() as usize * 128;
            for (i, packet) in baseline.iter().enumerate() {
                let skip = if i == 0 {
                    case["first_skip_samples"].as_u64().unwrap() as usize
                } else {
                    0
                };
                assert_eq!(
                    packet.data.len(),
                    (frame_samples - skip) * 8,
                    "negative AAC preroll must be removed"
                );
                assert_eq!(
                    packet.pts,
                    case["source_pts_ns"][i].as_i64().unwrap().max(0) as u64
                );
                assert_eq!(packet.timebase_den, 1_000_000_000);
                for (n, bytes) in packet.data.chunks_exact(4).enumerate() {
                    let channel = n % 2;
                    let sample = i * frame_samples + skip + n / 2;
                    let offset = case["pcm"][channel][0].as_u64().unwrap() as usize + sample * 8;
                    let expected =
                        f64::from_le_bytes(oracle[offset..offset + 8].try_into().unwrap()) as f32;
                    let actual = f32::from_le_bytes(bytes.try_into().unwrap());
                    assert!(
                        (actual - expected).abs() <= 2. * f32::EPSILON * expected.abs() + 2e-16
                    );
                }
            }
            for (target, start) in [(40_000_000, 1usize), (0, 0)] {
                captured.lock().unwrap().clear();
                worker.drain_commands(Some(Command::Seek(target)));
                for _ in 0..5 {
                    let _ = worker.decode_next();
                }
                let actual = captured.lock().unwrap();
                assert_eq!(actual.len(), baseline.len() - start);
                for (a, e) in actual.iter().zip(&baseline[start..]) {
                    assert_eq!(a.data, e.data);
                    assert_eq!(a.pts, e.pts);
                    assert_eq!(a.timebase_den, e.timebase_den);
                }
            }
        }
    }
}

#[cfg(test)]
mod ogg_opus_configuration_tests {
    use super::*;
    use crate::audio::AudioStream;
    fn run(worker: &mut Worker) {
        for _ in 0..64 {
            match worker.decode_next().0 {
                Some(AudioEvent::Ended(_)) => return,
                Some(AudioEvent::Error(e)) => panic!("Ogg Opus worker: {e}"),
                _ => {}
            }
        }
        panic!("Ogg Opus worker did not terminate");
    }
    #[test]
    fn ogg_opus_worker_reconfigures_mapping_and_gain_after_boundaries_and_navigation() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let mut expected = Vec::new();
        for (name, trim) in [
            ("hevc-opus-surround-companion.mkv", 137),
            ("hevc-opus-reconfigured-companion.mkv", 211),
        ] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let mut pcm = Vec::new();
            fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                std::io::Cursor::new(bytes),
                &mut pcm,
                None,
                &Default::default(),
            )
            .unwrap();
            pcm.truncate(pcm.len() - trim * 24);
            expected.extend(pcm);
        }
        let bytes = std::fs::read(root.join("opus-surround-reconfigured.opus")).unwrap();
        let stream =
            crate::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default())
                .unwrap();
        let decoder = stream.make_decoder().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let (_command_tx, commands) = sync_channel(1);
        let (events, _) = sync_channel(8);
        let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,
            duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),
            stream: Box::new(stream),
            decoder,
            backend: Box::new(super::presentation_window_tests::Capture(captured.clone())),
            commands,
            events,
            playing: true,
            ended: false, draining: false, queued_frontier: Duration::ZERO,
            generation: 0,
            position: Arc::new(Mutex::new(Duration::ZERO)),
        };
        let check = |from: usize| {
            let packets = captured.lock().unwrap();
            let mut cursor = from as u64;
            let mut pcm = Vec::new();
            for packet in packets.iter() {
                assert_eq!(packet.pts, cursor);
                assert_eq!(packet.timebase_den, 48000);
                cursor += packet.data.len() as u64 / 24;
                pcm.extend_from_slice(&packet.data);
            }
            assert_eq!(cursor, 10212);
            assert_eq!(pcm, expected[from * 24..]);
        };
        run(&mut worker);
        check(0);
        for target in [7000usize, 0, 5311, 5310, 7000] {
            captured.lock().unwrap().clear();
            assert!(worker.drain_commands(Some(Command::Seek(target as i64))));
            run(&mut worker);
            check(target);
        }
        captured.lock().unwrap().clear();
        assert!(worker.drain_commands(Some(Command::Rewind)));
        run(&mut worker);
        check(0);
    }
}

#[cfg(test)]
mod ogg_opus_output_layout_tests {
    use super::*;
    use crate::audio::AudioStream;
    struct Capture {
        pcm: Arc<Mutex<Vec<crate::audio::AudioPacket>>>,
        starts: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl AudioBackend for Capture {
        fn start(&mut self, spec: crate::audio::AudioSpec) -> Result<(), crate::audio::AudioError> {
            assert_eq!(spec.channels, 6);
            assert_eq!(spec.sample_rate, 48000);
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn push(&mut self, p: crate::audio::AudioPacket) -> Result<(), crate::audio::AudioError> {
            assert_eq!(p.data.len() % 24, 0);
            self.pcm.lock().unwrap().push(p);
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
    }
    fn run(worker: &mut Worker) {
        for _ in 0..64 {
            match worker.decode_next().0 {
                Some(AudioEvent::Ended(_)) => return,
                Some(AudioEvent::Error(e)) => panic!("Opus layout worker: {e}"),
                _ => {}
            }
        }
        panic!("Opus layout worker did not terminate");
    }
    #[test]
    fn ogg_opus_channel_changes_preserve_pcm_without_restarting_the_backend() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let decode_video = |name: &str| {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let mut pcm = Vec::new();
            fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
                std::io::Cursor::new(bytes),
                &mut pcm,
                None,
                &Default::default(),
            )
            .unwrap();
            pcm
        };
        let six = decode_video("hevc-opus-surround-companion.mkv");
        let quad = decode_video("hevc-opus-quad-companion.mkv");
        let expand = |bytes: &[u8]| {
            let mut out = Vec::new();
            for row in bytes.chunks_exact(16) {
                out.extend_from_slice(&row[..8]);
                out.extend([0; 8]);
                out.extend_from_slice(&row[8..]);
            }
            out
        };
        for reverse in [false, true] {
            let name = if reverse {
                "opus-quad-to-surround.opus"
            } else {
                "opus-surround-to-quad.opus"
            };
            let mut expected = if reverse {
                expand(&quad[..5311 * 16])
            } else {
                six[..5311 * 24].to_vec()
            };
            expected.extend(if reverse {
                six[336 * 24..5237 * 24].to_vec()
            } else {
                expand(&quad[336 * 16..5237 * 16])
            });
            let bytes = std::fs::read(root.join(name)).unwrap();
            let stream =
                crate::playback_ogg_opus::OggOpusAudioReader::open(&bytes[..], Default::default())
                    .unwrap();
            let decoder = stream.make_decoder().unwrap();
            let captured = Arc::new(Mutex::new(Vec::new()));
            let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut backend = Capture {
                pcm: captured.clone(),
                starts: starts.clone(),
            };
            backend
                .start(crate::audio::AudioSpec {
                    sample_rate: stream.sample_rate(),
                    channels: stream.channels(),
                    format: crate::audio::SampleFormat::F32,
                })
                .unwrap();
            let (_tx, commands) = sync_channel(1);
            let (events, _) = sync_channel(8);
            let mut worker = Worker { output_configuration: None, reconfiguration: None, output_needs_restart: false,
                duration: Arc::new(Mutex::new(None)), checkpoints: Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),
                stream: Box::new(stream),
                decoder,
                backend: Box::new(backend),
                commands,
                events,
                playing: true,
                ended: false, draining: false, queued_frontier: Duration::ZERO,
                generation: 0,
                position: Arc::new(Mutex::new(Duration::ZERO)),
            };
            let check = |from: usize| {
                let packets = captured.lock().unwrap();
                let mut cursor = from as u64;
                let mut actual = Vec::new();
                for p in packets.iter() {
                    assert_eq!(p.pts, cursor);
                    assert_eq!(p.timebase_den, 48000);
                    cursor += p.data.len() as u64 / 24;
                    actual.extend_from_slice(&p.data);
                }
                assert_eq!(cursor, 10212);
                assert_eq!(actual, expected[from * 24..]);
                assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
            };
            run(&mut worker);
            check(0);
            for target in [7000usize, 0, 5311, 5310] {
                captured.lock().unwrap().clear();
                assert!(worker.drain_commands(Some(Command::Seek(target as i64))));
                run(&mut worker);
                check(target);
            }
            captured.lock().unwrap().clear();
            assert!(worker.drain_commands(Some(Command::Rewind)));
            run(&mut worker);
            check(0);
        }
    }
}

#[cfg(test)]
#[path = "playback/audio/vorbis_worker_tests.rs"]
mod vorbis_worker_tests;

#[cfg(test)]
mod exact_seek_clock_regressions {
    use super::*;
    #[test]
    fn duration_to_seek_ticks_preserves_the_stored_matroska_boundary() {
        let bytes=include_bytes!("../tests/fixtures/playback-errors/audio-stream-clock/clock.mkv");
        let mut source=crate::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(bytes.as_slice()),Default::default()).unwrap();
        let packet=source.next_packet().unwrap().unwrap();
        assert_eq!(packet.pts,9_000_000_000_001_000_000);
        let (commands,received)=sync_channel(QUEUE);
        let (_,events)=sync_channel(QUEUE);
        let mut playback=AudioPlayback{commands,events,generation:0,thread:None,
            position:Arc::new(Mutex::new(Duration::ZERO)),timescale:source.timescale(),duration:Arc::new(Mutex::new(None))};
        for (target,expected) in [
            (Duration::new(9_000_000_000,1_000_000),packet.pts),
            (Duration::from_nanos(1),1),
            (Duration::from_nanos(999_999_999),999_999_999),
            (Duration::MAX,i64::MAX),
        ] {
            playback.seek(target);
            match received.try_recv().unwrap() {
                Command::SeekTime(received)=>{
                    assert_eq!(received,target);
                    assert_eq!(super::sample_seek_clock_regressions::Clock(source.timescale()).seek_time(received),expected,"target={target:?}");
                },
                _=>panic!("seek did not send a navigation command"),
            }
        }
    }
}

#[cfg(test)]
mod pause_failure_regressions {
    use super::*;
    use crate::audio::{AudioError,AudioPacket,AudioSpec};
    use std::sync::atomic::{AtomicBool,Ordering};
    struct Device { pcm:Arc<Mutex<Vec<u8>>>,paused:Arc<AtomicBool>,stopped:Arc<AtomicBool>,fail:bool,fail_stop:bool }
    impl AudioBackend for Device {
        fn start(&mut self,_:AudioSpec)->Result<(),AudioError>{Ok(())}
        fn push(&mut self,p:AudioPacket)->Result<(),AudioError>{self.pcm.lock().unwrap().extend(p.data);Ok(())}
        fn position(&self)->Duration{Duration::ZERO}
        fn flush(&mut self,_:Duration)->Result<(),AudioError>{Ok(())}
        fn pause(&mut self)->Result<(),AudioError>{self.paused.store(true,Ordering::Release);if self.fail{Err(AudioError::PlaybackError("injected pause failure".into()))}else{Ok(())}}
        fn resume(&mut self)->Result<(),AudioError>{Ok(())}
        fn stop(&mut self)->Result<(),AudioError>{self.stopped.store(true,Ordering::Release);if self.fail_stop{Err(AudioError::PlaybackError("injected stop failure".into()))}else{Ok(())}}
        fn drain_status(&self)->Result<bool,AudioError>{Ok(false)}
    }
    #[test]
    fn pause_failure_reports_terminal_error_and_stops_device_after_valid_pcm() {
        let media=include_bytes!("../tests/fixtures/playback-errors/audio-pause-failure/clock.mkv");
        let expected=include_bytes!("../tests/fixtures/playback-errors/audio-pause-failure/expected-source.f32le");
        let mut video=crate::playback_native::NativeReader::software(std::io::Cursor::new(media.as_slice()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        for (fail,fail_stop) in [(false,false),(true,false),(true,true)] {
            let source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(media.as_slice()),Default::default()).unwrap();
            let pcm=Arc::new(Mutex::new(Vec::new()));let paused=Arc::new(AtomicBool::new(false));let stopped=Arc::new(AtomicBool::new(false));
            let device=Device{pcm:pcm.clone(),paused:paused.clone(),stopped:stopped.clone(),fail,fail_stop};
            let playback=AudioPlayback::start(Box::new(source),move||Box::new(device));playback.play();
            let deadline=std::time::Instant::now()+Duration::from_secs(5);
            while pcm.lock().unwrap().len()!=expected.len(){assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(1));}
            assert_eq!(pcm.lock().unwrap().as_slice(),expected);
            playback.pause();let deadline=std::time::Instant::now()+Duration::from_millis(100);
            let mut failure=None;
            while std::time::Instant::now()<deadline {
                match playback.poll(){Some(AudioEvent::Error(e))=>{failure=Some(e);break},Some(AudioEvent::Ended(_))=>panic!("pause failure must not become EOF"),_=>{}}
                thread::sleep(Duration::from_millis(1));
            }
            assert!(paused.load(Ordering::Acquire));
            if fail {
                assert!(failure.as_deref().is_some_and(|e|e.contains("injected pause failure")),"pause failure silently discarded: {failure:?}");
                assert!(stopped.load(Ordering::Acquire),"failed pause did not attempt to stop device");
                if fail_stop {assert!(failure.as_deref().unwrap().contains("injected stop failure"));}
            } else {assert!(failure.is_none());assert!(!stopped.load(Ordering::Acquire));}
        }
    }
}

#[cfg(test)]
mod flush_failure_regressions {
    use super::*;
    use crate::audio::{AudioError,AudioPacket,AudioSpec};
    use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
    struct Device { pcm:Arc<Mutex<Vec<u8>>>,total:Arc<AtomicUsize>,flushed:Arc<AtomicBool>,stopped:Arc<AtomicBool>,fail:bool }
    impl AudioBackend for Device {
        fn start(&mut self,_:AudioSpec)->Result<(),AudioError>{Ok(())}
        fn push(&mut self,p:AudioPacket)->Result<(),AudioError>{self.total.fetch_add(p.data.len(),Ordering::AcqRel);self.pcm.lock().unwrap().extend(p.data);Ok(())}
        fn position(&self)->Duration{Duration::ZERO}
        fn flush(&mut self,_:Duration)->Result<(),AudioError>{self.flushed.store(true,Ordering::Release);if self.fail{Err(AudioError::PlaybackError("injected flush failure".into()))}else{self.pcm.lock().unwrap().clear();Ok(())}}
        fn pause(&mut self)->Result<(),AudioError>{Ok(())}
        fn resume(&mut self)->Result<(),AudioError>{Ok(())}
        fn stop(&mut self)->Result<(),AudioError>{self.stopped.store(true,Ordering::Release);Ok(())}
        fn drain_status(&self)->Result<bool,AudioError>{Ok(false)}
    }
    #[test]
    fn failed_navigation_flush_cannot_mix_new_decoding_into_old_device_queue() {
        let media=include_bytes!("../tests/fixtures/playback-errors/audio-flush-failure/navigation.mkv");
        let expected=include_bytes!("../tests/fixtures/playback-errors/audio-flush-failure/expected-before.f32le");
        let mut video=crate::playback_native::NativeReader::software(std::io::Cursor::new(media.as_slice()),32<<20).unwrap();
        for _ in 0..3 {assert!(video.read_frame_raw().unwrap().is_some());}
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut ignored=Vec::new();
        for rewind in [false,true] {for fail in [false,true] {
            let source=crate::playback_webm_audio::WebmAudioReader::open(std::io::Cursor::new(media.as_slice()),Default::default()).unwrap();
            let pcm=Arc::new(Mutex::new(Vec::new()));let total=Arc::new(AtomicUsize::new(0));let flushed=Arc::new(AtomicBool::new(false));let stopped=Arc::new(AtomicBool::new(false));
            let device=Device{pcm:pcm.clone(),total:total.clone(),flushed:flushed.clone(),stopped:stopped.clone(),fail};
            let mut playback=AudioPlayback::start(Box::new(source),move||Box::new(device));playback.play();
            let deadline=std::time::Instant::now()+Duration::from_secs(5);
            while pcm.lock().unwrap().len()!=expected.len(){assert!(std::time::Instant::now()<deadline);thread::sleep(Duration::from_millis(1));}
            assert_eq!(pcm.lock().unwrap().as_slice(),expected);
            if rewind {playback.rewind();}else{playback.seek(Duration::from_millis(20));}
            let deadline=std::time::Instant::now()+Duration::from_millis(100);let mut failure=None;
            while std::time::Instant::now()<deadline {
                match playback.poll(){Some(AudioEvent::Error(e))=>{failure=Some(e);break},Some(AudioEvent::Ended(_))=>panic!("flush failure must not become EOF"),_=>{}}
                thread::sleep(Duration::from_millis(1));
            }
            assert!(flushed.load(Ordering::Acquire));
            if fail {
                if !failure.as_deref().is_some_and(|e|e.contains("injected flush failure")) {
                    ignored.push(format!("rewind={rewind}, bytes queued={}",total.load(Ordering::Acquire)));
                    continue;
                }
                assert_eq!(total.load(Ordering::Acquire),expected.len());
                assert!(stopped.load(Ordering::Acquire));
            }else{
                assert!(failure.is_none(),"{failure:?}");
                assert_eq!(pcm.lock().unwrap().as_slice(),if rewind {expected.as_slice()} else {&expected[640..]});
            }
        }}
        assert!(ignored.is_empty(),"flush refusals were ignored: {ignored:?}");
    }
}

#[cfg(test)]
mod duration_publication_guards {
    use super::*;
    struct Source { value:Option<Duration>, valid:bool }
    impl AudioStream for Source {
        fn codec(&self)->&str {"A_PCM/FLOAT/IEEE"}
        fn timescale(&self)->u32 {48000}
        fn sample_rate(&self)->u32 {48000}
        fn channels(&self)->u16 {1}
        fn bits_per_sample(&self)->u16 {32}
        fn duration(&self)->Option<Duration> {self.value}
        fn extra_data(&self)->&[u8] {&[]}
        fn audio_tracks(&self)->Vec<crate::audio::AudioTrack> {Vec::new()}
        fn next_packet(&mut self)->crate::Result<Option<crate::audio::EncodedPacket>> {Ok(None)}
        fn rewind(&mut self) {}
        fn seek_to(&mut self,pts:i64)->i64 {pts}
        fn validate_eof(&self)->crate::Result<()> {if self.valid {Ok(())}else{Err(crate::error::invalid("injected incomplete source"))}}
    }
    #[test]
    fn completion_publication_preserves_known_length_on_unknown_or_invalid_eof() {
        for (value,valid,expected) in [(None,true,60),(Some(Duration::ZERO),true,0),(Some(Duration::from_millis(20)),false,60)] {
            let stream=Source{value,valid};let decoder=stream.make_decoder().unwrap();
            let duration=Arc::new(Mutex::new(Some(Duration::from_millis(60))));
            let (_tx,commands)=sync_channel(1);let(events,_rx)=sync_channel(1);
            let mut worker=Worker{ output_configuration: None, reconfiguration: None, output_needs_restart: false,duration:duration.clone(),checkpoints:Vec::new(), dolby_policy: DolbyPlaybackPolicy::default(),stream:Box::new(stream),decoder,backend:Box::new(crate::audio::NullBackend::default()),commands,events,playing:true,ended:false,draining:false,queued_frontier:Duration::ZERO,generation:0,position:Arc::new(Mutex::new(Duration::ZERO))};
            let result=worker.finish_source().0;
            assert_eq!(*duration.lock().unwrap(),Some(Duration::from_millis(expected)));
            assert_eq!(matches!(result,Some(AudioEvent::Error(_))),!valid);
        }
    }
}

#[cfg(test)]
mod sample_seek_clock_regressions {
    use super::*;
    pub(super) struct Clock(pub u32);
    impl AudioStream for Clock {
        fn codec(&self)->&str {"A_PCM/FLOAT/IEEE"}
        fn timescale(&self)->u32 {self.0}
        fn sample_rate(&self)->u32 {self.0}
        fn channels(&self)->u16 {1}
        fn extra_data(&self)->&[u8] {&[]}
        fn audio_tracks(&self)->Vec<crate::audio::AudioTrack> {Vec::new()}
        fn next_packet(&mut self)->crate::Result<Option<crate::audio::EncodedPacket>> {Ok(None)}
        fn rewind(&mut self) {}
        fn seek_to(&mut self,pts:i64)->i64 {pts}
    }

    #[test]
    fn inverse_seek_recovers_floored_sample_boundaries_and_keeps_other_targets_floored() {
        for scale in [8000u32,44100,48000,1_000_000_000] {
            let (commands,received)=sync_channel(QUEUE);let (_,events)=sync_channel(QUEUE);
            let mut playback=AudioPlayback{commands,events,generation:0,thread:None,position:Arc::new(Mutex::new(Duration::ZERO)),timescale:scale,duration:Arc::new(Mutex::new(None))};
            for tick in [0u64,1,2,7,2816,12032,9_000_000_000_000_000_000,i64::MAX as u64] {
                let rate=u64::from(scale);let target=Duration::new(tick/rate,((tick%rate)*1_000_000_000/rate) as u32);
                playback.seek(target);match received.try_recv().unwrap(){Command::SeekTime(at)=>{assert_eq!(at,target);assert_eq!(Clock(scale).seek_time(at),tick as i64,"rate{scale} tick{tick}")},_=>panic!("seek command")}
            }
            if scale==48000 {for (ns,expected) in [(1,0),(20832,0),(20833,1),(20834,1),(41665,1),(41666,2)] {
                playback.seek(Duration::from_nanos(ns));match received.try_recv().unwrap(){Command::SeekTime(at)=>{assert_eq!(at,Duration::from_nanos(ns));assert_eq!(Clock(scale).seek_time(at),expected,"fractional target{ns}")},_=>panic!("seek command")}
            }}
        }
    }
}
