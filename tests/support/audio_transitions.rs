use fvid::{
    audio::{AudioBackend, AudioError, AudioSpec, AudioStep, AudioStream},
    audio_thread::{AudioEvent, AudioPlayback},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
pub type Frames = Vec<(u32, u16, Duration, Vec<u8>)>;
pub fn compare(actual: &Frames, expected: &Frames) {
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
pub fn collect(stream: &mut dyn AudioStream) -> Frames {
    let decoder = stream.make_decoder().unwrap();
    collect_with_decoder(stream, decoder)
}
pub fn collect_with_decoder(
    stream: &mut dyn AudioStream,
    mut decoder: Box<dyn fvid::audio::AudioDecode>,
) -> Frames {
    let mut out = Vec::new();
    fn present(stream: &dyn AudioStream, mut frame: fvid::audio::DecodedAudio, out: &mut Frames) {
        if let Some(frames) = stream
            .decoded_packet_sample_limit(frame.source_pts, frame.source_duration)
            .unwrap()
        {
            let bytes = frames
                .checked_mul(usize::from(stream.channels()) * 4)
                .unwrap();
            assert!(
                bytes <= frame.packet.data.len(),
                "decoded window exceeds PCM: stamp {} duration {} rate {} ch {} wanted {} actual {}",
                frame.source_pts,
                frame.source_duration,
                stream.sample_rate(),
                stream.channels(),
                bytes,
                frame.packet.data.len()
            );
            frame.packet.data.truncate(bytes);
        }
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
                    .decode_packet(&p.data, stream.decoder_stamp(&p), p.duration as u64)
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
            None => {
                if !stream.drain_decoded_at_eof() {
                    decoder.reset();
                    break;
                }
                if let Some(frame) = decoder.finish_packet().unwrap() {
                    present(stream, frame, &mut out);
                    continue;
                }
                break;
            }
        }
    }
    stream.validate_eof().unwrap();
    out
}
#[derive(Default)]
pub struct Trace {
    pub packets: Frames,
    pub configs: Vec<AudioSpec>,
    pub intermediate_drains: usize,
    pub begins: usize,
    pub flushes: usize,
}
pub struct Capture {
    pub trace: Arc<Mutex<Trace>>,
    pub spec: Option<AudioSpec>,
    pub at: Duration,
    pub remaining: AtomicUsize,
    pub draining: bool,
    pub fail_next: Option<Arc<AtomicBool>>,
    pub gate: Option<Arc<AtomicBool>>,
    pub paused: Option<Arc<AtomicBool>>,
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
pub fn ended(audio: &AudioPlayback, generation: u64) {
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

impl Capture {
    pub fn new(trace: Arc<Mutex<Trace>>) -> Self {
        Self {
            trace,
            spec: None,
            at: Duration::ZERO,
            remaining: AtomicUsize::new(0),
            draining: false,
            fail_next: None,
            gate: None,
            paused: None,
        }
    }
}
