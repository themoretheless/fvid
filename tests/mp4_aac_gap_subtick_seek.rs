#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-aac-gap-subtick-seek")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn open(n: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap()
}
fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            a.is_finite() && b.is_finite() && (a - b).abs() < 2e-6,
            "{a}!={b}"
        );
    }
}
fn expected() -> Vec<u8> {
    let r = bytes("reference.f32le");
    [r[..48 * 4].to_vec(), vec![0; 48 * 4]].concat()
}
#[test]
fn independent_coded_pcm_gap_silence_and_native_video_controls() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut r = open(name);
        let first = r.track().samples.get(0).unwrap();
        let second = r.track().samples.get(1).unwrap();
        let delta = if r.track().timescale == 50 { 1 } else { 960 };
        assert_eq!(
            (first.pts, first.duration, second.pts),
            (0, delta, 2 * i64::from(delta))
        );
        assert_eq!(r.duration(), Some(Duration::from_millis(2)));
        let pcm: Vec<_> = transitions::collect(&mut r)
            .into_iter()
            .flat_map(|p| p.3)
            .collect();
        compare(&pcm, &expected());
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(name)), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert_eq!(
                v.read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
        }
        assert!(v.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn gap_seek_clock_anchor_matches_the_first_native_silence_sample() {
    let oracle = expected();
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut r = open(name);
        for target in [
            Duration::from_millis(1),
            Duration::from_nanos(1000001),
            Duration::from_nanos(1500001),
            Duration::from_nanos(1999999),
            Duration::from_millis(2),
            Duration::MAX,
        ] {
            let first = (target.as_nanos() * 48000).div_ceil(1_000_000_000).min(96) as usize;
            let at = r.seek_time(target);
            let clock = r.time_of(at);
            let packets = transitions::collect(&mut r);
            let pcm: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
            compare(&pcm, &oracle[first * 4..]);
            let expected = Duration::from_nanos(first as u64 * 1_000_000_000 / 48000);
            if clock != expected {
                failures.push(format!(
                    "{name}:{target:?}:anchor{clock:?},expected{expected:?}"
                ));
            }
            if let Some(p) = packets.first() {
                assert_eq!(p.2, expected);
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

struct FlushCapture {
    inner: transitions::Capture,
    anchors: std::sync::Arc<std::sync::Mutex<Vec<Duration>>>,
}
impl fvid::audio::AudioBackend for FlushCapture {
    fn start(&mut self, s: fvid::audio::AudioSpec) -> Result<(), fvid::audio::AudioError> {
        self.inner.start(s)
    }
    fn push(&mut self, p: fvid::audio::AudioPacket) -> Result<(), fvid::audio::AudioError> {
        self.inner.push(p)
    }
    fn position(&self) -> Duration {
        self.inner.position()
    }
    fn flush(&mut self, at: Duration) -> Result<(), fvid::audio::AudioError> {
        self.anchors.lock().unwrap().push(at);
        self.inner.flush(at)
    }
    fn begin_drain(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.begin_drain()
    }
    fn drain_status(&self) -> Result<bool, fvid::audio::AudioError> {
        self.inner.drain_status()
    }
    fn end_drain(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.end_drain()
    }
    fn pause(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.pause()
    }
    fn resume(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.resume()
    }
    fn stop(&mut self) -> Result<(), fvid::audio::AudioError> {
        self.inner.stop()
    }
    fn reconfigure(
        &mut self,
        s: fvid::audio::AudioSpec,
        r: Option<fvid_media::owned_stereo_router::StereoRouter>,
        at: Duration,
        p: bool,
    ) -> Result<(), fvid::audio::AudioError> {
        self.inner.reconfigure(s, r, at, p)
    }
}
#[test]
fn actual_worker_flushes_at_the_native_silence_sample_or_exact_eof() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    let oracle = expected();
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let anchors = Arc::new(Mutex::new(Vec::new()));
        let shared = trace.clone();
        let times = anchors.clone();
        let mut worker = AudioPlayback::start(Box::new(open(name)), move || {
            Box::new(FlushCapture {
                inner: Capture::new(shared),
                anchors: times,
            })
        });
        worker.play();
        ended(&worker, 0);
        let full: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        compare(&full, &oracle);
        for target in [
            Duration::from_millis(1),
            Duration::from_nanos(1000001),
            Duration::from_nanos(1500001),
            Duration::from_nanos(1999999),
            Duration::from_millis(2),
            Duration::MAX,
        ] {
            anchors.lock().unwrap().clear();
            worker.seek(target);
            ended(&worker, worker.generation());
            let first = (target.as_nanos() * 48000).div_ceil(1_000_000_000).min(96) as usize;
            let at = Duration::from_nanos(first as u64 * 1_000_000_000 / 48000);
            let got = anchors.lock().unwrap().clone();
            if got != vec![at] {
                failures.push(format!("{name}:{target:?}:flush{got:?},expected{at:?}"));
            }
            let guard = trace.lock().unwrap();
            let pcm: Vec<_> = guard
                .packets
                .iter()
                .flat_map(|p| p.3.iter().copied())
                .collect();
            compare(&pcm, &oracle[first * 4..]);
            if let Some(p) = guard.packets.first() {
                assert_eq!(p.2, at);
            } else if worker.position() != Duration::from_millis(2) {
                failures.push(format!(
                    "{name}:{target:?}:EOF position{:?}",
                    worker.position()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
