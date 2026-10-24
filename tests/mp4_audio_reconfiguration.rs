#![cfg(feature = "player")]
use fvid::{
    audio::{AudioBackend, AudioError, AudioSpec, AudioStep, AudioStream},
    audio_thread::{AudioEvent, AudioPlayback},
    playback_mp4_audio::Mp4AudioReader,
};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-audio-reconfiguration")
            .join(name),
    )
    .unwrap()
}
type Frames = Vec<(u32, u16, Duration, Vec<u8>)>;
fn wanted() -> Frames {
    let doc: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    doc["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rate"].as_u64().unwrap() as u32,
                f["channels"].as_u64().unwrap() as u16,
                Duration::from_micros(f["pts"].as_u64().unwrap()),
                bytes(f["oracle"].as_str().unwrap()),
            )
        })
        .collect()
}
fn compare(actual: &Frames, expected: &Frames) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!((a.0, a.1, a.2), (b.0, b.1, b.2), "packet{i}");
        assert!(
            a.3 == b.3,
            "packet{i}: exact PCM mismatch {} vs {} bytes",
            a.3.len(),
            b.3.len()
        );
    }
}
fn collect(stream: &mut dyn AudioStream) -> Frames {
    let decoder = stream.make_decoder().unwrap();
    collect_with_decoder(stream, decoder)
}
fn collect_with_decoder(
    stream: &mut dyn AudioStream,
    mut decoder: Box<dyn fvid::audio::AudioDecode>,
) -> Frames {
    let mut out = Vec::new();
    fn present(stream: &dyn AudioStream, frame: fvid::audio::DecodedAudio, out: &mut Frames) {
        if let Some(packet) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            out.push((
                stream.sample_rate(),
                stream.channels(),
                packet.presentation_time(),
                packet.data,
            ));
        }
    }
    loop {
        match stream.next_step().unwrap() {
            Some(AudioStep::Encoded(p)) => {
                if let Some(frame) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    present(stream, frame, &mut out);
                }
            }
            Some(AudioStep::ReconfigureDecoder) => {
                while let Some(frame) = decoder.finish_packet().unwrap() {
                    present(stream, frame, &mut out);
                }
                stream.advance_configuration().unwrap();
                decoder = stream.make_decoder().unwrap();
            }
            Some(AudioStep::ResetDecoder) => stream.reset_decoder(&mut decoder).unwrap(),
            Some(AudioStep::Pcm(p)) => out.push((
                stream.sample_rate(),
                stream.channels(),
                p.presentation_time(),
                p.data,
            )),
            None => break,
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        present(stream, frame, &mut out);
    }
    stream.validate_eof().unwrap();
    out
}
#[test]
fn pcm_coding_rate_channels_and_precision_follow_actual_packet_descriptions() {
    for name in [
        "pcm.mp4",
        "pcm-fragmented.mp4",
        "pcm-start-third.mp4",
        "pcm-start-third-fragmented.mp4",
    ] {
        let mut stream =
            Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        compare(&collect(&mut stream), &wanted());
        stream.rewind();
        compare(&collect(&mut stream), &wanted());
        let target = stream.seek_to(50000);
        assert_eq!(target, 50000);
        let mut decoder = stream.make_decoder().unwrap();
        stream.reset_decoder(&mut decoder).unwrap();
        let mut expected = wanted();
        expected.drain(..2);
        expected[0].2 = Duration::from_millis(50);
        expected[0].3 = expected[0].3[441 * 8..].to_vec();
        compare(&collect(&mut stream), &expected);
    }
}
#[derive(Default)]
struct Trace {
    packets: Frames,
    configs: Vec<AudioSpec>,
    intermediate_drains: usize,
    begins: usize,
    flushes: usize,
}
struct Capture {
    trace: Arc<Mutex<Trace>>,
    spec: Option<AudioSpec>,
    at: Duration,
    remaining: AtomicUsize,
    draining: bool,
    fail_next: Option<Arc<AtomicBool>>,
    gate: Option<Arc<AtomicBool>>,
    paused: Option<Arc<AtomicBool>>,
}
impl AudioBackend for Capture {
    fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError> {
        self.spec = Some(spec);
        self.trace.lock().unwrap().configs.push(spec);
        Ok(())
    }
    fn push(&mut self, p: fvid::audio::AudioPacket) -> Result<(), AudioError> {
        assert!(!self.draining, "new PCM arrived before drain completion");
        let s = self.spec.unwrap();
        let pts = p.presentation_time();
        self.at = pts
            + Duration::from_nanos(
                (p.data.len() / 4 / usize::from(s.channels)) as u64 * 1_000_000_000
                    / u64::from(s.sample_rate),
            );
        self.trace
            .lock()
            .unwrap()
            .packets
            .push((s.sample_rate, s.channels, pts, p.data));
        Ok(())
    }
    fn begin_drain(&mut self) -> Result<(), AudioError> {
        self.trace.lock().unwrap().begins += 1;
        self.draining = true;
        self.remaining.store(2, Ordering::SeqCst);
        Ok(())
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        if self
            .gate
            .as_ref()
            .is_some_and(|g| !g.load(Ordering::SeqCst))
        {
            return Ok(false);
        }
        Ok(self
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            })
            .unwrap()
            == 0)
    }
    fn end_drain(&mut self) -> Result<(), AudioError> {
        assert_eq!(self.remaining.load(Ordering::SeqCst), 0);
        self.draining = false;
        self.trace.lock().unwrap().intermediate_drains += 1;
        Ok(())
    }
    fn reconfigure(
        &mut self,
        spec: AudioSpec,
        _: Option<fvid_media::owned_stereo_router::StereoRouter>,
        at: Duration,
        _: bool,
    ) -> Result<(), AudioError> {
        if self
            .fail_next
            .as_ref()
            .is_some_and(|f| f.swap(false, Ordering::SeqCst))
        {
            return Err(AudioError::PlaybackError(
                "injected output setup failure".into(),
            ));
        }
        if self.draining {
            assert_eq!(
                self.remaining.load(Ordering::SeqCst),
                0,
                "output replaced before old samples drained"
            );
        }
        self.at = at;
        self.spec = Some(spec);
        self.trace.lock().unwrap().configs.push(spec);
        Ok(())
    }
    fn position(&self) -> Duration {
        self.at
    }
    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.at = at;
        self.draining = false;
        self.remaining.store(0, Ordering::SeqCst);
        let mut trace = self.trace.lock().unwrap();
        trace.packets.clear();
        trace.flushes += 1;
        Ok(())
    }
    fn pause(&mut self) -> Result<(), AudioError> {
        if let Some(flag) = &self.paused {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn resume(&mut self) -> Result<(), AudioError> {
        if let Some(flag) = &self.paused {
            flag.store(false, Ordering::SeqCst);
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
}
fn ended(audio: &AudioPlayback, generation: u64) {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < until, "worker timeout");
        while let Some(event) = audio.poll() {
            match event {
                AudioEvent::Error(e) => panic!("{e}"),
                AudioEvent::Ended(g) if g == generation => return,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn real_worker_drains_old_output_before_changing_geometry_and_keeps_navigation() {
    for name in [
        "pcm.mp4",
        "pcm-fragmented.mp4",
        "pcm-start-third.mp4",
        "pcm-start-third-fragmented.mp4",
    ] {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let stream = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(stream), move || {
            Box::new(Capture {
                trace: shared,
                spec: None,
                at: Duration::ZERO,
                remaining: AtomicUsize::new(0),
                draining: false,
                fail_next: None,
                gate: None,
                paused: None,
            })
        });
        audio.play();
        ended(&audio, 0);
        compare(&trace.lock().unwrap().packets, &wanted());
        audio.rewind();
        ended(&audio, audio.generation());
        compare(&trace.lock().unwrap().packets, &wanted());
        audio.seek(Duration::from_millis(50));
        ended(&audio, audio.generation());
        let mut expected = wanted();
        expected.drain(..2);
        expected[0].2 = Duration::from_millis(50);
        expected[0].3 = expected[0].3[441 * 8..].to_vec();
        compare(&trace.lock().unwrap().packets, &expected);
        assert!(trace.lock().unwrap().intermediate_drains >= 10);
    }
}
#[test]
fn variable_rate_edit_scheduler_has_an_explicit_refusal_witness() {
    let error = Mp4AudioReader::open(Cursor::new(bytes("pcm-edits.mp4")), Default::default())
        .err()
        .expect("variable-rate edits remain open");
    assert!(
        error.to_string().contains("variable-rate edit scheduler"),
        "{error}"
    );
}
#[test]
#[ignore = "Variable-rate audio edit scheduling is not implemented; refusal does not prove playback acceptance"]
fn changing_descriptions_with_leading_silence_play_the_intended_timeline() {
    let mut stream =
        Mp4AudioReader::open(Cursor::new(bytes("pcm-edits.mp4")), Default::default()).unwrap();
    let mut expected = vec![(48000, 2, Duration::ZERO, vec![0; 960 * 8])];
    expected.extend(
        wanted()
            .into_iter()
            .map(|(r, c, t, b)| (r, c, t + Duration::from_millis(20), b)),
    );
    compare(&collect(&mut stream), &expected);
}

#[test]
fn eac3_delayed_tail_keeps_its_old_geometry_before_output_reconfiguration() {
    let mut expected = Vec::new();
    for (identity, start) in [(1, 0), (2, 96000), (1, 240000)] {
        let mut control = Mp4AudioReader::open(
            Cursor::new(bytes(&format!("eac3-control-{identity}.mp4"))),
            Default::default(),
        )
        .unwrap();
        let frames = collect(&mut control);
        let channels = if identity == 1 { 2 } else { 1 };
        assert_eq!(
            frames
                .iter()
                .map(|f| f.3.len() / 4 / channels)
                .sum::<usize>(),
            3 * 1536,
            "control must include its delayed final packet"
        );
        assert!(
            frames
                .iter()
                .flat_map(|f| f.3.chunks_exact(4))
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()) != 0.)
        );
        expected.extend(
            frames
                .into_iter()
                .map(|(r, c, t, b)| (r, c, t + Duration::from_micros(start), b)),
        );
    }
    let mut stream =
        Mp4AudioReader::open(Cursor::new(bytes("eac3.mp4")), Default::default()).unwrap();
    compare(&collect(&mut stream), &expected);
    let trace = Arc::new(Mutex::new(Trace::default()));
    let shared = trace.clone();
    let stream = Mp4AudioReader::open(Cursor::new(bytes("eac3.mp4")), Default::default()).unwrap();
    let mut audio = AudioPlayback::start(Box::new(stream), move || {
        Box::new(Capture {
            trace: shared,
            spec: None,
            at: Duration::ZERO,
            remaining: AtomicUsize::new(0),
            draining: false,
            fail_next: None,
            gate: None,
            paused: None,
        })
    });
    audio.play();
    ended(&audio, 0);
    compare(&trace.lock().unwrap().packets, &expected);
    audio.rewind();
    ended(&audio, audio.generation());
    compare(&trace.lock().unwrap().packets, &expected);
    audio.seek(Duration::from_millis(120));
    ended(&audio, audio.generation());
    let mut suffix = expected.clone();
    suffix.drain(..3);
    suffix[0].2 = Duration::from_millis(120);
    suffix[0].3 = suffix[0].3[768 * 4..].to_vec();
    compare(&trace.lock().unwrap().packets, &suffix);
}

fn controlled(
    fail: Option<Arc<AtomicBool>>,
    gate: Option<Arc<AtomicBool>>,
    paused: Option<Arc<AtomicBool>>,
) -> (AudioPlayback, Arc<Mutex<Trace>>) {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let shared = trace.clone();
    let stream = Mp4AudioReader::open(Cursor::new(bytes("pcm.mp4")), Default::default()).unwrap();
    let audio = AudioPlayback::start(Box::new(stream), move || {
        Box::new(Capture {
            trace: shared,
            spec: None,
            at: Duration::ZERO,
            remaining: AtomicUsize::new(0),
            draining: false,
            fail_next: fail,
            gate,
            paused,
        })
    });
    (audio, trace)
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < until, "control acknowledgement timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn failed_output_reconfiguration_latches_error_and_rewind_rebuilds_the_device() {
    let fail = Arc::new(AtomicBool::new(true));
    let (mut audio, trace) = controlled(Some(fail), None, None);
    audio.play();
    let until = Instant::now() + Duration::from_secs(3);
    'failure: loop {
        assert!(
            Instant::now() < until,
            "injected configuration failure not observed"
        );
        while let Some(event) = audio.poll() {
            match event {
                AudioEvent::Error(e) => {
                    assert!(e.contains("injected output setup failure"), "{e}");
                    break 'failure;
                }
                AudioEvent::Ended(_) => panic!("intermediate configuration boundary emitted EOF"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    audio.rewind();
    audio.play();
    ended(&audio, audio.generation());
    compare(&trace.lock().unwrap().packets, &wanted());
}
#[test]
fn paused_seek_cancels_a_pending_output_drain_without_decoding_stale_pcm() {
    let gate = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let (mut audio, trace) = controlled(None, Some(gate.clone()), Some(paused.clone()));
    audio.play();
    wait_until(|| trace.lock().unwrap().begins > 0);
    audio.pause();
    wait_until(|| paused.load(Ordering::SeqCst));
    audio.seek(Duration::from_millis(50));
    wait_until(|| {
        let t = trace.lock().unwrap();
        t.flushes > 0 && t.configs.last().is_some_and(|s| s.sample_rate == 44100)
    });
    assert!(
        trace.lock().unwrap().packets.is_empty(),
        "paused navigation must not enqueue new PCM"
    );
    gate.store(true, Ordering::SeqCst);
    audio.play();
    ended(&audio, audio.generation());
    let mut expected = wanted();
    expected.drain(..2);
    expected[0].2 = Duration::from_millis(50);
    expected[0].3 = expected[0].3[441 * 8..].to_vec();
    compare(&trace.lock().unwrap().packets, &expected);
}
#[test]
fn stop_interrupts_output_drain_that_never_completes() {
    let (audio, trace) = controlled(None, Some(Arc::new(AtomicBool::new(false))), None);
    audio.play();
    wait_until(|| trace.lock().unwrap().begins > 0);
    let (sent, received) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        drop(audio);
        sent.send(()).unwrap();
    });
    received
        .recv_timeout(Duration::from_secs(3))
        .expect("stop must interrupt pending drain");
    join.join().unwrap();
}

#[test]
fn eac3_session_rf_and_dialogue_choices_survive_natural_decoder_recreation() {
    use fvid::audio_thread::DolbyPlaybackPolicy;
    use fvid::codec::{ac3_decoder::CompressionMode, eac3_decoder::Eac3Decoder};
    fn reference(identity: usize, mode: CompressionMode, normalize: bool) -> Frames {
        let mut control = Mp4AudioReader::open(
            Cursor::new(bytes(&format!("eac3-control-{identity}.mp4"))),
            Default::default(),
        )
        .unwrap();
        let decoder = Eac3Decoder::for_container_with_compression(
            control.extra_data(),
            control.sample_rate(),
            control.channels(),
            mode,
        )
        .unwrap()
        .with_dialogue_normalization(normalize);
        collect_with_decoder(&mut control, Box::new(decoder))
    }
    let rf = reference(1, CompressionMode::Rf, true);
    assert!(
        rf != reference(1, CompressionMode::Line, true),
        "fixture must distinguish RF and line compression"
    );
    assert!(
        rf != reference(1, CompressionMode::Rf, false),
        "fixture must distinguish dialogue normalization"
    );
    let mut expected = Vec::new();
    for (identity, start) in [(1, 0), (2, 96000), (1, 240000)] {
        expected.extend(
            reference(identity, CompressionMode::Rf, true)
                .into_iter()
                .map(|(r, c, t, b)| (r, c, t + Duration::from_micros(start), b)),
        );
    }
    let trace = Arc::new(Mutex::new(Trace::default()));
    let shared = trace.clone();
    let stream = Mp4AudioReader::open(Cursor::new(bytes("eac3.mp4")), Default::default()).unwrap();
    let mut audio = AudioPlayback::start_with_dolby_policy(
        Box::new(stream),
        DolbyPlaybackPolicy {
            compression: CompressionMode::Rf,
            normalize_dialogue: true,
        },
        move || {
            Box::new(Capture {
                trace: shared,
                spec: None,
                at: Duration::ZERO,
                remaining: AtomicUsize::new(0),
                draining: false,
                fail_next: None,
                gate: None,
                paused: None,
            })
        },
    );
    audio.play();
    ended(&audio, 0);
    compare(&trace.lock().unwrap().packets, &expected);
    audio.rewind();
    ended(&audio, audio.generation());
    compare(&trace.lock().unwrap().packets, &expected);
    audio.seek(Duration::from_millis(120));
    ended(&audio, audio.generation());
    let mut suffix = expected.clone();
    suffix.drain(..3);
    suffix[0].2 = Duration::from_millis(120);
    suffix[0].3 = suffix[0].3[768 * 4..].to_vec();
    compare(&trace.lock().unwrap().packets, &suffix);
}

#[test]
fn owned_video_companions_play_with_exact_planes_and_clocks() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for (name, count) in [
        ("pcm.mp4", 6),
        ("pcm-fragmented.mp4", 6),
        ("pcm-start-third.mp4", 6),
        ("pcm-start-third-fragmented.mp4", 6),
        ("pcm-edits.mp4", 6),
        ("eac3.mp4", 17),
        ("eac3-control-1.mp4", 17),
        ("eac3-control-2.mp4", 17),
    ] {
        let mut video = NativeReader::software(Cursor::new(bytes(name)), 32 << 20).unwrap();
        for i in 0..count {
            match video.read_frame_raw().unwrap().unwrap() {
                RawFrame::Planar(p) => {
                    assert_eq!((p.depth, p.frame.width, p.frame.height), (8, 16, 16));
                    assert!(p.frame.data == vec![128; 384], "{name}/{i}");
                }
                _ => panic!("owned VP9 must retain packed planes"),
            }
            assert_eq!(video.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
}
