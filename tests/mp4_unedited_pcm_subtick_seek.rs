#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-unedited-pcm-subtick-seek")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
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
fn time(q: &serde_json::Value) -> Duration {
    let n: u128 = q["target_ns"].as_str().unwrap().parse().unwrap();
    Duration::new((n / 1_000_000_000) as u64, (n % 1_000_000_000) as u32)
}
fn check(got: &transitions::Frames, q: &serde_json::Value) {
    let parts = q["parts"].as_array().unwrap();
    let mut group = 0;
    let mut consumed = 0;
    for p in got {
        assert!(group < parts.len(), "unexpected PCM");
        let e = &parts[group];
        let rate = e["rate"].as_u64().unwrap();
        let ch = e["channels"].as_u64().unwrap();
        assert_eq!((p.0 as u64, p.1 as u64), (rate, ch));
        let start = e["first"].as_u64().unwrap() + consumed;
        let count = p.3.len() as u64 / (ch * 4);
        assert_eq!(p.3.len() as u64, count * ch * 4);
        assert!(start + count <= e["last"].as_u64().unwrap());
        assert_eq!(
            p.2,
            Duration::from_nanos(e["start_ns"].as_u64().unwrap() + start * 1_000_000_000 / rate)
        );
        let r = bytes(&format!(
            "reference-{}.f32le",
            e["control"].as_u64().unwrap()
        ));
        let expected = &r[(start * ch * 4) as usize..((start + count) * ch * 4) as usize];
        for (a, b) in p.3.chunks_exact(4).zip(expected.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(a.is_finite() && b.is_finite() && (a - b).abs() < 2e-6);
        }
        consumed += count;
        if e["first"].as_u64().unwrap() + consumed == e["last"].as_u64().unwrap() {
            group += 1;
            consumed = 0;
        }
    }
    assert_eq!(group, parts.len(), "missing PCM");
}
#[test]
fn independent_lc_main_pcm_and_native_video_controls() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = open(n);
        assert_eq!(u64::from(r.track().timescale), c["scale"].as_u64().unwrap());
        assert_eq!(
            r.duration(),
            Some(Duration::from_nanos(c["duration_ns"].as_u64().unwrap()))
        );
        check(&transitions::collect(&mut r), &c["queries"][0]);
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for _ in 0..c["video_frames"].as_u64().unwrap() {
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
fn unedited_pcm_duration_seek_preserves_exact_native_floor() {
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = open(n);
        for q in c["queries"].as_array().unwrap() {
            let t = time(q);
            let at = r.seek_time(t);
            let got = transitions::collect(&mut r);
            let actual: usize = got.iter().map(|p| p.3.len()).sum();
            let expected: usize = q["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    ((p["last"].as_u64().unwrap() - p["first"].as_u64().unwrap())
                        * p["channels"].as_u64().unwrap()
                        * 4) as usize
                })
                .sum();
            if actual != expected {
                failures.push(format!("{n}:{t:?}:{actual}bytes expected{expected}"));
                continue;
            }
            check(&got, q);
            let expected = Duration::from_nanos(q["anchor_ns"].as_u64().unwrap());
            if r.time_of(at) != expected {
                failures.push(format!(
                    "{n}:{t:?}:clock{:?},expected{expected:?}",
                    r.time_of(at)
                ));
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

fn expected_bytes(q: &serde_json::Value) -> usize {
    q["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            ((p["last"].as_u64().unwrap() - p["first"].as_u64().unwrap())
                * p["channels"].as_u64().unwrap()
                * 4) as usize
        })
        .sum()
}
#[test]
fn actual_worker_unedited_pcm_flushes_at_the_native_sample() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    let mut failures = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let anchors = Arc::new(Mutex::new(Vec::new()));
        let times = anchors.clone();
        let mut w = AudioPlayback::start(Box::new(open(n)), move || {
            Box::new(FlushCapture {
                inner: Capture::new(shared),
                anchors: times,
            })
        });
        w.play();
        ended(&w, 0);
        check(&trace.lock().unwrap().packets, &c["queries"][0]);
        for q in c["queries"].as_array().unwrap() {
            anchors.lock().unwrap().clear();
            w.seek(time(q));
            ended(&w, w.generation());
            let expected = Duration::from_nanos(q["anchor_ns"].as_u64().unwrap());
            let got = anchors.lock().unwrap().clone();
            if got != vec![expected] {
                failures.push(format!(
                    "{n}:{:?}:flush{got:?},expected{expected:?}",
                    time(q)
                ));
            }
            let guard = trace.lock().unwrap();
            let actual: usize = guard.packets.iter().map(|p| p.3.len()).sum();
            let wanted = expected_bytes(q);
            if actual != wanted {
                failures.push(format!("{n}:{:?}:{actual}bytes expected{wanted}", time(q)));
                continue;
            }
            check(&guard.packets, q);
            if q["parts"].as_array().unwrap().is_empty() && w.position() != expected {
                failures.push(format!("{n}:EOF{:?} expected{expected:?}", w.position()));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn raw_tick_silence_navigation_retains_its_declared_grid_contract() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-quantized-audio-clock/ps-edits-900-floor.mp4");
    let mut r = Mp4AudioReader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(r.track().timescale, 900);
    let rate = u64::from(r.sample_rate());
    let at = r.seek_to(1);
    let raw = Duration::from_nanos(1_000_000_000 / 900);
    assert_eq!(r.time_of(at), raw);
    let expected = Duration::from_nanos(rate.div_ceil(900) * 1_000_000_000 / rate);
    assert!(expected > raw);
    let first = transitions::collect(&mut r);
    assert_eq!(first[0].2, expected);
    r.rewind();
    let at = r.seek_to(1);
    assert_eq!(r.time_of(at), raw);
    let second = transitions::collect(&mut r);
    transitions::compare(&first, &second);
}

#[test]
fn raw_tick_reseek_discards_the_precise_seek_floor_and_anchor() {
    for c in cases() {
        let n=c["file"].as_str().unwrap();let mut r=open(n);
        r.seek_time(Duration::from_nanos(125001));let _=transitions::collect(&mut r);
        let raw=r.seek_to(0);assert_eq!(raw,0,"{n}:raw origin");
        assert_eq!(r.time_of(raw),Duration::ZERO,"{n}:stale anchor");
        check(&transitions::collect(&mut r),&c["queries"][0]);
    }
}
