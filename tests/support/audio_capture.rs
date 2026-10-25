use fvid::codec::ac3_decoder::CompressionMode;
use fvid::{audio::AudioStream, audio_thread::DolbyPlaybackPolicy};
use std::time::Duration;

struct Capture {
    packets: std::sync::Arc<std::sync::Mutex<Vec<(Duration, Vec<u8>)>>>,
    spec: Option<fvid::audio::AudioSpec>,
    at: Duration,
    wanted_mask: Option<u32>,
}
impl fvid::audio::AudioBackend for Capture {
    fn start(&mut self, spec: fvid::audio::AudioSpec) -> Result<(), fvid::audio::AudioError> {
        self.spec = Some(spec);
        Ok(())
    }
    fn start_with_layout(
        &mut self,
        spec: fvid::audio::AudioSpec,
        mask: Option<u32>,
    ) -> Result<(), fvid::audio::AudioError> {
        if let Some(wanted) = self.wanted_mask {
            if mask != Some(wanted) {
                return Err(fvid::audio::AudioError::InitFailed(format!(
                    "speaker mask {mask:?}, expected {wanted:#x}"
                )));
            }
        }
        self.start(spec)
    }
    fn push(&mut self, packet: fvid::audio::AudioPacket) -> Result<(), fvid::audio::AudioError> {
        let spec = self.spec.unwrap();
        self.at = packet.presentation_time()
            + Duration::from_nanos(
                (packet.data.len() / 4 / usize::from(spec.channels)) as u64 * 1_000_000_000
                    / u64::from(spec.sample_rate),
            );
        self.packets
            .lock()
            .unwrap()
            .push((packet.presentation_time(), packet.data));
        Ok(())
    }
    fn position(&self) -> Duration {
        self.at
    }
    fn flush(&mut self, at: Duration) -> Result<(), fvid::audio::AudioError> {
        self.at = at;
        self.packets.lock().unwrap().clear();
        Ok(())
    }
    fn pause(&mut self) -> Result<(), fvid::audio::AudioError> {
        Ok(())
    }
    fn resume(&mut self) -> Result<(), fvid::audio::AudioError> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), fvid::audio::AudioError> {
        Ok(())
    }
}
#[allow(dead_code)]
pub fn worker_check(audio: Box<dyn AudioStream>, expected: &[(Duration, Vec<u8>)]) {
    worker_check_with_seek(audio, expected, None);
}
#[allow(dead_code)]
pub fn worker_check_with_seek(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Option<Duration>,
) {
    worker_check_impl(
        audio,
        expected,
        seek,
        None,
        None,
        CompressionMode::Line.into(),
    );
}
#[allow(dead_code)]
pub fn worker_check_with_seek_expected(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Duration,
    expected_seek: &[(Duration, Vec<u8>)],
) {
    worker_check_impl(
        audio,
        expected,
        Some(seek),
        Some(expected_seek),
        None,
        CompressionMode::Line.into(),
    );
}
#[allow(dead_code)]
pub fn worker_check_with_layout_and_seek_expected(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Duration,
    expected_seek: &[(Duration, Vec<u8>)],
    mask: u32,
) {
    worker_check_impl(
        audio,
        expected,
        Some(seek),
        Some(expected_seek),
        Some(mask),
        CompressionMode::Line.into(),
    );
}
#[allow(dead_code)]
pub fn worker_check_with_compression(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Duration,
    expected_seek: &[(Duration, Vec<u8>)],
    mask: Option<u32>,
    compression: CompressionMode,
) {
    worker_check_impl(
        audio,
        expected,
        Some(seek),
        Some(expected_seek),
        mask,
        compression.into(),
    );
}
#[allow(dead_code)]
pub fn worker_check_with_dolby_policy(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Duration,
    expected_seek: &[(Duration, Vec<u8>)],
    mask: Option<u32>,
    policy: DolbyPlaybackPolicy,
) {
    worker_check_impl(
        audio,
        expected,
        Some(seek),
        Some(expected_seek),
        mask,
        policy,
    );
}
fn worker_check_impl(
    audio: Box<dyn AudioStream>,
    expected: &[(Duration, Vec<u8>)],
    seek: Option<Duration>,
    expected_seek: Option<&[(Duration, Vec<u8>)]>,
    wanted_mask: Option<u32>,
    policy: DolbyPlaybackPolicy,
) {
    let spec = (audio.sample_rate(), if (policy.eac3_stereo_output && matches!(audio.codec(), "A_EAC3" | "ec-3")) || (policy.ac3_stereo_output && matches!(audio.codec(), "A_AC3" | "ac-3" | "A_AC3/BSID9" | "A_AC3/BSID10")) {2}else{audio.channels()});
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = captured.clone();
    let mut playback =
        fvid::audio_thread::AudioPlayback::start_with_dolby_policy(audio, policy, move || {
            Box::new(Capture {
                packets: sink,
                spec: None,
                at: Duration::ZERO,
                wanted_mask,
            })
        });
    for generation in 0..if seek.is_some() { 3 } else { 2 } {
        if generation == 1 {
            playback.rewind();
        }
        if generation == 2 {
            playback.seek(seek.unwrap());
        }
        playback.play();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "audio worker did not reach EOF"
            );
            match playback.poll() {
                Some(fvid::audio_thread::AudioEvent::Ended(g)) if g == generation => break,
                Some(fvid::audio_thread::AudioEvent::Error(e)) => panic!("{e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let wanted: Vec<_> = if generation == 2 && expected_seek.is_some() {
            expected_seek.unwrap().to_vec()
        } else {
            expected
                .iter()
                .filter(|(at, _)| generation != 2 || *at >= seek.unwrap())
                .cloned()
                .collect()
        };
        {
            let actual = captured.lock().unwrap();
            assert_eq!(
                actual.len(),
                wanted.len(),
                "worker generation{generation}: packet count"
            );
            for (index, (actual, wanted)) in actual.iter().zip(&wanted).enumerate() {
                assert_eq!(
                    actual.0, wanted.0,
                    "worker generation{generation} packet{index}: PTS"
                );
                assert!(
                    actual.1 == wanted.1,
                    "worker generation{generation} packet{index}: PCM differs (actual{} expected{} bytes)",
                    actual.1.len(),
                    wanted.1.len()
                );
            }
        }
        let end = wanted
            .last()
            .map(|(at, data)| {
                *at + Duration::from_nanos(
                    (data.len() / 4 / usize::from(spec.1)) as u64 * 1_000_000_000
                        / u64::from(spec.0),
                )
            })
            .unwrap_or(if generation == 2 {
                seek.unwrap()
            } else {
                Duration::ZERO
            });
        assert_eq!(
            playback.position(),
            end,
            "worker generation{generation}: audible end position"
        );
    }
}
#[allow(dead_code)]
pub fn worker_refusal(audio: Box<dyn AudioStream>, needle: &str) {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = captured.clone();
    let playback = fvid::audio_thread::AudioPlayback::start(audio, move || {
        Box::new(Capture {
            packets: sink,
            spec: None,
            at: Duration::ZERO,
            wanted_mask: None,
        })
    });
    playback.play();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "audio worker did not refuse invalid padding"
        );
        match playback.poll() {
            Some(fvid::audio_thread::AudioEvent::Error(error)) => {
                assert!(error.contains(needle), "{error}");
                assert!(captured.lock().unwrap().is_empty());
                return;
            }
            Some(fvid::audio_thread::AudioEvent::Ended(_)) => {
                panic!("invalid audio accepted to EOF")
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
