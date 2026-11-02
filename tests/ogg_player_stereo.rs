#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec, AudioStream};
use rational::{Packet, collect, exact};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ogg-player-stereo")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let mut c =
        serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
            .as_array()
            .unwrap()
            .clone();
    c.sort_by_key(|c| if c["channels"] == 6 { 0 } else { 1 });
    c
}
fn source(c: &serde_json::Value, catalog: bool) -> Box<dyn AudioStream> {
    let path = root().join(c["file"].as_str().unwrap());
    if catalog || c["kind"] == "mixed" {
        Box::new(
            fvid::playback_ogg_catalog::Reader::open(
                &path,
                c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            )
            .unwrap(),
        )
    } else {
        let data = std::fs::read(path).unwrap();
        if c["kind"] == "vorbis" {
            Box::new(
                fvid::playback_ogg_audio::OggAudioReader::open(data.as_slice(), Default::default())
                    .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_ogg_opus::OggOpusAudioReader::open(
                    data.as_slice(),
                    Default::default(),
                )
                .unwrap(),
            )
        }
    }
}
fn roles(c: &serde_json::Value) -> Vec<&str> {
    c["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect()
}
fn weights(c: &serde_json::Value) -> Vec<[f32; 2]> {
    let k = std::f32::consts::FRAC_1_SQRT_2;
    roles(c)
        .iter()
        .map(|r| match *r {
            "FL" => [1., 0.],
            "FR" => [0., 1.],
            "FC" if c["channels"] == 1 => [1., 1.],
            "FC" | "BC" => [k, k],
            "BL" | "SL" => [k, 0.],
            "BR" | "SR" => [0., k],
            "LFE" => [0., 0.],
            _ => [1. / c["channels"].as_u64().unwrap() as f32; 2],
        })
        .collect()
}
fn stereo(full: &[Packet], c: &serde_json::Value) -> Vec<Packet> {
    let w = weights(c);
    full.iter()
        .map(|p| {
            let mut out = Vec::new();
            for f in p.data.chunks_exact(w.len() * 4) {
                let mut sum = [0f32; 2];
                for (x, weights) in f.chunks_exact(4).zip(&w) {
                    let value = f32::from_le_bytes(x.try_into().unwrap());
                    sum[0] += value * weights[0];
                    sum[1] += value * weights[1];
                }
                out.extend(sum.into_iter().flat_map(f32::to_le_bytes));
            }
            Packet {
                rate: p.rate,
                channels: 2,
                num: p.num,
                den: p.den,
                data: out,
            }
        })
        .collect()
}
#[test]
fn native_tones_have_the_declared_speaker_order_before_device_routing() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut direct = source(&c, false);
        let full = collect(&mut *direct);
        let data: Vec<_> = full.iter().flat_map(|p| p.data.clone()).collect();
        let channels = c["channels"].as_u64().unwrap() as usize;
        assert!(!data.is_empty());
        assert!(
            data.chunks_exact(4)
                .all(|b| f32::from_le_bytes(b.try_into().unwrap()).is_finite())
        );
        assert!(
            data.chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-5)
        );
        assert_eq!(direct.channels() as usize, channels);
        if c.get("frequencies").is_some() {
            let freq = c["frequencies"].as_object().unwrap();
            for (i, role) in roles(&c).iter().enumerate() {
                let score = |hz: f64| -> f64 {
                    let (mut sin, mut cos) = (0., 0.);
                    for (frame, s) in data.chunks_exact(channels * 4).enumerate() {
                        let value =
                            f32::from_le_bytes(s[i * 4..i * 4 + 4].try_into().unwrap()) as f64;
                        let a = 2. * std::f64::consts::PI * hz * frame as f64 / 48000.;
                        sin += value * a.sin();
                        cos += value * a.cos();
                    }
                    (sin * sin + cos * cos).sqrt()
                };
                let wanted = score(freq[*role].as_f64().unwrap());
                let wrong = freq
                    .iter()
                    .filter(|(r, _)| r.as_str() != *role)
                    .map(|(_, v)| score(v.as_f64().unwrap()))
                    .fold(0f64, f64::max);
                if !(wanted > wrong * 2. + 1e-6) {
                    failures.push(format!("{name}: channel{i}/{role}: {wanted} vs{wrong}"));
                }
            }
        }
        let mut catalog = source(&c, true);
        assert_eq!(
            catalog.audio_track_index(),
            c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize
        );
        exact(&collect(&mut *catalog), &full, name);
        assert_eq!(catalog.channels() as usize, channels);
        assert_eq!(catalog.duration(), direct.duration());
    }
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        let data: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(data, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(v.read_frame().unwrap().is_none());
    assert!(failures.is_empty(), "tone roles: {failures:?}");
}
#[test]
fn native_and_catalogue_readers_supply_exact_stereo_coefficients() {
    for c in cases() {
        for catalogue in [false, true] {
            if c["kind"] == "mixed" && !catalogue {
                continue;
            }
            let s = source(&c, catalogue);
            let router = s
                .stereo_router()
                .unwrap()
                .expect("Ogg must negotiate source speakers for a stereo device");
            assert_eq!(router.input_channels(), s.channels());
            assert_eq!(
                router.coefficients(),
                weights(&c),
                "{} catalog{catalogue}",
                c["file"]
            );
            let n = s.channels() as usize;
            let basis: Vec<_> = (0..n)
                .flat_map(|active| {
                    (0..n).flat_map(move |i| (if i == active { 1f32 } else { 0. }).to_le_bytes())
                })
                .collect();
            let out = router.route(&basis, n * 8).unwrap();
            assert_eq!(out, weights(&c).into_iter().flatten().collect::<Vec<_>>());
            assert!(router.route(&basis, n * 8 - 1).is_err());
            assert!(router.route(&basis[..basis.len() - 1], n * 8).is_err());
        }
    }
}
struct StereoSink {
    inner: rational::RawCapture,
    router: Option<fvid_media::owned_stereo_router::StereoRouter>,
}
impl AudioBackend for StereoSink {
    fn start(&mut self, s: AudioSpec) -> Result<(), AudioError> {
        if s.channels != 2 {
            return Err(AudioError::InitFailed(format!(
                "stereo-only synthetic sink rejects {} channels",
                s.channels
            )));
        }
        self.inner.start(s)
    }
    fn start_with_router(
        &mut self,
        s: AudioSpec,
        r: Option<fvid_media::owned_stereo_router::StereoRouter>,
    ) -> Result<(), AudioError> {
        if let Some(r) = &r {
            assert_eq!(r.input_channels(), s.channels);
        }
        let channels = if r.is_some() { 2 } else { s.channels };
        self.router = r;
        self.start(AudioSpec { channels, ..s })
    }
    fn push(&mut self, mut p: AudioPacket) -> Result<(), AudioError> {
        if let Some(r) = &self.router {
            p.data = r
                .route(&p.data, 32 << 20)
                .map_err(AudioError::PlaybackError)?
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect();
        }
        self.inner.push(p)
    }
    fn position(&self) -> Duration {
        self.inner.position()
    }
    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.inner.flush(at)
    }
    fn pause(&mut self) -> Result<(), AudioError> {
        self.inner.pause()
    }
    fn resume(&mut self) -> Result<(), AudioError> {
        self.inner.resume()
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        self.inner.stop()
    }
    fn begin_drain(&mut self) -> Result<(), AudioError> {
        self.inner.begin_drain()
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        self.inner.drain_status()
    }
    fn end_drain(&mut self) -> Result<(), AudioError> {
        self.inner.end_drain()
    }
}
#[test]
fn real_ogg_workers_play_exact_stereo_pcm_after_seek_and_at_eof() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    for c in cases() {
        let native = collect(&mut *source(&c, false));
        let last = native.last().unwrap();
        let end = last.point(last.frames());
        let folded = stereo(&native, &c);
        for catalogue in [false, true] {
            if c["kind"] == "mixed" && !catalogue {
                continue;
            }
            for at in [
                Duration::ZERO,
                Duration::from_millis(30),
                Duration::from_millis(150),
                Duration::MAX,
            ] {
                let expected = rational::suffix(&folded, at, end);
                let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
                let captured = raw.clone();
                let trace = Arc::new(Mutex::new(Trace::default()));
                let mut worker = AudioPlayback::start(source(&c, catalogue), move || {
                    Box::new(StereoSink {
                        inner: rational::RawCapture {
                            inner: Capture::new(trace),
                            raw: captured,
                        },
                        router: None,
                    })
                });
                worker.seek(at);
                worker.play();
                ended(&worker, worker.generation());
                let got = raw.lock().unwrap();
                exact(&got.packets, &expected, c["file"].as_str().unwrap());
                assert_eq!(got.anchors, vec![rational::anchor(&expected, end)]);
            }
        }
    }
}
