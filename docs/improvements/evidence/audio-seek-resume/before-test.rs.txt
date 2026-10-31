//! Device resume must observe the selected source clock, including command bursts.
use super::*;
use std::{io::Cursor, path::PathBuf};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/audio-seek-resume")
            .join(name),
    )
    .unwrap()
}
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Flush(Duration),
    Resume(Duration),
    Pause,
}
#[derive(Default)]
struct Trace {
    actions: Vec<Action>,
    pcm: Vec<u8>,
}
struct Device {
    trace: Arc<Mutex<Trace>>,
    at: Duration,
}
impl AudioBackend for Device {
    fn start(&mut self, spec: crate::audio::AudioSpec) -> Result<(), crate::audio::AudioError> {
        assert_eq!((spec.sample_rate, spec.channels), (8000, 1));
        Ok(())
    }
    fn push(&mut self, p: crate::audio::AudioPacket) -> Result<(), crate::audio::AudioError> {
        self.at = p.presentation_time() + Duration::from_nanos(p.data.len() as u64 / 4 * 125000);
        self.trace.lock().unwrap().pcm.extend(p.data);
        Ok(())
    }
    fn position(&self) -> Duration {
        self.at
    }
    fn flush(&mut self, at: Duration) -> Result<(), crate::audio::AudioError> {
        self.at = at;
        let mut t = self.trace.lock().unwrap();
        t.actions.push(Action::Flush(at));
        t.pcm.clear();
        Ok(())
    }
    fn pause(&mut self) -> Result<(), crate::audio::AudioError> {
        self.trace.lock().unwrap().actions.push(Action::Pause);
        Ok(())
    }
    fn resume(&mut self) -> Result<(), crate::audio::AudioError> {
        self.trace
            .lock()
            .unwrap()
            .actions
            .push(Action::Resume(self.at));
        Ok(())
    }
    fn stop(&mut self) -> Result<(), crate::audio::AudioError> {
        Ok(())
    }
}
fn stream(codec: &str) -> Box<dyn AudioStream> {
    Box::new(
        crate::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(&format!("{codec}.mp4"))),
            Default::default(),
        )
        .unwrap(),
    )
}
fn worker(codec: &str) -> (Worker, SyncSender<Command>, Arc<Mutex<Trace>>) {
    let stream = stream(codec);
    let decoder = stream.make_decoder().unwrap();
    let trace = Arc::new(Mutex::new(Trace::default()));
    let (tx, commands) = sync_channel(16);
    let (events, _) = sync_channel(16);
    let worker = Worker {
        output_configuration: None,
        reconfiguration: None,
        output_needs_restart: false,
        duration: Arc::new(Mutex::new(stream.duration())),
        checkpoints: Vec::new(),
        dolby_policy: DolbyPlaybackPolicy::default(),
        stream,
        decoder,
        backend: Box::new(Device {
            trace: trace.clone(),
            at: Duration::ZERO,
        }),
        commands,
        events,
        playing: false,
        ended: false,
        draining: false,
        queued_frontier: Duration::ZERO,
        generation: 0,
        position: Arc::new(Mutex::new(Duration::ZERO)),
    };
    (worker, tx, trace)
}
fn finish(w: &mut Worker) {
    for _ in 0..32 {
        match w.decode_next().0 {
            Some(AudioEvent::Ended(g)) => {
                assert_eq!(g, w.generation);
                return;
            }
            Some(AudioEvent::Error(e)) => panic!("{e}"),
            _ => {}
        }
    }
    panic!("source did not finish");
}
#[test]
fn authored_video_and_continuous_pcm_controls() {
    for codec in ["ima", "alac"] {
        let mut video = crate::playback_native::NativeReader::software(
            Cursor::new(bytes(&format!("{codec}.mp4"))),
            32 << 20,
        )
        .unwrap();
        let mut n = 0;
        while video.read_frame_raw().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        let (mut w, _tx, trace) = worker(codec);
        finish(&mut w);
        assert_eq!(trace.lock().unwrap().pcm, bytes(&format!("{codec}.f32le")));
    }
}
#[test]
fn separate_seek_and_play_commands_resume_at_selected_sample() {
    for codec in ["ima", "alac"] {
        let (mut w, _tx, trace) = worker(codec);
        let target = Duration::from_millis(if codec == "ima" { 8 } else { 20 });
        assert!(w.drain_commands(Some(Command::SeekTime(target))));
        assert!(w.drain_commands(Some(Command::Play)));
        assert_eq!(
            trace.lock().unwrap().actions,
            [Action::Flush(target), Action::Resume(target)]
        );
        finish(&mut w);
        assert_eq!(
            trace.lock().unwrap().pcm,
            bytes(&format!("{codec}.f32le"))[if codec == "ima" { 64 * 4 } else { 160 * 4 }..]
        );
    }
}
#[test]
fn coalesced_seek_and_play_resume_only_after_selected_clock_is_installed() {
    for codec in ["ima", "alac"] {
        let (mut w, tx, trace) = worker(codec);
        let target = Duration::from_millis(if codec == "ima" { 8 } else { 20 });
        tx.send(Command::Play).unwrap();
        assert!(w.drain_commands(Some(Command::SeekTime(target))));
        assert_eq!(
            trace.lock().unwrap().actions,
            [Action::Flush(target), Action::Resume(target)],
            "{codec}: backend resumed from stale clock"
        );
        assert!(w.playing);
        assert_eq!(w.generation, 1);
        finish(&mut w);
        assert_eq!(
            trace.lock().unwrap().pcm,
            bytes(&format!("{codec}.f32le"))[if codec == "ima" { 64 * 4 } else { 160 * 4 }..]
        );
    }
}
#[test]
fn coalesced_rewind_and_play_resume_from_zero() {
    let (mut w, tx, trace) = worker("ima");
    let old = Duration::from_millis(8);
    assert!(w.drain_commands(Some(Command::SeekTime(old))));
    trace.lock().unwrap().actions.clear();
    tx.send(Command::Play).unwrap();
    assert!(w.drain_commands(Some(Command::Rewind)));
    assert_eq!(
        trace.lock().unwrap().actions,
        [
            Action::Flush(Duration::ZERO),
            Action::Resume(Duration::ZERO)
        ]
    );
    finish(&mut w);
    assert_eq!(trace.lock().unwrap().pcm, bytes("ima.f32le"));
}

#[test]
fn queued_play_seek_play_uses_only_the_final_navigation_clock() {
    let (mut w, tx, trace) = worker("ima");
    let target = Duration::from_millis(8);
    tx.send(Command::SeekTime(target)).unwrap();
    tx.send(Command::Play).unwrap();
    assert!(w.drain_commands(Some(Command::Play)));
    assert_eq!(
        trace.lock().unwrap().actions,
        [Action::Flush(target), Action::Resume(target)]
    );
    finish(&mut w);
    assert_eq!(trace.lock().unwrap().pcm, bytes("ima.f32le")[64 * 4..]);
}

#[test]
fn final_pause_in_seek_burst_does_not_start_the_device() {
    let (mut w, tx, trace) = worker("ima");
    let target = Duration::from_millis(8);
    tx.send(Command::Play).unwrap();
    tx.send(Command::Pause).unwrap();
    assert!(w.drain_commands(Some(Command::SeekTime(target))));
    assert!(!w.playing);
    assert_eq!(trace.lock().unwrap().actions, [Action::Flush(target)]);
    assert!(trace.lock().unwrap().pcm.is_empty());
}

#[test]
fn real_audio_thread_resumes_at_the_selected_sample_after_queued_seek() {
    use std::sync::mpsc::channel;
    for codec in ["ima", "alac"] {
        let target = Duration::from_millis(if codec == "ima" { 8 } else { 20 });
        let skip = if codec == "ima" { 64 } else { 160 };
        let trace = Arc::new(Mutex::new(Trace::default()));
        let seen = trace.clone();
        let (ready_tx, ready_rx) = channel();
        let (release_tx, release_rx) = channel();
        let mut playback = AudioPlayback::start(stream(codec), move || {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Box::new(Device {
                trace: seen,
                at: Duration::ZERO,
            })
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        playback.seek(target);
        playback.play();
        release_tx.send(()).unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(std::time::Instant::now() < until, "audio thread stalled");
            match playback.poll() {
                Some(AudioEvent::Ended(g)) if g == playback.generation() => break,
                Some(AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let t = trace.lock().unwrap();
        assert_eq!(
            t.actions,
            [Action::Flush(target), Action::Resume(target)],
            "{codec}: real worker resumed before navigation"
        );
        assert_eq!(t.pcm, bytes(&format!("{codec}.f32le"))[skip * 4..]);
    }
}
